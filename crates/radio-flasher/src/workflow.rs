use std::{error::Error, fmt, io, io::Read, io::Write};

use crate::{
    codec::{receive_packet, receive_packet_with_response_crc, send_packet, Packet},
    image::{ApplicationImage, EepromBackup, ImageError, EEPROM_BYTES, FLASH_PAGE_BYTES},
};

const SESSION_WORD: u32 = 0x6457_396A;
const COMMAND_HELLO_REQUEST: u16 = 0x0514;
const COMMAND_HELLO_RESPONSE: u16 = 0x0515;
const COMMAND_KEYPAD_REQUEST: u16 = 0x7F10;
const COMMAND_KEYPAD_RESPONSE: u16 = 0x7F11;
const COMMAND_CLOCK_REQUEST: u16 = 0x7F12;
const COMMAND_CLOCK_RESPONSE: u16 = 0x7F13;
const COMMAND_CLOCK_REGISTER_REQUESTS: [u16; 4] = [0x7F14, 0x7F16, 0x7F18, 0x7F1A];
const COMMAND_CLOCK_REGISTER_RESPONSES: [u16; 4] = [0x7F15, 0x7F17, 0x7F19, 0x7F1B];
const COMMAND_CLOCK_CONTROL_REQUEST: u16 = 0x7F1C;
const COMMAND_CLOCK_CONTROL_RESPONSE: u16 = 0x7F1D;
const COMMAND_RF_REQUEST: u16 = 0x7F1E;
const COMMAND_RF_RESPONSE: u16 = 0x7F1F;
const COMMAND_RF_AUDIO_ON_REQUEST: u16 = 0x7F20;
const COMMAND_RF_AUDIO_OFF_REQUEST: u16 = 0x7F22;
const CLOCK_CONTROL_MARKER: u32 = 0x4B31_434C;
const COMMAND_READ_EEPROM_REQUEST: u16 = 0x051B;
const COMMAND_READ_EEPROM_RESPONSE: u16 = 0x051C;
const COMMAND_V2_BEACON: u16 = 0x0518;
const COMMAND_V2_PAGE_REQUEST: u16 = 0x0519;
const COMMAND_V2_PAGE_RESPONSE: u16 = 0x051A;
const COMMAND_V2_VERSION_REQUEST: u16 = 0x0530;
const COMMAND_V5_BEACON: u16 = 0x057A;
const EEPROM_BLOCK_BYTES: usize = 0x80;
const MAX_BEACONS_BEFORE_ACK: usize = 4;

/// Exact destructive target phrase required by the library.
pub const QUALIFIED_TARGET_CONFIRMATION: &str = "UV-K5-V1-DP32G030";
/// Exact additional phrase required before an AFIK rather than recovery image.
pub const RECOVERY_REHEARSED_CONFIRMATION: &str = "RECOVERY-REHEARSED-ON-THIS-UNIT";

/// Failure from bounded serial framing, validation, backup, or flashing.
#[derive(Debug)]
pub enum FlashError {
    /// Host transport I/O failed.
    Io(io::Error),
    /// The configured transport returned no byte within the bounded read budget.
    Timeout,
    /// A packet exceeded the largest observed request/response payload.
    PacketTooLarge(usize),
    /// An otherwise bounded packet did not end with the exact footer.
    InvalidFooter([u8; 2]),
    /// A radio response did not contain the observed decoded `0xFFFF` trailer.
    InvalidResponseCrc(u16),
    /// No packet header was found within the bounded resynchronisation window.
    SyncLimit,
    /// A response command, declared length, or actual length was unexpected.
    UnexpectedPacket {
        /// Expected response description.
        expected: &'static str,
        /// Actual command when at least four bytes were available.
        command: Option<u16>,
        /// Actual decoded payload length.
        length: usize,
    },
    /// A bounded text field was empty, malformed, or unsupported.
    InvalidText(&'static str),
    /// Normal firmware reported a custom key or password lock.
    ProtectedFirmware,
    /// A beacon belongs to bootloader v5 or another unsupported protocol.
    UnsupportedBootloader(u16),
    /// The operator did not supply the exact qualified-hardware phrase.
    TargetNotConfirmed,
    /// The typed image CRC-32 did not select the actual padded image.
    ImageNotConfirmed {
        /// CRC-32 calculated from the selected complete image.
        expected: u32,
        /// CRC-32 typed by the operator.
        supplied: u32,
    },
    /// Recovery rehearsal selected a different image than the recovery image.
    InvalidRecoveryRehearsal,
    /// AFIK flashing lacked the exact same-unit recovery-rehearsal phrase.
    RecoveryNotRehearsed,
    /// AFIK and recovery inputs resolved to the same complete padded image.
    RecoveryImageMatchesApplication,
    /// The explicit per-run transaction identifier was zero.
    InvalidTransactionId,
    /// An EEPROM block response did not match the exact requested range.
    EepromBlockMismatch {
        /// Requested EEPROM offset.
        expected_offset: u16,
        /// Offset named by the response.
        actual_offset: u16,
        /// Requested block length.
        expected_length: u8,
        /// Length named by the response.
        actual_length: u8,
    },
    /// A completed read did not form a credible complete EEPROM backup.
    InvalidEepromBackup(ImageError),
    /// A page acknowledgement named the wrong transaction or page.
    PageAcknowledgementMismatch {
        /// Sequential page that was awaiting acknowledgement.
        expected_page: u16,
    },
    /// The bootloader returned a nonzero page result.
    PageRejected {
        /// Page named by the exact acknowledgement.
        page: u16,
        /// Nonzero bootloader result.
        result: u16,
    },
}

impl fmt::Display for FlashError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "serial I/O failed: {error}"),
            Self::Timeout => formatter.write_str("serial response timed out"),
            Self::PacketTooLarge(length) => write!(formatter, "packet is too large: {length}"),
            Self::InvalidFooter(footer) => {
                write!(formatter, "invalid packet footer: {:02x}{:02x}", footer[0], footer[1])
            }
            Self::InvalidResponseCrc(crc) => {
                write!(formatter, "unexpected decoded radio CRC trailer: 0x{crc:04x}")
            }
            Self::SyncLimit => formatter.write_str("packet resynchronisation limit reached"),
            Self::UnexpectedPacket {
                expected,
                command,
                length,
            } => write!(
                formatter,
                "unexpected response for {expected}: command={command:?}, length={length}"
            ),
            Self::InvalidText(field) => write!(formatter, "invalid {field}"),
            Self::ProtectedFirmware => {
                formatter.write_str("custom-key or password-protected firmware is unsupported")
            }
            Self::UnsupportedBootloader(command) => {
                write!(formatter, "unsupported bootloader beacon: 0x{command:04x}")
            }
            Self::TargetNotConfirmed => formatter.write_str("qualified K5 V1 target not confirmed"),
            Self::ImageNotConfirmed { expected, supplied } => write!(
                formatter,
                "image CRC-32 confirmation mismatch: expected {expected:08x}, supplied {supplied:08x}"
            ),
            Self::InvalidRecoveryRehearsal => formatter.write_str(
                "recovery rehearsal must flash the exact validated recovery image",
            ),
            Self::RecoveryNotRehearsed => {
                formatter.write_str("same-unit recovery rehearsal not confirmed")
            }
            Self::RecoveryImageMatchesApplication => formatter.write_str(
                "AFIK application and recovery image must be distinct",
            ),
            Self::InvalidTransactionId => {
                formatter.write_str("flash transaction identifier must be nonzero")
            }
            Self::EepromBlockMismatch {
                expected_offset,
                actual_offset,
                expected_length,
                actual_length,
            } => write!(
                formatter,
                "EEPROM response mismatch: expected {expected_offset:04x}/{expected_length:02x}, got {actual_offset:04x}/{actual_length:02x}"
            ),
            Self::InvalidEepromBackup(error) => write!(formatter, "invalid EEPROM backup: {error}"),
            Self::PageAcknowledgementMismatch { expected_page } => write!(
                formatter,
                "page acknowledgement does not match page 0x{expected_page:04x}"
            ),
            Self::PageRejected { page, result } => {
                write!(formatter, "bootloader rejected page 0x{page:04x}: result={result}")
            }
        }
    }
}

