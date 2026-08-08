//! Host proof that the studio's programmer can program this exact device.
//!
//! The transport here is the byte stream, not a shortcut: the host programmer
//! library encodes real frames, the K1's own configuration service consumes
//! them, and the result is decoded with the same code the image runs. What the
//! editor and the CLI do to a radio is therefore covered without hardware.

use radio_channel_plan::{
    BankFlags, BankMask, BankName, ChannelBank, ChannelDefinition, ChannelFlags, ChannelName,
    ChannelRecord,
};
use radio_device::DeviceService;
use radio_domain::{
    Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel, RadioConfig,
    SquelchLevel, Tone, TxClass,
};
use radio_firmware_k1::configuration::{
    device_service, Programmed, MAX_CHANNELS, MAX_OBJECTS, RETAINED_IMAGE_BYTES,
};
/// The region the K1 image claims in external memory for its configuration.
const CONFIGURATION_BYTES: u32 = 4_096;
#[allow(unused_imports)]
use radio_firmware_k1::configuration::kind_limits as _kind_limits;
use radio_programmer::{Programmer, ProgrammerError, ProtocolTransport, RadioProject};
use radio_protocol::{Command, DeviceErrorCode, MAX_ENCODED_FRAME};

/// The device service behind an in-process byte stream.
struct DeviceTransport {
    service: DeviceService<MAX_OBJECTS>,
    queue: std::collections::VecDeque<u8>,
}

impl DeviceTransport {
    fn new() -> Self {
        Self {
            service: device_service(CONFIGURATION_BYTES),
            queue: std::collections::VecDeque::new(),
        }
    }
}

impl ProtocolTransport for DeviceTransport {
    type Error = core::convert::Infallible;

    fn send(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        let mut response = [0_u8; MAX_ENCODED_FRAME];
        for byte in frame {
            if let Some(length) = self.service.push(*byte, &mut response, &mut |_| {}) {
                self.queue.extend(&response[..length]);
            }
        }
        Ok(())
    }

    fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error> {
        let count = buffer.len().min(self.queue.len());
        for slot in &mut buffer[..count] {
            *slot = self.queue.pop_front().unwrap_or(0);
        }
        Ok(count)
    }
}

fn channel(id: u16, hz: u32, name: &str, bank: Option<u16>) -> ChannelRecord {
    let receive = Frequency::from_hz(hz).expect("frequency");
    let banks = match bank {
        Some(bank) => BankMask::default()
            .with(BankId::new(bank), true)
            .expect("bank mask"),
        None => BankMask::default(),
    };
    ChannelRecord::new(ChannelDefinition {
        id: ChannelId::new(id),
        name: ChannelName::new(name).expect("name"),
        receive,
        transmit: receive,
        rx_tone: Tone::None,
        tx_tone: Tone::None,
        modulation: Modulation::Fm,
        bandwidth: Bandwidth::Narrow,
        power: PowerLevel::Low,
        step: FrequencyStep::from_hz(12_500).expect("step"),
        squelch: SquelchLevel::new(3).expect("squelch"),
        flags: ChannelFlags::default(),
        banks,
        tx_class: TxClass::Never,
    })
    .expect("channel")
}

fn project(channels: u16) -> RadioProject {
    let mut project = RadioProject::new();
    for index in 0..channels {
        project.add_channel(channel(
            index + 1,
            145_000_000 + u32::from(index) * 25_000,
            "CH",
            Some(index % 2),
        ));
    }
    project.add_bank(
        ChannelBank::new(
            BankId::new(0),
            BankName::new("VHF EVEN").expect("name"),
            BankFlags::default(),
        )
        .expect("bank"),
    );
    project.add_bank(
        ChannelBank::new(
            BankId::new(1),
            BankName::new("VHF ODD").expect("name"),
            BankFlags::default(),
        )
        .expect("bank"),
    );
    project.set_config(RadioConfig {
        backlight_seconds: 30,
        ..RadioConfig::conservative()
    });
    project
}

#[test]
fn the_host_programmer_writes_reads_back_and_activates_a_full_configuration() {
    let mut programmer = Programmer::connect(DeviceTransport::new()).expect("connect");
    let capabilities = programmer.capabilities();
    assert_eq!(
        capabilities.max_objects,
        u16::try_from(MAX_OBJECTS).unwrap()
    );

    let project = project(u16::try_from(MAX_CHANNELS).unwrap());
    let compiled = programmer
        .compiler()
        .compile(&project)
        .expect("compile against negotiated capabilities");
    let receipt = programmer
        .write_configuration_verified(&compiled)
        .expect("transactional write with read-back");
    assert_eq!(receipt.generation, 1);

    // The image decodes the active snapshot with the same code the radio runs.
    let activated = Programmed::from_objects(programmer.transport().service.active_objects())
        .expect("programmed configuration");
    assert_eq!(
        activated.channel_count(),
        u16::try_from(MAX_CHANNELS).unwrap()
    );
    assert_eq!(activated.config().backlight_seconds, 30);
    assert_eq!(
        activated
            .bank(BankId::new(1))
            .expect("named bank")
            .name()
            .as_str(),
        "VHF ODD"
    );
    let (banks, count) = activated.populated_banks();
    assert_eq!(count, 2);
    assert_eq!(banks[0], Some(BankId::new(0)));

    // Every programmed channel keeps the class the host wrote, and this image
    // has no transmit path regardless.
    for index in 0..activated.channel_count() {
        use radio_channel_control::ChannelSource;
        let channel = activated.memory().get(index).expect("channel");
        assert_eq!(channel.tx_class(), TxClass::Never);
    }
}

