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
use radio_domain::{
    Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel, RadioConfig,
    SquelchLevel, Tone, TxClass,
};
use radio_firmware_k1::configuration::{
    device_service, K1DeviceService, Programmed, CONFIGURATION_STORE_BYTES, RETAINED_IMAGE_BYTES,
};
use radio_programmer::{CompileError, Programmer, ProtocolTransport, RadioProject};

/// Explicit channels these tests program.
///
/// Nothing about the image fixes this number any more; it is simply a full
/// project which comfortably fits the bytes the K1 declares.
const CHANNELS: u16 = 8;
use radio_protocol::MAX_ENCODED_FRAME;

/// The device service behind an in-process byte stream.
struct DeviceTransport {
    service: K1DeviceService,
    queue: std::collections::VecDeque<u8>,
}

impl DeviceTransport {
    fn new() -> Self {
        Self {
            service: device_service(),
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
        capabilities.configuration_bytes,
        u32::try_from(CONFIGURATION_STORE_BYTES).unwrap(),
        "the device declares the one number which bounds a configuration"
    );

    let project = project(CHANNELS);
    let compiled = programmer
        .compiler()
        .compile(&project)
        .expect("compile against negotiated capabilities");
    let receipt = programmer
        .write_configuration_verified(&compiled)
        .expect("transactional write with read-back");
    assert_eq!(receipt.generation, 1);

    // The image decodes the active snapshot with the same code the radio runs.
    let activated = Programmed::index(programmer.transport().service.active_objects())
        .expect("programmed configuration");
    assert_eq!(activated.channel_count(), CHANNELS);
    assert_eq!(activated.config().backlight_seconds, 30);
    assert_eq!(
        activated
            .bank_name(
                programmer.transport().service.active_objects(),
                BankId::new(1)
            )
            .expect("named bank")
            .as_str(),
        "VHF ODD"
    );
    let (banks, count) = activated.populated_banks();
    assert_eq!(count, 2);
    assert_eq!(banks[0], Some(BankId::new(0)));

    // Every programmed channel keeps the class the host wrote, and this image
    // has no transmit path regardless.
    for index in 0..activated.channel_count() {
        let channel = activated
            .channel_at(programmer.transport().service.active_objects(), index)
            .expect("channel");
        assert_eq!(channel.tx_class(), TxClass::Never);
    }
}

#[test]
fn a_written_configuration_survives_the_retained_image_round_trip() {
    let mut programmer = Programmer::connect(DeviceTransport::new()).expect("connect");
    let compiled = programmer
        .compiler()
        .compile(&project(CHANNELS))
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
    let mut restarted = device_service();
    assert_eq!(restarted.load_image(&image[..length]), Ok(1));
    assert_eq!(
        Programmed::index(restarted.active_objects()).expect("restored"),
        Programmed::index(programmer.transport().service.active_objects()).expect("programmed")
    );
}

/// A project is refused for the bytes it needs, and for nothing else.
#[test]
fn a_configuration_larger_than_the_declared_bytes_is_refused_by_the_host() {
    let programmer = Programmer::connect(DeviceTransport::new()).expect("connect");
    let available = u32::try_from(CONFIGURATION_STORE_BYTES).unwrap();

    // Explicit channels are the expensive way to fill a store, which is the
    // point: the same bytes buy thousands of channels as plans.
    let fits = u16::try_from(available / 47).expect("channels which fit");
    let compiled = programmer
        .compiler()
        .compile(&project(fits - 2))
        .expect("a project which fits is compiled");
    assert!(compiled.report().storage_bytes <= available);
    assert!(
        compiled.report().explicit_channels > 20,
        "a byte-bounded store holds far more explicit channels than a fixed table did"
    );

    let error = programmer
        .compiler()
        .compile(&project(fits + 8))
        .expect_err("a project which does not fit is refused");
    match error {
        CompileError::ConfigurationTooLarge {
            needed,
            available: declared,
        } => {
            assert!(needed > declared);
            assert_eq!(declared, available);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn a_compact_generated_plan_is_written_once_and_expands_on_the_radio() {
    use radio_channel_plan::{GeneratedBank, PlanEncoding};

    let mut programmer = Programmer::connect(DeviceTransport::new()).expect("connect");
    assert_eq!(
        programmer.capabilities().plan_encodings,
        PlanEncoding::LinearSimplex.capability_bit()
            | PlanEncoding::LinearFixedOffset.capability_bit(),
        "this image expands both arithmetic plan families itself"
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

    let activated = Programmed::index(programmer.transport().service.active_objects())
        .expect("programmed configuration");
    assert_eq!(activated.channel_count(), 16);
    assert_eq!(
        activated.stored_channels(),
        0,
        "no channel record was stored for an expanded channel"
    );
    let first = activated
        .channel_at(programmer.transport().service.active_objects(), 0)
        .expect("first expanded channel");
    assert_eq!(first.name().as_str(), "PMR 1");
    assert_eq!(first.receive().as_hz(), 446_006_250);
    assert_eq!(first.tx_class(), TxClass::Never);
    assert!(first.is_member_of(BankId::new(3)));
    assert_eq!(
        activated
            .channel_at(programmer.transport().service.active_objects(), 15)
            .expect("last")
            .receive()
            .as_hz(),
        446_193_750
    );

    // The plan names and populates its own bank, so the operator can filter to
    // it without a named-bank object beside it.
    let (banks, count) = activated.populated_banks();
    assert_eq!(count, 1);
    assert_eq!(banks[0], Some(BankId::new(3)));
    assert_eq!(
        activated
            .bank_name(
                programmer.transport().service.active_objects(),
                BankId::new(3)
            )
            .expect("plan name")
            .as_str(),
        "PMR446"
    );
}