impl Error for FlashError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::InvalidEepromBackup(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for FlashError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// A strictly validated version-2 firmware-family negotiation string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FirmwareVersion {
    bytes: [u8; 16],
    len: usize,
}

impl FirmwareVersion {
    /// Accepts only a non-wildcard ASCII `2.` version composed of digits/dots.
    pub fn new(version: &str) -> Result<Self, FlashError> {
        let raw = version.as_bytes();
        if raw.len() < 3
            || raw.len() > 16
            || !raw.starts_with(b"2.")
            || raw
                .iter()
                .any(|byte| !byte.is_ascii_digit() && *byte != b'.')
        {
            return Err(FlashError::InvalidText("version-2 firmware string"));
        }
        let mut bytes = [0_u8; 16];
        bytes[..raw.len()].copy_from_slice(raw);
        Ok(Self {
            bytes,
            len: raw.len(),
        })
    }

    /// Returns the validated printable version.
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or_default()
    }
}

/// Read-only identity returned by normal firmware before EEPROM backup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalFirmwareInfo {
    version: String,
}

/// Raw receive-only K1 main-key matrix observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeypadMatrixReport {
    gpio_b_idr_by_column: [u16; 4],
    scan_valid: bool,
    captured: bool,
}

impl KeypadMatrixReport {
    /// Returns raw GPIOB IDR snapshots in PB6, PB5, PB4, PB3 column order.
    pub const fn gpio_b_idr_by_column(&self) -> [u16; 4] {
        self.gpio_b_idr_by_column
    }

    /// Returns whether the target completed the raw four-column scan.
    pub const fn scan_valid(&self) -> bool {
        self.scan_valid
    }

    /// Returns whether the rows were latched from a nonzero scan since the
    /// previous successful probe.
    pub const fn captured(&self) -> bool {
        self.captured
    }
}

/// Raw read-only K1 receive observation returned by the AFIK application.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RfReport {
    identity_register: u16,
    identity_address: u8,
    stage: u8,
    frequency_hz: u32,
    rssi_dbm_x2: i16,
    glitch: u8,
    noise: u8,
    squelch_open: bool,
    samples: u16,
    audio_routed: bool,
}

impl RfReport {
    /// Returns whether demodulated audio is routed to the speaker.
    pub const fn audio_routed(&self) -> bool {
        self.audio_routed
    }

    /// Returns the value read back from the receiver after configuration.
    pub const fn identity_register(&self) -> u16 {
        self.identity_register
    }

    /// Returns the register address the read-back value came from.
    pub const fn identity_address(&self) -> u8 {
        self.identity_address
    }

    /// Returns the bring-up stage the target reached.
    pub const fn stage(&self) -> u8 {
        self.stage
    }

    /// Returns the tuned receive frequency in hertz, or zero before tuning.
    pub const fn frequency_hz(&self) -> u32 {
        self.frequency_hz
    }

    /// Returns approximate RSSI multiplied by two, in half-dBm steps.
    pub const fn rssi_dbm_x2(&self) -> i16 {
        self.rssi_dbm_x2
    }

    /// Returns the raw glitch indicator.
    pub const fn glitch(&self) -> u8 {
        self.glitch
    }

    /// Returns the raw excess-noise indicator.
    pub const fn noise(&self) -> u8 {
        self.noise
    }

    /// Returns whether the carrier squelch link reads open.
    pub const fn squelch_open(&self) -> bool {
        self.squelch_open
    }

    /// Returns how many metric samples the target has completed.
    pub const fn samples(&self) -> u16 {
        self.samples
    }
}

/// Raw read-only K1 inherited-clock observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockSnapshotReport {
    registers: [u32; 4],
    contract_valid: bool,
}

impl ClockSnapshotReport {
    /// Returns RCC CR, ICSCR, CFGR, and PLLCFGR in that order.
    pub const fn registers(&self) -> [u32; 4] {
        self.registers
    }

    /// Returns whether the target's fail-closed inherited-clock contract passed.
    pub const fn contract_valid(&self) -> bool {
        self.contract_valid
    }
}

impl NormalFirmwareInfo {
    /// Returns the exact bounded normal-firmware version text.
    pub fn version(&self) -> &str {
        &self.version
    }
}

/// Qualified version-2 bootloader observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BootloaderInfo {
    version: String,
}

impl BootloaderInfo {
    /// Returns the exact printable version from the accepted beacon.
    pub fn version(&self) -> &str {
        &self.version
    }
}

/// Protocol family classified from a validated bootloader beacon.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BootloaderFamily {
    /// Qualified UV-K5 V1/version-2 protocol (`2.*` beacon).
    K5V1(BootloaderInfo),
    /// Pinned UV-K1 protocol (`7.03.*` beacon).
    K1(BootloaderInfo),
}

impl BootloaderFamily {
    /// Returns the validated bootloader information.
    pub fn info(&self) -> &BootloaderInfo {
        match self {
            Self::K5V1(info) | Self::K1(info) => info,
        }
    }

    /// Returns the stable protocol-family label.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::K5V1(_) => "K5-V1",
            Self::K1(_) => "K1",
        }
    }
}