#[test]
fn a_written_configuration_survives_the_retained_image_round_trip() {
    let mut programmer = Programmer::connect(DeviceTransport::new()).expect("connect");
    let compiled = programmer
        .compiler()
        .compile(&project(u16::try_from(MAX_CHANNELS).unwrap()))
        .expect("compile");
    programmer
        .write_configuration_verified(&compiled)
        .expect("write");

    let mut image = [0_u8; RETAINED_IMAGE_BYTES];
    let length = programmer
        .transport()
        .service
        .encode_active_image(&mut image)
        .expect("retained image fits the reserved region");
    assert!(length <= RETAINED_IMAGE_BYTES);

    // A restart restores from those exact bytes and reaches the same state.
    let mut restarted = device_service(CONFIGURATION_BYTES);
    assert_eq!(restarted.load_image(&image[..length]), Ok(1));
    assert_eq!(
        Programmed::from_objects(restarted.active_objects()).expect("restored"),
        Programmed::from_objects(programmer.transport().service.active_objects())
            .expect("programmed")
    );
}

#[test]
fn more_channels_than_the_interface_can_select_are_refused_before_activation() {
    let mut programmer = Programmer::connect(DeviceTransport::new()).expect("connect");
    let compiled = programmer
        .compiler()
        .compile(&project(u16::try_from(MAX_CHANNELS).unwrap() + 1))
        .expect("the store can stage more than the interface selects");
    let error = programmer
        .write_configuration_verified(&compiled)
        .expect_err("the device must refuse the over-large candidate");
    assert!(
        matches!(
            error,
            ProgrammerError::Device {
                command: Command::ValidateTransaction,
                code: DeviceErrorCode::ValidationFailed,
            }
        ),
        "unexpected error: {error:?}"
    );
    assert_eq!(programmer.transport().service.generation(), 0);
    assert_eq!(programmer.transport().service.active_objects().count(), 0);
}

#[test]
fn a_compact_generated_plan_is_written_once_and_expands_on_the_radio() {
    use radio_channel_control::ChannelSource;
    use radio_channel_plan::{GeneratedBank, PlanEncoding};

    let mut programmer = Programmer::connect(DeviceTransport::new()).expect("connect");
    assert_eq!(
        programmer.capabilities().plan_encodings,
        PlanEncoding::LinearSimplex.capability_bit(),
        "this image expands linear simplex plans itself"
    );

    let mut project = RadioProject::new();
    project.add_generated_bank(
        GeneratedBank::linear_simplex(
            BankId::new(3),
            BankName::new("PMR446").expect("name"),
            Frequency::from_hz(446_006_250).expect("frequency"),
            FrequencyStep::from_hz(12_500).expect("step"),
            16,
            TxClass::Never,
        )
        .expect("generated bank"),
    );
    let compiled = programmer
        .compiler()
        .compile(&project)
        .expect("compile against negotiated capabilities");
    // Sixteen channels cost one object, which is the whole point of the plan.
    assert_eq!(compiled.report().object_count, 1);
    programmer
        .write_configuration_verified(&compiled)
        .expect("transactional write with read-back");

    let activated = Programmed::from_objects(programmer.transport().service.active_objects())
        .expect("programmed configuration");
    assert_eq!(activated.channel_count(), 16);
    assert_eq!(
        activated.memory().stored_len(),
        0,
        "no channel record was stored for an expanded channel"
    );
    let first = activated.memory().get(0).expect("first expanded channel");
    assert_eq!(first.name().as_str(), "PMR446 01");
    assert_eq!(first.receive().as_hz(), 446_006_250);
    assert_eq!(first.tx_class(), TxClass::Never);
    assert!(first.is_member_of(BankId::new(3)));
    assert_eq!(
        activated.memory().get(15).expect("last").receive().as_hz(),
        446_193_750
    );

    // The plan names and populates its own bank, so the operator can filter to
    // it without a named-bank object beside it.
    let (banks, count) = activated.populated_banks();
    assert_eq!(count, 1);
    assert_eq!(banks[0], Some(BankId::new(3)));
    assert_eq!(
        activated
            .bank_name(BankId::new(3))
            .expect("plan name")
            .as_str(),
        "PMR446"
    );
}