/// Whether the selected image is the recovery rehearsal or an AFIK attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlashPurpose<'a> {
    /// The selected image must be exactly the separately supplied recovery image.
    RecoveryRehearsal,
    /// The selected image must differ from recovery and use the exact phrase.
    Afik {
        /// Must equal [`RECOVERY_REHEARSED_CONFIRMATION`].
        recovery_rehearsed_confirmation: &'a str,
    },
}

/// Complete prerequisites validated before the first serial read or write.
#[derive(Clone, Copy, Debug)]
pub struct FlashPrerequisites<'a> {
    /// Complete selected application after vector checks and `0xFF` padding.
    pub image: &'a ApplicationImage,
    /// Known-good recovery application for the exact qualified unit.
    pub recovery_image: &'a ApplicationImage,
    /// Complete non-uniform EEPROM/calibration backup from the exact unit.
    pub eeprom_backup: &'a EepromBackup,
    /// Non-wildcard version-2 negotiation value.
    pub version: &'a FirmwareVersion,
    /// Exact target phrase proving deliberate V1/DP32 selection.
    pub target_confirmation: &'a str,
    /// Exact CRC-32 of the selected padded image.
    pub image_crc32_confirmation: u32,
    /// Explicit nonzero per-run identifier echoed by every page response.
    pub transaction_id: u32,
    /// Recovery rehearsal versus separately confirmed AFIK attempt.
    pub purpose: FlashPurpose<'a>,
}

/// Result after every application page returned an exact zero-result ack.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlashReport {
    /// Accepted bootloader version.
    pub bootloader: BootloaderInfo,
    /// Number of sequential pages acknowledged.
    pub pages_acknowledged: u16,
    /// CRC-32 selection guard for the complete written application request.
    pub image_crc32: u32,
    /// Per-run transaction identifier echoed by every accepted page response.
    pub transaction_id: u32,
}

/// Reads and validates the complete 8 KiB EEPROM without exposing any write.
pub fn backup_eeprom<T: Read + Write>(
    transport: &mut T,
) -> Result<(NormalFirmwareInfo, EepromBackup), FlashError> {
    let hello = hello_request();
    send_packet(transport, &hello)?;
    let info = parse_hello_response(&receive_packet(transport)?)?;

    let mut bytes = vec![0_u8; EEPROM_BYTES];
    for block in 0_u16..64 {
        let offset_u16 = block * 0x80;
        let offset = usize::from(offset_u16);
        let request = eeprom_read_request(offset_u16);
        send_packet(transport, &request)?;
        let packet = receive_packet(transport)?;
        parse_eeprom_response(
            &packet,
            offset_u16,
            &mut bytes[offset..offset + EEPROM_BLOCK_BYTES],
        )?;
    }
    let backup = EepromBackup::from_raw(&bytes).map_err(FlashError::InvalidEepromBackup)?;
    Ok((info, backup))
}

/// Sends the observed read-only normal-firmware hello and returns its identity.
///
/// This is intentionally separate from [`backup_eeprom`]: it performs one
/// hello exchange and never requests EEPROM/configuration data. It is the
/// host-side witness for an AFIK application running over the K1 serial path.
pub fn probe_normal_firmware<T: Read + Write>(
    transport: &mut T,
) -> Result<NormalFirmwareInfo, FlashError> {
    let hello = hello_request();
    send_packet(transport, &hello)?;
    parse_hello_response(&receive_packet(transport)?)
}

/// Requests one raw, receive-only main-key matrix observation.
pub fn probe_keypad_matrix<T: Read + Write>(
    transport: &mut T,
) -> Result<KeypadMatrixReport, FlashError> {
    send_packet(transport, &session_request(COMMAND_KEYPAD_REQUEST))?;
    parse_keypad_response(&receive_packet(transport)?)
}

/// Requests one raw, read-only BK4819 receive observation.
pub fn probe_rf<T: Read + Write>(transport: &mut T) -> Result<RfReport, FlashError> {
    send_packet(transport, &session_request(COMMAND_RF_REQUEST))?;
    parse_rf_response(&receive_packet(transport)?)
}

/// Routes or mutes demodulated receive audio and returns a fresh observation.
///
/// This drives the receive audio chain only and carries no transmit capability.
pub fn set_rf_audio<T: Read + Write>(
    transport: &mut T,
    routed: bool,
) -> Result<RfReport, FlashError> {
    let command = if routed {
        COMMAND_RF_AUDIO_ON_REQUEST
    } else {
        COMMAND_RF_AUDIO_OFF_REQUEST
    };
    send_packet(transport, &session_request(command))?;
    parse_rf_response(&receive_packet(transport)?)
}

/// Requests one raw, read-only inherited RCC clock observation.
pub fn probe_clock_snapshot<T: Read + Write>(
    transport: &mut T,
) -> Result<ClockSnapshotReport, FlashError> {
    send_packet(transport, &session_request(COMMAND_CLOCK_REQUEST))?;
    parse_clock_response(&receive_packet(transport)?)
}

/// Requests the four inherited RCC registers as individually identified reads.
pub fn probe_clock_registers<T: Read + Write>(transport: &mut T) -> Result<[u32; 4], FlashError> {
    let mut registers = [0_u32; 4];
    for (index, register) in registers.iter_mut().enumerate() {
        *register = probe_clock_register(transport, index)?;
    }
    Ok(registers)
}

/// Requests one indexed inherited RCC register without touching the others.
pub fn probe_clock_register<T: Read + Write>(
    transport: &mut T,
    index: usize,
) -> Result<u32, FlashError> {
    let Some(command) = COMMAND_CLOCK_REGISTER_REQUESTS.get(index) else {
        return Err(FlashError::UnexpectedPacket {
            expected: "AFIK K1 RCC register index 0..3",
            command: None,
            length: 0,
        });
    };
    send_packet(transport, &session_request(*command))?;
    parse_clock_register_response(&receive_packet(transport)?, index)
}

/// Exercises the clock diagnostic command/response path without any MMIO read.
pub fn probe_clock_control<T: Read + Write>(transport: &mut T) -> Result<u32, FlashError> {
    send_packet(transport, &session_request(COMMAND_CLOCK_CONTROL_REQUEST))?;
    let packet = receive_packet(transport)?;
    let payload = packet.as_slice();
    require_packet(
        payload,
        COMMAND_CLOCK_CONTROL_RESPONSE,
        8,
        12,
        "AFIK K1 no-MMIO clock control",
    )?;
    if payload[4] != 0xA5 || payload[5..8] != [0, 0, 0] {
        return Err(FlashError::UnexpectedPacket {
            expected: "bounded AFIK K1 no-MMIO clock control fields",
            command: Some(COMMAND_CLOCK_CONTROL_RESPONSE),
            length: payload.len(),
        });
    }
    let mut marker_bytes = [0_u8; 4];
    marker_bytes.copy_from_slice(&payload[8..12]);
    let marker = u32::from_le_bytes(marker_bytes);
    if marker != CLOCK_CONTROL_MARKER {
        return Err(FlashError::UnexpectedPacket {
            expected: "AFIK K1 no-MMIO clock control marker",
            command: Some(COMMAND_CLOCK_CONTROL_RESPONSE),
            length: payload.len(),
        });
    }
    Ok(marker)
}

/// Waits for one exact, printable version-2 bootloader beacon.
pub fn probe_bootloader_v2<T: Read>(transport: &mut T) -> Result<BootloaderInfo, FlashError> {
    parse_bootloader_beacon(&receive_packet(transport)?)
}

/// Reads one beacon and classifies only the pinned K1 or qualified K5 protocol.
pub fn detect_bootloader<T: Read>(transport: &mut T) -> Result<BootloaderFamily, FlashError> {
    let (packet, response_crc) = receive_packet_with_response_crc(transport)?;
    let family = parse_bootloader_family(&packet)?;
    if response_crc != 0xFFFF && !matches!(family, BootloaderFamily::K1(_)) {
        return Err(FlashError::InvalidResponseCrc(response_crc));
    }
    Ok(family)
}

/// Writes all 240 application pages after validating every prerequisite.
///
/// The callback runs after each exact acknowledgement. Missing or ambiguous
/// acknowledgements stop immediately and are never retried.
pub fn flash_application<T, F>(
    transport: &mut T,
    prerequisites: FlashPrerequisites<'_>,
    mut page_acknowledged: F,
) -> Result<FlashReport, FlashError>
where
    T: Read + Write,
    F: FnMut(u16),
{
    validate_prerequisites(prerequisites)?;
    let bootloader = probe_bootloader_v2(transport)?;

    let version_request = version_request(prerequisites.version);
    send_packet(transport, &version_request)?;
    let confirmed_bootloader = probe_bootloader_v2(transport)?;
    if confirmed_bootloader != bootloader {
        return Err(FlashError::UnexpectedPacket {
            expected: "stable bootloader version negotiation",
            command: Some(COMMAND_V2_BEACON),
            length: 36,
        });
    }

    for page_number in 0_u16..240 {
        let start = usize::from(page_number) * FLASH_PAGE_BYTES;
        let request = page_request(
            prerequisites.transaction_id,
            page_number,
            &prerequisites.image.bytes()[start..start + FLASH_PAGE_BYTES],
        );
        send_packet(transport, &request)?;
        receive_page_acknowledgement(transport, prerequisites.transaction_id, page_number)?;
        page_acknowledged(page_number);
    }

    Ok(FlashReport {
        bootloader,
        pages_acknowledged: 240,
        image_crc32: prerequisites.image.crc32(),
        transaction_id: prerequisites.transaction_id,
    })
}

fn validate_prerequisites(prerequisites: FlashPrerequisites<'_>) -> Result<(), FlashError> {
    if prerequisites.target_confirmation != QUALIFIED_TARGET_CONFIRMATION {
        return Err(FlashError::TargetNotConfirmed);
    }
    if prerequisites.image_crc32_confirmation != prerequisites.image.crc32() {
        return Err(FlashError::ImageNotConfirmed {
            expected: prerequisites.image.crc32(),
            supplied: prerequisites.image_crc32_confirmation,
        });
    }
    if prerequisites.transaction_id == 0 {
        return Err(FlashError::InvalidTransactionId);
    }
    match prerequisites.purpose {
        FlashPurpose::RecoveryRehearsal => {
            if prerequisites.image.bytes() != prerequisites.recovery_image.bytes() {
                return Err(FlashError::InvalidRecoveryRehearsal);
            }
        }
        FlashPurpose::Afik {
            recovery_rehearsed_confirmation,
        } => {
            if recovery_rehearsed_confirmation != RECOVERY_REHEARSED_CONFIRMATION {
                return Err(FlashError::RecoveryNotRehearsed);
            }
            if prerequisites.image.bytes() == prerequisites.recovery_image.bytes() {
                return Err(FlashError::RecoveryImageMatchesApplication);
            }
        }
    }
    Ok(())
}

fn hello_request() -> [u8; 8] {
    session_request(COMMAND_HELLO_REQUEST)
}

fn session_request(command: u16) -> [u8; 8] {
    let mut payload = [0_u8; 8];
    payload[0..2].copy_from_slice(&command.to_le_bytes());
    payload[2..4].copy_from_slice(&4_u16.to_le_bytes());
    payload[4..8].copy_from_slice(&SESSION_WORD.to_le_bytes());
    payload
}

fn parse_rf_response(packet: &Packet) -> Result<RfReport, FlashError> {
    let payload = packet.as_slice();
    require_packet(
        payload,
        COMMAND_RF_RESPONSE,
        16,
        20,
        "AFIK K1 raw receive observation",
    )?;
    if payload[16] > 1 || payload[17] > 1 {
        return Err(FlashError::UnexpectedPacket {
            expected: "bounded AFIK K1 raw receive fields",
            command: Some(COMMAND_RF_RESPONSE),
            length: payload.len(),
        });
    }
    Ok(RfReport {
        identity_register: u16::from_le_bytes([payload[4], payload[5]]),
        identity_address: payload[6],
        stage: payload[7],
        frequency_hz: u32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]),
        rssi_dbm_x2: i16::from_le_bytes([payload[12], payload[13]]),
        glitch: payload[14],
        noise: payload[15],
        squelch_open: payload[16] == 1,
        samples: u16::from_le_bytes([payload[18], payload[19]]),
        audio_routed: payload[17] == 1,
    })
}

fn parse_keypad_response(packet: &Packet) -> Result<KeypadMatrixReport, FlashError> {
    let payload = packet.as_slice();
    require_packet(
        payload,
        COMMAND_KEYPAD_RESPONSE,
        12,
        16,
        "AFIK K1 raw GPIOB snapshots",
    )?;
    if payload[12] > 1 || payload[13] > 1 || payload[14..16] != [0, 0] {
        return Err(FlashError::UnexpectedPacket {
            expected: "bounded AFIK K1 raw GPIOB fields",
            command: Some(COMMAND_KEYPAD_RESPONSE),
            length: payload.len(),
        });
    }
    Ok(KeypadMatrixReport {
        gpio_b_idr_by_column: [
            u16::from_le_bytes([payload[4], payload[5]]),
            u16::from_le_bytes([payload[6], payload[7]]),
            u16::from_le_bytes([payload[8], payload[9]]),
            u16::from_le_bytes([payload[10], payload[11]]),
        ],
        scan_valid: payload[12] == 1,
        captured: payload[13] == 1,
    })
}

fn parse_clock_response(packet: &Packet) -> Result<ClockSnapshotReport, FlashError> {
    let payload = packet.as_slice();
    require_packet(
        payload,
        COMMAND_CLOCK_RESPONSE,
        20,
        24,
        "AFIK K1 raw RCC snapshot",
    )?;
    if payload[20] > 1 || payload[21..24] != [0, 0, 0] {
        return Err(FlashError::UnexpectedPacket {
            expected: "bounded AFIK K1 raw RCC fields",
            command: Some(COMMAND_CLOCK_RESPONSE),
            length: payload.len(),
        });
    }
    let mut registers = [0_u32; 4];
    for (index, register) in registers.iter_mut().enumerate() {
        let start = 4 + index * 4;
        *register = u32::from_le_bytes(payload[start..start + 4].try_into().expect("four bytes"));
    }
    Ok(ClockSnapshotReport {
        registers,
        contract_valid: payload[20] == 1,
    })
}

fn parse_clock_register_response(
    packet: &Packet,
    expected_index: usize,
) -> Result<u32, FlashError> {
    let payload = packet.as_slice();
    require_packet(
        payload,
        COMMAND_CLOCK_REGISTER_RESPONSES[expected_index],
        8,
        12,
        "AFIK K1 individually identified RCC register",
    )?;
    if payload[4] != u8::try_from(expected_index).expect("four registers")
        || payload[5..8] != [0, 0, 0]
    {
        return Err(FlashError::UnexpectedPacket {
            expected: "bounded AFIK K1 RCC register fields",
            command: Some(COMMAND_CLOCK_REGISTER_RESPONSES[expected_index]),
            length: payload.len(),
        });
    }
    Ok(u32::from_le_bytes(
        payload[8..12].try_into().expect("four bytes"),
    ))
}

fn parse_hello_response(packet: &Packet) -> Result<NormalFirmwareInfo, FlashError> {
    let payload = packet.as_slice();
    require_packet(
        payload,
        COMMAND_HELLO_RESPONSE,
        36,
        40,
        "normal-firmware hello",
    )?;
    if payload[20] != 0 || payload[21] != 0 {
        return Err(FlashError::ProtectedFirmware);
    }
    let version = parse_text(&payload[4..20], "normal-firmware version")?;
    Ok(NormalFirmwareInfo { version })
}

fn eeprom_read_request(offset: u16) -> [u8; 12] {
    let mut payload = [0_u8; 12];
    payload[0..2].copy_from_slice(&COMMAND_READ_EEPROM_REQUEST.to_le_bytes());
    payload[2..4].copy_from_slice(&8_u16.to_le_bytes());
    payload[4..6].copy_from_slice(&offset.to_le_bytes());
    payload[6] = u8::try_from(EEPROM_BLOCK_BYTES).expect("bounded EEPROM block");
    payload[8..12].copy_from_slice(&SESSION_WORD.to_le_bytes());
    payload
}

fn parse_eeprom_response(
    packet: &Packet,
    expected_offset: u16,
    output: &mut [u8],
) -> Result<(), FlashError> {
    let payload = packet.as_slice();
    let expected_length = u8::try_from(output.len()).expect("bounded EEPROM block");
    let expected_declared = u16::try_from(output.len() + 4).expect("bounded response");
    require_packet(
        payload,
        COMMAND_READ_EEPROM_RESPONSE,
        expected_declared,
        output.len() + 8,
        "EEPROM read",
    )?;
    let actual_offset = u16::from_le_bytes([payload[4], payload[5]]);
    let actual_length = payload[6];
    if actual_offset != expected_offset || actual_length != expected_length || payload[7] != 0 {
        return Err(FlashError::EepromBlockMismatch {
            expected_offset,
            actual_offset,
            expected_length,
            actual_length,
        });
    }
    output.copy_from_slice(&payload[8..]);
    Ok(())
}

fn parse_bootloader_beacon(packet: &Packet) -> Result<BootloaderInfo, FlashError> {
    let payload = packet.as_slice();
    let command = packet_command(payload);
    if command == Some(COMMAND_V5_BEACON) {
        return Err(FlashError::UnsupportedBootloader(COMMAND_V5_BEACON));
    }
    if command != Some(COMMAND_V2_BEACON) {
        return Err(FlashError::UnsupportedBootloader(command.unwrap_or(0)));
    }
    require_packet(payload, COMMAND_V2_BEACON, 32, 36, "bootloader-v2 beacon")?;
    let version = parse_text(&payload[20..36], "bootloader-v2 version")?;
    if !version.starts_with("2.")
        || version
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && byte != b'.')
    {
        return Err(FlashError::InvalidText("bootloader-v2 version"));
    }
    Ok(BootloaderInfo { version })
}

fn parse_bootloader_family(packet: &Packet) -> Result<BootloaderFamily, FlashError> {
    let payload = packet.as_slice();
    let command = packet_command(payload);
    if command == Some(COMMAND_V5_BEACON) {
        return Err(FlashError::UnsupportedBootloader(COMMAND_V5_BEACON));
    }
    require_packet(payload, COMMAND_V2_BEACON, 32, 36, "bootloader beacon")?;
    let version = parse_text(&payload[20..36], "bootloader version")?;
    if version.starts_with("2.")
        && version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return Ok(BootloaderFamily::K5V1(BootloaderInfo { version }));
    }
    if version.starts_with("7.03.")
        && version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return Ok(BootloaderFamily::K1(BootloaderInfo { version }));
    }
    Err(FlashError::InvalidText("supported bootloader version"))
}

fn version_request(version: &FirmwareVersion) -> [u8; 20] {
    let mut payload = [0_u8; 20];
    payload[0..2].copy_from_slice(&COMMAND_V2_VERSION_REQUEST.to_le_bytes());
    payload[2..4].copy_from_slice(&16_u16.to_le_bytes());
    payload[4..20].copy_from_slice(&version.bytes);
    payload
}

fn page_request(transaction_id: u32, page: u16, data: &[u8]) -> [u8; 272] {
    debug_assert_eq!(data.len(), FLASH_PAGE_BYTES);
    let mut payload = [0_u8; 272];
    payload[0..2].copy_from_slice(&COMMAND_V2_PAGE_REQUEST.to_le_bytes());
    payload[2..4].copy_from_slice(&0x010C_u16.to_le_bytes());
    payload[4..8].copy_from_slice(&transaction_id.to_le_bytes());
    payload[8..10].copy_from_slice(&page.to_le_bytes());
    payload[10..12].copy_from_slice(&240_u16.to_le_bytes());
    payload[12..14].copy_from_slice(&256_u16.to_le_bytes());
    payload[16..].copy_from_slice(data);
    payload
}

fn receive_page_acknowledgement<T: Read>(
    transport: &mut T,
    transaction_id: u32,
    expected_page: u16,
) -> Result<(), FlashError> {
    for _ in 0..=MAX_BEACONS_BEFORE_ACK {
        let packet = receive_packet(transport)?;
        if expected_page == 0 && packet_command(packet.as_slice()) == Some(COMMAND_V2_BEACON) {
            parse_bootloader_beacon(&packet)?;
            continue;
        }
        return parse_page_acknowledgement(&packet, transaction_id, expected_page);
    }
    Err(FlashError::PageAcknowledgementMismatch { expected_page })
}

fn parse_page_acknowledgement(
    packet: &Packet,
    transaction_id: u32,
    expected_page: u16,
) -> Result<(), FlashError> {
    let payload = packet.as_slice();
    require_packet(
        payload,
        COMMAND_V2_PAGE_RESPONSE,
        8,
        12,
        "bootloader page acknowledgement",
    )?;
    let transaction = u32::from_le_bytes(payload[4..8].try_into().expect("four bytes"));
    let page = u16::from_le_bytes([payload[8], payload[9]]);
    let result = u16::from_le_bytes([payload[10], payload[11]]);
    if transaction != transaction_id || page != expected_page {
        return Err(FlashError::PageAcknowledgementMismatch { expected_page });
    }
    if result != 0 {
        return Err(FlashError::PageRejected { page, result });
    }
    Ok(())
}

fn require_packet(
    payload: &[u8],
    expected_command: u16,
    expected_declared: u16,
    expected_length: usize,
    expected: &'static str,
) -> Result<(), FlashError> {
    let command = packet_command(payload);
    let declared = payload
        .get(2..4)
        .map(|bytes| u16::from_le_bytes(bytes.try_into().expect("two bytes")));
    if command != Some(expected_command)
        || declared != Some(expected_declared)
        || payload.len() != expected_length
    {
        return Err(FlashError::UnexpectedPacket {
            expected,
            command,
            length: payload.len(),
        });
    }
    Ok(())
}

fn packet_command(payload: &[u8]) -> Option<u16> {
    payload
        .get(0..2)
        .map(|bytes| u16::from_le_bytes(bytes.try_into().expect("two bytes")))
}

fn parse_text(bytes: &[u8], field: &'static str) -> Result<String, FlashError> {
    let length = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let text = &bytes[..length];
    if text.is_empty() || text.iter().any(|byte| !(0x20..=0x7E).contains(byte)) {
        return Err(FlashError::InvalidText(field));
    }
    Ok(std::str::from_utf8(text)
        .expect("validated ASCII text")
        .to_owned())
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, io, io::Read, io::Write};

    use crate::{
        codec::{crc16_xmodem, encode_response, encode_response_with_trailer},
        image::{ApplicationImage, EepromBackup, EEPROM_BYTES, FLASH_PAGE_COUNT},
    };

    use super::{
        backup_eeprom, detect_bootloader, flash_application, probe_bootloader_v2,
        probe_clock_control, probe_clock_registers, probe_clock_snapshot, probe_keypad_matrix,
        probe_normal_firmware, BootloaderFamily, FirmwareVersion, FlashError, FlashPrerequisites,
        FlashPurpose, QUALIFIED_TARGET_CONFIRMATION, RECOVERY_REHEARSED_CONFIRMATION,
    };

    const TEST_TRANSACTION_ID: u32 = 0xA55A_1234;

    struct ScriptedTransport {
        reads: VecDeque<u8>,
        writes: Vec<u8>,
        read_fragment: usize,
    }

    impl ScriptedTransport {
        fn new(responses: Vec<u8>, read_fragment: usize) -> Self {
            Self {
                reads: responses.into(),
                writes: Vec::new(),
                read_fragment,
            }
        }
    }

    impl Read for ScriptedTransport {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = buffer.len().min(self.read_fragment).min(self.reads.len());
            for byte in &mut buffer[..count] {
                *byte = self.reads.pop_front().expect("bounded response queue");
            }
            Ok(count)
        }
    }

    impl Write for ScriptedTransport {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.writes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn valid_raw(fill: u8) -> Vec<u8> {
        let mut bytes = vec![fill; 32];
        bytes[0..4].copy_from_slice(&0x2000_4000_u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&9_u32.to_le_bytes());
        bytes
    }

    fn valid_backup() -> EepromBackup {
        let bytes = (0_u8..=u8::MAX)
            .cycle()
            .take(EEPROM_BYTES)
            .collect::<Vec<_>>();
        EepromBackup::from_raw(&bytes).unwrap()
    }

    fn beacon() -> Vec<u8> {
        let mut payload = vec![0_u8; 36];
        payload[0..4].copy_from_slice(&[0x18, 0x05, 0x20, 0x00]);
        payload[4..20].copy_from_slice(&[
            0x01, 0x02, 0x02, 0x06, 0x1C, 0x53, 0x50, 0x4A, 0x37, 0x47, 0xFF, 0x0F, 0x8C, 0x00,
            0x53, 0x00,
        ]);
        payload[20..28].copy_from_slice(b"2.00.06\0");
        payload[28..36].copy_from_slice(&[0x34, 0x0A, 0, 0, 0, 0, 0, 0x20]);
        encode_response(&payload)
    }

    fn acknowledgement(page: u16, result: u16) -> Vec<u8> {
        let mut payload = [0_u8; 12];
        payload[0..4].copy_from_slice(&[0x1A, 0x05, 0x08, 0x00]);
        payload[4..8].copy_from_slice(&TEST_TRANSACTION_ID.to_le_bytes());
        payload[8..10].copy_from_slice(&page.to_le_bytes());
        payload[10..12].copy_from_slice(&result.to_le_bytes());
        encode_response(&payload)
    }

    fn afik_prerequisites<'a>(
        image: &'a ApplicationImage,
        recovery: &'a ApplicationImage,
        backup: &'a EepromBackup,
        version: &'a FirmwareVersion,
    ) -> FlashPrerequisites<'a> {
        FlashPrerequisites {
            image,
            recovery_image: recovery,
            eeprom_backup: backup,
            version,
            target_confirmation: QUALIFIED_TARGET_CONFIRMATION,
            image_crc32_confirmation: image.crc32(),
            transaction_id: TEST_TRANSACTION_ID,
            purpose: FlashPurpose::Afik {
                recovery_rehearsed_confirmation: RECOVERY_REHEARSED_CONFIRMATION,
            },
        }
    }

    #[test]
    fn complete_flash_writes_exact_pages_in_order_with_one_byte_reads() {
        let image = ApplicationImage::from_raw(&valid_raw(0xA5)).unwrap();
        let recovery = ApplicationImage::from_raw(&valid_raw(0x5A)).unwrap();
        let backup = valid_backup();
        let version = FirmwareVersion::new("2.01.23").unwrap();
        let mut responses = beacon();
        responses.extend(beacon());
        for page in 0..FLASH_PAGE_COUNT {
            responses.extend(acknowledgement(u16::try_from(page).unwrap(), 0));
        }
        let mut transport = ScriptedTransport::new(responses, 1);
        let mut progress = Vec::new();
        let report = flash_application(
            &mut transport,
            afik_prerequisites(&image, &recovery, &backup, &version),
            |page| progress.push(page),
        )
        .unwrap();

        assert_eq!(report.bootloader.version(), "2.00.06");
        assert_eq!(report.pages_acknowledged, 240);
        assert_eq!(progress, (0..240).collect::<Vec<_>>());
        let requests = decode_requests(&transport.writes);
        assert_eq!(requests.len(), 241);
        assert_eq!(&requests[0][0..4], &[0x30, 0x05, 0x10, 0x00]);
        assert_eq!(&requests[0][4..11], b"2.01.23");
        assert_eq!(requests[0][11], 0);
        assert_eq!(&requests[1][0..4], &[0x19, 0x05, 0x0C, 0x01]);
        assert_eq!(u16::from_le_bytes([requests[1][8], requests[1][9]]), 0);
        let last = requests.last().unwrap();
        assert_eq!(u16::from_le_bytes([last[8], last[9]]), 239);
        assert_eq!(&last[16..], &image.bytes()[0xEF00..0xF000]);
    }

    #[test]
    fn prerequisites_fail_before_any_transport_access() {
        let image = ApplicationImage::from_raw(&valid_raw(0xA5)).unwrap();
        let recovery = ApplicationImage::from_raw(&valid_raw(0x5A)).unwrap();
        let backup = valid_backup();
        let version = FirmwareVersion::new("2.01.23").unwrap();
        let mut prerequisites = afik_prerequisites(&image, &recovery, &backup, &version);
        prerequisites.target_confirmation = "K5";
        let mut transport = ScriptedTransport::new(Vec::new(), 1);
        assert!(matches!(
            flash_application(&mut transport, prerequisites, |_| {}),
            Err(FlashError::TargetNotConfirmed)
        ));
        assert!(transport.writes.is_empty());

        let mut prerequisites = afik_prerequisites(&image, &recovery, &backup, &version);
        prerequisites.transaction_id = 0;
        assert!(matches!(
            flash_application(&mut transport, prerequisites, |_| {}),
            Err(FlashError::InvalidTransactionId)
        ));
        assert!(transport.writes.is_empty());

        let mut prerequisites = afik_prerequisites(&image, &recovery, &backup, &version);
        prerequisites.purpose = FlashPurpose::RecoveryRehearsal;
        assert!(matches!(
            flash_application(&mut transport, prerequisites, |_| {}),
            Err(FlashError::InvalidRecoveryRehearsal)
        ));
        assert!(transport.writes.is_empty());
    }

    #[test]
    fn missing_or_mismatched_ack_stops_without_retry() {
        let image = ApplicationImage::from_raw(&valid_raw(0xA5)).unwrap();
        let recovery = ApplicationImage::from_raw(&valid_raw(0x5A)).unwrap();
        let backup = valid_backup();
        let version = FirmwareVersion::new("2.01.23").unwrap();

        let mut missing = beacon();
        missing.extend(beacon());
        let mut transport = ScriptedTransport::new(missing, 1);
        assert!(matches!(
            flash_application(
                &mut transport,
                afik_prerequisites(&image, &recovery, &backup, &version),
                |_| {}
            ),
            Err(FlashError::Timeout)
        ));
        assert_eq!(decode_requests(&transport.writes).len(), 2);

        let mut rejected = beacon();
        rejected.extend(beacon());
        rejected.extend(acknowledgement(0, 3));
        let mut transport = ScriptedTransport::new(rejected, 1);
        assert!(matches!(
            flash_application(
                &mut transport,
                afik_prerequisites(&image, &recovery, &backup, &version),
                |_| {}
            ),
            Err(FlashError::PageRejected { page: 0, result: 3 })
        ));
        assert_eq!(decode_requests(&transport.writes).len(), 2);

        let mut mismatched = beacon();
        mismatched.extend(beacon());
        mismatched.extend(acknowledgement(1, 0));
        let mut transport = ScriptedTransport::new(mismatched, 1);
        assert!(matches!(
            flash_application(
                &mut transport,
                afik_prerequisites(&image, &recovery, &backup, &version),
                |_| {}
            ),
            Err(FlashError::PageAcknowledgementMismatch { expected_page: 0 })
        ));
        assert_eq!(decode_requests(&transport.writes).len(), 2);
    }

    #[test]
    fn v5_and_wildcard_versions_are_rejected() {
        assert!(FirmwareVersion::new("*.01.23").is_err());
        assert!(FirmwareVersion::new("5.00.05").is_err());
        let mut payload = vec![0_u8; 36];
        payload[0..4].copy_from_slice(&[0x7A, 0x05, 0x20, 0x00]);
        let mut transport = ScriptedTransport::new(encode_response(&payload), 1);
        assert!(matches!(
            probe_bootloader_v2(&mut transport),
            Err(FlashError::UnsupportedBootloader(0x057A))
        ));
    }

    #[test]
    fn detector_classifies_only_the_pinned_k1_and_k5_families() {
        let mut k5 = ScriptedTransport::new(beacon(), 1);
        assert!(matches!(
            detect_bootloader(&mut k5).unwrap(),
            BootloaderFamily::K5V1(ref info) if info.version() == "2.00.06"
        ));

        let mut k1_payload = vec![0_u8; 36];
        k1_payload[0..4].copy_from_slice(&[0x18, 0x05, 0x20, 0x00]);
        k1_payload[20..27].copy_from_slice(b"7.03.01");
        let mut k1 = ScriptedTransport::new(encode_response_with_trailer(&k1_payload, 0x6ED1), 1);
        assert!(matches!(
            detect_bootloader(&mut k1).unwrap(),
            BootloaderFamily::K1(ref info) if info.version() == "7.03.01"
        ));

        let mut unknown_payload = k1_payload;
        unknown_payload[20..27].copy_from_slice(b"7.04.01");
        let mut unknown = ScriptedTransport::new(encode_response(&unknown_payload), 1);
        assert!(matches!(
            detect_bootloader(&mut unknown),
            Err(FlashError::InvalidText("supported bootloader version"))
        ));
    }

    #[test]
    fn complete_eeprom_backup_checks_every_offset_and_length() {
        let mut hello = vec![0_u8; 40];
        hello[0..4].copy_from_slice(&[0x15, 0x05, 0x24, 0x00]);
        hello[4..15].copy_from_slice(b"k5_2.01.23\0");
        let mut responses = encode_response(&hello);
        let expected = (0_u8..=u8::MAX)
            .cycle()
            .take(EEPROM_BYTES)
            .collect::<Vec<_>>();
        for offset in (0..EEPROM_BYTES).step_by(0x80) {
            let mut payload = vec![0_u8; 0x88];
            payload[0..4].copy_from_slice(&[0x1C, 0x05, 0x84, 0x00]);
            payload[4..6].copy_from_slice(&u16::try_from(offset).unwrap().to_le_bytes());
            payload[6] = 0x80;
            payload[8..].copy_from_slice(&expected[offset..offset + 0x80]);
            responses.extend(encode_response(&payload));
        }
        let mut transport = ScriptedTransport::new(responses, 1);
        let (info, backup) = backup_eeprom(&mut transport).unwrap();
        assert_eq!(info.version(), "k5_2.01.23");
        assert_eq!(backup.bytes(), expected);
        assert_eq!(decode_requests(&transport.writes).len(), 65);
    }

    #[test]
    fn normal_probe_is_one_read_only_hello_exchange() {
        let mut hello = vec![0_u8; 40];
        hello[0..4].copy_from_slice(&[0x15, 0x05, 0x24, 0x00]);
        hello[4..16].copy_from_slice(b"AFIK-K1-0.1\0");
        let mut transport = ScriptedTransport::new(encode_response(&hello), 1);

        let info = probe_normal_firmware(&mut transport).unwrap();

        assert_eq!(info.version(), "AFIK-K1-0.1");
        let requests = decode_requests(&transport.writes);
        assert_eq!(
            requests,
            vec![vec![0x14, 0x05, 0x04, 0x00, 0x6A, 0x39, 0x57, 0x64]]
        );
    }

    #[test]
    fn keypad_probe_returns_only_bounded_raw_masks() {
        let response = [
            0x11, 0x7F, 0x0C, 0x00, 1, 0x10, 2, 0x20, 4, 0x40, 8, 0x80, 1, 1, 0, 0,
        ];
        let mut transport = ScriptedTransport::new(encode_response(&response), 1);

        let report = probe_keypad_matrix(&mut transport).unwrap();

        assert_eq!(
            report.gpio_b_idr_by_column(),
            [0x1001, 0x2002, 0x4004, 0x8008]
        );
        assert!(report.scan_valid());
        assert!(report.captured());
        assert_eq!(
            decode_requests(&transport.writes),
            vec![vec![0x10, 0x7F, 0x04, 0x00, 0x6A, 0x39, 0x57, 0x64]]
        );

        let invalid = [0x11, 0x7F, 0x0C, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0];
        let mut transport = ScriptedTransport::new(encode_response(&invalid), 1);
        assert!(matches!(
            probe_keypad_matrix(&mut transport),
            Err(FlashError::UnexpectedPacket { .. })
        ));
    }

    #[test]
    fn clock_probe_returns_only_bounded_raw_registers() {
        let response = [
            0x13, 0x7F, 20, 0, 0, 5, 0, 3, 0, 128, 0, 0, 18, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0,
        ];
        let mut transport = ScriptedTransport::new(encode_response(&response), 1);

        let report = probe_clock_snapshot(&mut transport).unwrap();

        assert_eq!(report.registers(), [0x0300_0500, 0x0000_8000, 0x12, 0]);
        assert!(report.contract_valid());
        assert_eq!(
            decode_requests(&transport.writes),
            vec![vec![0x12, 0x7F, 0x04, 0x00, 0x6A, 0x39, 0x57, 0x64]]
        );

        let mut invalid = response;
        invalid[20] = 2;
        let mut transport = ScriptedTransport::new(encode_response(&invalid), 1);
        assert!(matches!(
            probe_clock_snapshot(&mut transport),
            Err(FlashError::UnexpectedPacket { .. })
        ));
    }

    #[test]
    fn individual_clock_register_probe_preserves_order_and_identity() {
        let mut responses = Vec::new();
        for (index, command) in [0x7F15_u16, 0x7F17, 0x7F19, 0x7F1B].into_iter().enumerate() {
            let mut payload = [0_u8; 12];
            payload[0..2].copy_from_slice(&command.to_le_bytes());
            payload[2..4].copy_from_slice(&8_u16.to_le_bytes());
            payload[4] = u8::try_from(index).unwrap();
            let value = 0x1000_u32 + u32::try_from(index).unwrap();
            payload[8..12].copy_from_slice(&value.to_le_bytes());
            responses.extend(encode_response(&payload));
        }
        let mut transport = ScriptedTransport::new(responses, 1);

        assert_eq!(
            probe_clock_registers(&mut transport).unwrap(),
            [0x1000, 0x1001, 0x1002, 0x1003]
        );
        assert_eq!(
            decode_requests(&transport.writes)
                .iter()
                .map(|request| u16::from_le_bytes([request[0], request[1]]))
                .collect::<Vec<_>>(),
            [0x7F14, 0x7F16, 0x7F18, 0x7F1A]
        );
    }

    #[test]
    fn clock_control_proves_the_path_without_register_data() {
        let response = [0x1D, 0x7F, 8, 0, 0xA5, 0, 0, 0, 0x4C, 0x43, 0x31, 0x4B];
        let mut transport = ScriptedTransport::new(encode_response(&response), 1);
        assert_eq!(probe_clock_control(&mut transport).unwrap(), 0x4B31_434C);
        assert_eq!(
            decode_requests(&transport.writes),
            vec![vec![0x1C, 0x7F, 0x04, 0x00, 0x6A, 0x39, 0x57, 0x64]]
        );
    }

    fn decode_requests(frames: &[u8]) -> Vec<Vec<u8>> {
        const KEY: [u8; 16] = [
            0x16, 0x6C, 0x14, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5, 0x40, 0x13, 0x03,
            0xE9, 0x80,
        ];
        let mut offset = 0;
        let mut packets = Vec::new();
        while offset < frames.len() {
            assert_eq!(&frames[offset..offset + 2], &[0xAB, 0xCD]);
            let length = usize::from(u16::from_le_bytes([frames[offset + 2], frames[offset + 3]]));
            let end = offset + length + 8;
            assert_eq!(&frames[end - 2..end], &[0xDC, 0xBA]);
            let mut decoded = frames[offset + 4..offset + 6 + length].to_vec();
            for (index, byte) in decoded.iter_mut().enumerate() {
                *byte ^= KEY[index % KEY.len()];
            }
            let crc = u16::from_le_bytes([decoded[length], decoded[length + 1]]);
            assert_eq!(crc, crc16_xmodem(&decoded[..length]));
            decoded.truncate(length);
            packets.push(decoded);
            offset = end;
        }
        packets
    }
}
