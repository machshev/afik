use std::{error::Error, fmt, io, io::Read, io::Write};

use crate::{
    codec::{
        is_device_marker, receive_packet, receive_packet_with_response_crc, send_packet, Packet,
    },
    image::{sha256, ApplicationImage, EepromBackup, ImageError, EEPROM_BYTES, FLASH_PAGE_BYTES},
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
const COMMAND_WRITE_EEPROM_REQUEST: u16 = 0x051D;
const COMMAND_WRITE_EEPROM_RESPONSE: u16 = 0x051E;
const COMMAND_V2_BEACON: u16 = 0x0518;
const COMMAND_V2_PAGE_REQUEST: u16 = 0x0519;
const COMMAND_V2_PAGE_RESPONSE: u16 = 0x051A;
const COMMAND_V2_VERSION_REQUEST: u16 = 0x0530;
const COMMAND_V5_BEACON: u16 = 0x057A;
const EEPROM_BLOCK_BYTES: usize = 0x80;
const EEPROM_WRITE_BYTES: usize = 8;
const EEPROM_WRITE_BYTES_U16: u16 = 8;
const EEPROM_WRITE_BLOCKS: u16 = 1024;
const MAX_BEACONS_BEFORE_ACK: usize = 4;
/// Exact raw source length of the separately retained qualified recovery image.
pub const K5_RECOVERY_SOURCE_BYTES: usize = 60_612;

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
    /// A radio response carried neither observed marker: the obfuscated `0xFFFF`
    /// normal firmware sends nor the literal one a bootloader sends.
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
    /// A K5 common-application response failed its bounded payload contract.
    InvalidK5Response(&'static str),
    /// A K5 state-changing request was answered by a receiver-fault view.
    K5StateChangeRejected,
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
    /// The default application flash accepts only an image naming itself AFIK.
    NotAfikImage,
    /// A normal-firmware request was answered by the bootloader's beacon.
    InBootloader,
    /// The running firmware serves no EEPROM reads; it carries this version.
    EepromNotServed(String),
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
    /// The current EEPROM image did not match the separately supplied backup.
    EepromCurrentMismatch,
    /// The live normal-firmware hello did not match the explicitly selected version.
    EepromFirmwareMismatch,
    /// The recovery image CRC or source-length confirmation was not exact.
    EepromRecoveryMismatch,
    /// A restore plan was forged or became internally inconsistent.
    InvalidEepromRestorePlan,
    /// The two immediate reads before a block write disagreed.
    EepromBlockChangedDuringPreflight {
        /// Block offset observed changing between the two reads.
        offset: u16,
    },
    /// The restore confirmation did not bind the qualified target and image.
    EepromRestoreConfirmationMismatch,
    /// A write response did not echo the exact requested EEPROM offset.
    EepromWriteAcknowledgementMismatch {
        /// Offset sent by the host.
        expected_offset: u16,
        /// Offset echoed by the radio.
        actual_offset: u16,
    },
    /// A changed EEPROM block failed its immediate read-back.
    EepromBlockVerificationFailed {
        /// Block offset that failed verification.
        offset: u16,
    },
    /// The complete restore read-back did not equal the requested image.
    EepromRestoreVerificationFailed,
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
    #[allow(clippy::too_many_lines)] // One arm per error variant.
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
            Self::InvalidK5Response(field) => {
                write!(formatter, "invalid K5 common response: {field}")
            }
            Self::K5StateChangeRejected => {
                formatter.write_str("K5 state-changing request rejected: receiver fault")
            }
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
            Self::NotAfikImage => formatter.write_str("image carries no AFIK-K5- identity"),
            Self::InBootloader => formatter.write_str(
                "the radio is in bootloader (flashing) mode; switch it on normally, \
                 without holding PTT, to talk to its firmware",
            ),
            Self::EepromNotServed(version) => write!(
                formatter,
                "the radio runs {version}, which serves no EEPROM reads; stock firmware \
                 and AFIK-K5 7.0.6 or later do"
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
            Self::EepromCurrentMismatch => {
                formatter.write_str("live EEPROM does not match the expected current backup")
            }
            Self::EepromFirmwareMismatch => {
                formatter.write_str("live normal firmware does not match the expected version")
            }
            Self::EepromRecoveryMismatch => {
                formatter.write_str("recovery image identity confirmation is not exact")
            }
            Self::InvalidEepromRestorePlan => {
                formatter.write_str("EEPROM restore plan is inconsistent or out of bounds")
            }
            Self::EepromBlockChangedDuringPreflight { offset } => write!(
                formatter,
                "EEPROM block changed during immediate preflight reads at offset {offset:04x}"
            ),
            Self::EepromRestoreConfirmationMismatch => formatter.write_str(
                "EEPROM restore confirmation must be TARGET:CRC32 for the selected image",
            ),
            Self::EepromWriteAcknowledgementMismatch {
                expected_offset,
                actual_offset,
            } => write!(
                formatter,
                "EEPROM write response mismatch: expected offset {expected_offset:04x}, got {actual_offset:04x}"
            ),
            Self::EepromBlockVerificationFailed { offset } => write!(
                formatter,
                "EEPROM block read-back failed at offset {offset:04x}"
            ),
            Self::EepromRestoreVerificationFailed => {
                formatter.write_str("complete EEPROM restore read-back failed")
            }
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

    /// Accepts the bounded printable identity returned by normal-mode hello.
    /// This is deliberately separate from the bootloader's version-2 grammar.
    pub fn normal(version: &str) -> Result<Self, FlashError> {
        let raw = version.as_bytes();
        if raw.is_empty()
            || raw.len() > 16
            || raw.iter().any(|byte| !byte.is_ascii() || *byte < 0x20)
        {
            return Err(FlashError::InvalidText("normal-firmware version"));
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

/// Inputs which must be independently confirmed before a K5 EEPROM restore.
#[derive(Clone, Copy, Debug)]
pub struct EepromRestorePrerequisites<'a> {
    /// Complete image to write.
    pub restore_image: &'a EepromBackup,
    /// Complete image expected to be on the radio before any write.
    pub expected_current: &'a EepromBackup,
    /// Separately retained, vector-checked recovery application for the unit.
    pub recovery_image: &'a ApplicationImage,
    /// Explicit source length of the independently checked recovery artifact.
    pub recovery_source_len: usize,
    /// CRC-32 confirmation for the padded recovery image.
    pub recovery_crc32: u32,
    /// Trusted SHA-256 of the raw recovery artifact, supplied out-of-band.
    pub recovery_sha256: [u8; 32],
    /// Exact version expected from the live normal-firmware hello.
    pub expected_firmware: &'a FirmwareVersion,
    /// Exact qualified K5 target phrase.
    pub target_confirmation: &'a str,
    /// Exact `TARGET:CRC32` confirmation for the selected restore image.
    pub restore_confirmation: &'a str,
}

/// Preflight result. No EEPROM write is sent while constructing this value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EepromRestorePlan {
    /// Firmware identity returned by the preflight hello.
    firmware: NormalFirmwareInfo,
    /// Complete live image captured and compared before writes.
    live_backup: EepromBackup,
    /// Complete validated image selected for restore.
    restore_image: EepromBackup,
    /// Changed eight-byte offsets in ascending order.
    changed_offsets: Vec<u16>,
}

impl EepromRestorePlan {
    /// Firmware identity returned by the preflight hello.
    pub fn firmware(&self) -> &NormalFirmwareInfo {
        &self.firmware
    }
    /// Complete live image captured before writes.
    pub fn live_backup(&self) -> &EepromBackup {
        &self.live_backup
    }
    /// Complete selected restore image.
    pub fn restore_image(&self) -> &EepromBackup {
        &self.restore_image
    }
    /// Changed eight-byte offsets in ascending order.
    pub fn changed_offsets(&self) -> &[u16] {
        &self.changed_offsets
    }
}

/// Result after a restore has either verified no changes or read back all writes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EepromRestoreReport {
    /// Firmware identity returned by the preflight hello.
    pub firmware: NormalFirmwareInfo,
    /// Number of changed eight-byte blocks applied.
    pub changed_blocks: u16,
    /// Complete image read-back matched the selected restore image.
    pub verified_read_back: bool,
}

/// Reads and validates the complete 8 KiB EEPROM without exposing any write.
pub fn backup_eeprom<T: Read + Write>(
    transport: &mut T,
) -> Result<(NormalFirmwareInfo, EepromBackup), FlashError> {
    let hello = hello_request();
    send_packet(transport, &hello)?;
    let info = parse_hello_response(&receive_packet(transport)?)?;
    require_eeprom_service(&info)?;

    let backup = read_eeprom_image(transport)?;
    Ok((info, backup))
}

/// Performs the hello, complete live backup, and changed-block plan without writing.
pub fn prepare_eeprom_restore<T: Read + Write>(
    transport: &mut T,
    prerequisites: EepromRestorePrerequisites<'_>,
) -> Result<EepromRestorePlan, FlashError> {
    validate_restore_prerequisites(&prerequisites)?;
    let hello = hello_request();
    send_packet(transport, &hello)?;
    let firmware = parse_hello_response(&receive_packet(transport)?)?;
    require_eeprom_service(&firmware)?;
    if firmware.version() != prerequisites.expected_firmware.as_str() {
        return Err(FlashError::EepromFirmwareMismatch);
    }
    let live_backup = read_eeprom_image(transport)?;
    if live_backup.bytes() != prerequisites.expected_current.bytes() {
        return Err(FlashError::EepromCurrentMismatch);
    }
    let mut changed_offsets = Vec::new();
    for block in 0_u16..EEPROM_WRITE_BLOCKS {
        let offset = usize::from(block) * EEPROM_WRITE_BYTES;
        if live_backup.bytes()[offset..offset + EEPROM_WRITE_BYTES]
            != prerequisites.restore_image.bytes()[offset..offset + EEPROM_WRITE_BYTES]
        {
            changed_offsets.push(block * EEPROM_WRITE_BYTES_U16);
        }
    }
    Ok(EepromRestorePlan {
        firmware,
        live_backup,
        restore_image: prerequisites.restore_image.clone(),
        changed_offsets,
    })
}

/// Applies a previously prepared plan. The caller must preserve `live_backup`
/// before calling this function; no retry is performed after an ambiguous write.
pub fn apply_eeprom_restore<T: Read + Write>(
    transport: &mut T,
    plan: &EepromRestorePlan,
) -> Result<EepromRestoreReport, FlashError> {
    validate_restore_plan(plan)?;
    if plan.changed_offsets.is_empty() {
        return Ok(EepromRestoreReport {
            firmware: plan.firmware.clone(),
            changed_blocks: 0,
            verified_read_back: true,
        });
    }
    for &offset in &plan.changed_offsets {
        let mut current = [0_u8; EEPROM_WRITE_BYTES];
        read_eeprom_bytes(transport, offset, &mut current)?;
        let mut stable = [0_u8; EEPROM_WRITE_BYTES];
        read_eeprom_bytes(transport, offset, &mut stable)?;
        if current != stable {
            return Err(FlashError::EepromBlockChangedDuringPreflight { offset });
        }
        let start = usize::from(offset);
        let expected = &plan.live_backup.bytes()[start..start + EEPROM_WRITE_BYTES];
        let replacement = &plan.restore_image.bytes()[start..start + EEPROM_WRITE_BYTES];
        if current != expected {
            return Err(FlashError::EepromCurrentMismatch);
        }
        let mut replacement_block = [0_u8; EEPROM_WRITE_BYTES];
        replacement_block.copy_from_slice(replacement);
        let request = eeprom_write_request(offset, replacement_block);
        send_packet(transport, &request)?;
        let response = receive_packet(transport)?;
        parse_eeprom_write_response(&response, offset)?;
        let mut verified = [0_u8; EEPROM_WRITE_BYTES];
        read_eeprom_bytes(transport, offset, &mut verified)?;
        if verified != replacement {
            return Err(FlashError::EepromBlockVerificationFailed { offset });
        }
    }
    let final_image = read_eeprom_image(transport)?;
    if final_image.bytes() != plan.restore_image.bytes() {
        return Err(FlashError::EepromRestoreVerificationFailed);
    }
    let changed_blocks = u16::try_from(plan.changed_offsets.len())
        .map_err(|_| FlashError::PacketTooLarge(plan.changed_offsets.len()))?;
    Ok(EepromRestoreReport {
        firmware: plan.firmware.clone(),
        changed_blocks,
        verified_read_back: true,
    })
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

/// Protocol shape announced by a beacon command, per `EVID-K5-015`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BeaconProtocol {
    /// `0x0518`: the page-index protocol the K1 and qualified K5 paths implement.
    PageWrite,
    /// `0x057A`: the bootloader K5TOOL records as using AES internally.
    AesEncrypted,
    /// A command no evidence covers.
    Unrecognised,
}

impl BeaconProtocol {
    /// Returns the stable printable label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::PageWrite => "page-write",
            Self::AesEncrypted => "aes-encrypted",
            Self::Unrecognised => "unrecognised",
        }
    }
}

/// One bootloader beacon as observed, carrying no hardware claim.
///
/// `ADR-070` makes the beacon evidence rather than a classifier: the command
/// says which protocol the bootloader speaks, and the version describes its
/// build. Neither identifies the radio, so an unfamiliar version is recorded
/// rather than refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BeaconObservation {
    command: u16,
    version: String,
}

impl BeaconObservation {
    /// Returns the exact beacon command.
    pub const fn command(&self) -> u16 {
        self.command
    }

    /// Returns the protocol shape the command announces.
    pub const fn protocol(&self) -> BeaconProtocol {
        match self.command {
            COMMAND_V2_BEACON => BeaconProtocol::PageWrite,
            COMMAND_V5_BEACON => BeaconProtocol::AesEncrypted,
            _ => BeaconProtocol::Unrecognised,
        }
    }

    /// Returns the printable bootloader version exactly as reported.
    pub fn version(&self) -> &str {
        &self.version
    }
}

/// Reads one beacon and reports it without judging what the radio is.
///
/// This is the read-only observation `ADR-070` requires. It validates the
/// envelope, the marker and the beacon layout, and it accepts any printable
/// version, including one no evidence covers. It selects no workflow and
/// authorises nothing.
pub fn observe_bootloader<T: Read>(transport: &mut T) -> Result<BeaconObservation, FlashError> {
    let (packet, response_crc) = receive_packet_with_response_crc(transport)?;
    let payload = packet.as_slice();
    if !is_device_marker(response_crc, payload.len()) {
        return Err(FlashError::InvalidResponseCrc(response_crc));
    }
    let command = packet_command(payload).unwrap_or(0);
    require_packet(payload, command, 32, 36, "bootloader beacon")?;
    let version = parse_text(&payload[20..36], "bootloader version")?;
    Ok(BeaconObservation { command, version })
}

/// Packets the identification listens to before giving up on a beacon.
const IDENTIFY_LISTEN_ATTEMPTS: usize = 3;

/// What answered on a serial port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RadioPresence {
    /// A radio in bootloader (flashing) mode, announcing itself by beacon.
    Bootloader(BootloaderFamily),
    /// A radio running its normal firmware, which answered the hello.
    Firmware(NormalFirmwareInfo),
}

/// Finds out what is on the other end of the cable, writing nothing to it.
///
/// It first listens for a bootloader beacon, which needs no request at all, so
/// a radio in flashing mode is never sent anything. Only when no beacon arrives
/// does it send the read-only normal-firmware hello.
pub fn identify_radio<T: Read + Write>(transport: &mut T) -> Result<RadioPresence, FlashError> {
    // A port opened partway through a beacon reads the tail of one packet
    // first. Bytes are arriving, so listen for the next packet rather than
    // reporting that nothing answered; silence goes straight to the hello.
    let mut silent = FlashError::Timeout;
    for _ in 0..IDENTIFY_LISTEN_ATTEMPTS {
        match detect_bootloader(transport) {
            Ok(family) => return Ok(RadioPresence::Bootloader(family)),
            Err(FlashError::Timeout) => break,
            Err(error) => silent = error,
        }
    }
    match probe_normal_firmware(transport) {
        Ok(info) => Ok(RadioPresence::Firmware(info)),
        // A beacon arriving now is a bootloader the listen missed or could not
        // classify; its own error says more than the hello's.
        Err(FlashError::InBootloader) => Err(silent),
        Err(error) => Err(error),
    }
}

/// Reports whether firmware with this hello version serves the `0x051B` EEPROM
/// read, which is what an EEPROM backup needs.
pub fn serves_eeprom_reads(version: &str) -> bool {
    require_eeprom_service(&NormalFirmwareInfo {
        version: version.to_owned(),
    })
    .is_ok()
}

/// Reads one beacon and classifies only the pinned K1 or qualified K5 protocol.
pub fn detect_bootloader<T: Read>(transport: &mut T) -> Result<BootloaderFamily, FlashError> {
    let (packet, response_crc) = receive_packet_with_response_crc(transport)?;
    if !is_device_marker(response_crc, packet.as_slice().len()) {
        return Err(FlashError::InvalidResponseCrc(response_crc));
    }
    parse_bootloader_family(&packet)
}

/// Writes all 240 application pages after validating every prerequisite.
///
/// The callback runs after each exact acknowledgement. Missing or ambiguous
/// acknowledgements stop immediately and are never retried.
pub fn flash_application<T, F>(
    transport: &mut T,
    prerequisites: FlashPrerequisites<'_>,
    page_acknowledged: F,
) -> Result<FlashReport, FlashError>
where
    T: Read + Write,
    F: FnMut(u16),
{
    validate_prerequisites(prerequisites)?;
    write_application(
        transport,
        prerequisites.image,
        prerequisites.version,
        prerequisites.transaction_id,
        page_acknowledged,
    )
}

/// Bootloader negotiation version sent by the default K5 AFIK flash.
pub const DEFAULT_NEGOTIATION_VERSION: &str = "2.01.23";

/// Identity every AFIK K5 application image carries in its version string.
const AFIK_K5_IDENTITY: &[u8] = b"AFIK-K5-";

/// Writes an AFIK application without recovery or EEPROM prerequisites.
///
/// This is the default K5 path for AFIK's own images. Like
/// [`flash_application`] it writes only the 240 application pages, never the
/// bootloader or EEPROM, so an image that does not boot is recovered by
/// flashing again through the same bootloader. The image must carry an
/// `AFIK-K5-` identity, which guards against selecting a different file.
pub fn flash_afik_application<T, F>(
    transport: &mut T,
    image: &ApplicationImage,
    version: &FirmwareVersion,
    transaction_id: u32,
    page_acknowledged: F,
) -> Result<FlashReport, FlashError>
where
    T: Read + Write,
    F: FnMut(u16),
{
    if transaction_id == 0 {
        return Err(FlashError::InvalidTransactionId);
    }
    if !image
        .bytes()
        .windows(AFIK_K5_IDENTITY.len())
        .any(|window| window == AFIK_K5_IDENTITY)
    {
        return Err(FlashError::NotAfikImage);
    }
    write_application(transport, image, version, transaction_id, page_acknowledged)
}

fn write_application<T, F>(
    transport: &mut T,
    image: &ApplicationImage,
    version: &FirmwareVersion,
    transaction_id: u32,
    mut page_acknowledged: F,
) -> Result<FlashReport, FlashError>
where
    T: Read + Write,
    F: FnMut(u16),
{
    let bootloader = probe_bootloader_v2(transport)?;

    let version_request = version_request(version);
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
            transaction_id,
            page_number,
            &image.bytes()[start..start + FLASH_PAGE_BYTES],
        );
        send_packet(transport, &request)?;
        receive_page_acknowledgement(transport, transaction_id, page_number)?;
        page_acknowledged(page_number);
    }

    Ok(FlashReport {
        bootloader,
        pages_acknowledged: 240,
        image_crc32: image.crc32(),
        transaction_id,
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

/// Identity prefix every AFIK application reports in its hello.
const AFIK_IDENTITY_PREFIX: &str = "AFIK-";
/// Identity prefix of the AFIK K5 application.
const AFIK_K5_IDENTITY_PREFIX: &str = "AFIK-K5-";
/// First AFIK K5 version which serves the stock `0x051B` EEPROM read.
const AFIK_K5_EEPROM_SINCE: [u32; 3] = [7, 0, 6];

/// Refuses before an EEPROM read that the running firmware would never answer.
///
/// Stock firmware serves `0x051B`, and so does AFIK K5 from
/// [`AFIK_K5_EEPROM_SINCE`]. Earlier AFIK K5 images and the K1 answer the hello
/// but have no read handler, so a read would only time out.
fn require_eeprom_service(firmware: &NormalFirmwareInfo) -> Result<(), FlashError> {
    let version = firmware.version();
    let Some(afik) = version.strip_prefix(AFIK_IDENTITY_PREFIX) else {
        return Ok(());
    };
    let serves = afik
        .strip_prefix(&AFIK_K5_IDENTITY_PREFIX[AFIK_IDENTITY_PREFIX.len()..])
        .and_then(release)
        .is_some_and(|release| release >= AFIK_K5_EEPROM_SINCE);
    if serves {
        Ok(())
    } else {
        Err(FlashError::EepromNotServed(version.to_owned()))
    }
}

/// Parses a plain `major.minor.patch` release; anything else is `None`.
fn release(text: &str) -> Option<[u32; 3]> {
    let mut parts = text.split('.').map(|part| part.parse::<u32>().ok());
    let release = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some(release)
}

fn parse_hello_response(packet: &Packet) -> Result<NormalFirmwareInfo, FlashError> {
    let payload = packet.as_slice();
    // A K5 bootloader announces itself unprompted, so the first packet after a
    // hello is its beacon when the radio was powered on into flashing mode.
    if packet_command(payload) == Some(COMMAND_V2_BEACON) {
        return Err(FlashError::InBootloader);
    }
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

fn validate_restore_prerequisites(
    prerequisites: &EepromRestorePrerequisites<'_>,
) -> Result<(), FlashError> {
    if prerequisites.target_confirmation != QUALIFIED_TARGET_CONFIRMATION {
        return Err(FlashError::TargetNotConfirmed);
    }
    let Some((target, digest)) = prerequisites.restore_confirmation.split_once(':') else {
        return Err(FlashError::EepromRestoreConfirmationMismatch);
    };
    if target != QUALIFIED_TARGET_CONFIRMATION
        || digest.len() != 8
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        || u32::from_str_radix(digest, 16).ok() != Some(prerequisites.restore_image.crc32())
    {
        return Err(FlashError::EepromRestoreConfirmationMismatch);
    }
    // ApplicationImage::from_raw has already performed the existing vector and
    // bounded-image checks. Requiring a distinct, validated recovery value here
    // prevents a missing recovery artifact from being papered over by the host.
    if prerequisites.recovery_source_len != K5_RECOVERY_SOURCE_BYTES
        || prerequisites.recovery_source_len > crate::image::APPLICATION_BYTES
        || prerequisites.recovery_source_len != prerequisites.recovery_image.source_len()
        || prerequisites.recovery_crc32 != prerequisites.recovery_image.crc32()
        || sha256(prerequisites.recovery_image.source_bytes()) != prerequisites.recovery_sha256
    {
        return Err(FlashError::EepromRecoveryMismatch);
    }
    Ok(())
}

fn validate_restore_plan(plan: &EepromRestorePlan) -> Result<(), FlashError> {
    if plan.live_backup.bytes().len() != EEPROM_BYTES
        || plan.restore_image.bytes().len() != EEPROM_BYTES
    {
        return Err(FlashError::InvalidEepromRestorePlan);
    }
    let mut expected = Vec::new();
    for block in 0_u16..EEPROM_WRITE_BLOCKS {
        let start = usize::from(block) * EEPROM_WRITE_BYTES;
        if plan.live_backup.bytes()[start..start + EEPROM_WRITE_BYTES]
            != plan.restore_image.bytes()[start..start + EEPROM_WRITE_BYTES]
        {
            expected.push(block * EEPROM_WRITE_BYTES_U16);
        }
    }
    if plan.changed_offsets != expected
        || plan
            .changed_offsets
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || plan.changed_offsets.iter().any(|offset| {
            usize::from(*offset) % EEPROM_WRITE_BYTES != 0
                || usize::from(*offset)
                    .checked_add(EEPROM_WRITE_BYTES)
                    .is_none_or(|end| end > EEPROM_BYTES)
        })
    {
        return Err(FlashError::InvalidEepromRestorePlan);
    }
    Ok(())
}

fn read_eeprom_image<T: Read + Write>(transport: &mut T) -> Result<EepromBackup, FlashError> {
    let mut bytes = vec![0_u8; EEPROM_BYTES];
    for block in 0_u16..64 {
        let offset_u16 = block * u16::try_from(EEPROM_BLOCK_BYTES).expect("bounded block");
        let offset = usize::from(offset_u16);
        read_eeprom_bytes(
            transport,
            offset_u16,
            &mut bytes[offset..offset + EEPROM_BLOCK_BYTES],
        )?;
    }
    EepromBackup::from_raw(&bytes).map_err(FlashError::InvalidEepromBackup)
}

fn read_eeprom_bytes<T: Read + Write>(
    transport: &mut T,
    offset: u16,
    output: &mut [u8],
) -> Result<(), FlashError> {
    if output.is_empty() || output.len() > EEPROM_BLOCK_BYTES {
        return Err(FlashError::PacketTooLarge(output.len()));
    }
    let request = eeprom_read_request(offset, output.len());
    send_packet(transport, &request)?;
    let packet = receive_packet(transport)?;
    parse_eeprom_response(&packet, offset, output)
}

fn eeprom_read_request(offset: u16, length: usize) -> [u8; 12] {
    let mut payload = [0_u8; 12];
    payload[0..2].copy_from_slice(&COMMAND_READ_EEPROM_REQUEST.to_le_bytes());
    payload[2..4].copy_from_slice(&8_u16.to_le_bytes());
    payload[4..6].copy_from_slice(&offset.to_le_bytes());
    payload[6] = u8::try_from(length).expect("bounded EEPROM block");
    payload[8..12].copy_from_slice(&SESSION_WORD.to_le_bytes());
    payload
}

fn eeprom_write_request(offset: u16, data: [u8; EEPROM_WRITE_BYTES]) -> [u8; 20] {
    let mut payload = [0_u8; 20];
    payload[0..2].copy_from_slice(&COMMAND_WRITE_EEPROM_REQUEST.to_le_bytes());
    payload[2..4].copy_from_slice(&16_u16.to_le_bytes());
    payload[4..6].copy_from_slice(&offset.to_le_bytes());
    payload[6] = u8::try_from(EEPROM_WRITE_BYTES).expect("bounded EEPROM write");
    payload[8..12].copy_from_slice(&SESSION_WORD.to_le_bytes());
    payload[12..20].copy_from_slice(&data);
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

fn parse_eeprom_write_response(packet: &Packet, expected_offset: u16) -> Result<(), FlashError> {
    let payload = packet.as_slice();
    require_packet(payload, COMMAND_WRITE_EEPROM_RESPONSE, 2, 6, "EEPROM write")?;
    let actual_offset = u16::from_le_bytes([payload[4], payload[5]]);
    if actual_offset != expected_offset {
        return Err(FlashError::EepromWriteAcknowledgementMismatch {
            expected_offset,
            actual_offset,
        });
    }
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
        apply_eeprom_restore, backup_eeprom, detect_bootloader, flash_afik_application,
        flash_application, identify_radio, observe_bootloader, prepare_eeprom_restore,
        probe_bootloader_v2, probe_clock_control, probe_clock_registers, probe_clock_snapshot,
        probe_keypad_matrix, probe_normal_firmware, require_eeprom_service, serves_eeprom_reads,
        BeaconProtocol, BootloaderFamily, EepromRestorePrerequisites, FirmwareVersion, FlashError,
        FlashPrerequisites, FlashPurpose, NormalFirmwareInfo, RadioPresence, EEPROM_BLOCK_BYTES,
        QUALIFIED_TARGET_CONFIRMATION, RECOVERY_REHEARSED_CONFIRMATION,
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

    fn recovery_image() -> ApplicationImage {
        let mut bytes = vec![0x5A; super::K5_RECOVERY_SOURCE_BYTES];
        bytes[0..4].copy_from_slice(&0x2000_4000_u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&9_u32.to_le_bytes());
        ApplicationImage::from_raw(&bytes).unwrap()
    }

    fn valid_backup() -> EepromBackup {
        let bytes = (0_u8..=u8::MAX)
            .cycle()
            .take(EEPROM_BYTES)
            .collect::<Vec<_>>();
        EepromBackup::from_raw(&bytes).unwrap()
    }

    fn hello_response() -> Vec<u8> {
        let mut hello = vec![0_u8; 40];
        hello[0..4].copy_from_slice(&[0x15, 0x05, 0x24, 0x00]);
        hello[4..20].copy_from_slice(b"F4HWN v4.3\0\0\0\0\0\0");
        encode_response(&hello)
    }

    fn eeprom_response(offset: usize, data: &[u8]) -> Vec<u8> {
        let mut payload = vec![0_u8; data.len() + 8];
        payload[0..4].copy_from_slice(&[
            0x1C,
            0x05,
            u8::try_from(data.len() + 4).unwrap(),
            u8::try_from((data.len() + 4) >> 8).unwrap(),
        ]);
        payload[4..6].copy_from_slice(&u16::try_from(offset).unwrap().to_le_bytes());
        payload[6] = u8::try_from(data.len()).unwrap();
        payload[8..].copy_from_slice(data);
        encode_response(&payload)
    }

    fn eeprom_write_ack(offset: usize) -> Vec<u8> {
        let mut payload = [0_u8; 6];
        payload[0..4].copy_from_slice(&[0x1E, 0x05, 0x02, 0x00]);
        payload[4..6].copy_from_slice(&u16::try_from(offset).unwrap().to_le_bytes());
        encode_response(&payload)
    }

    fn restore_responses(live: &[u8], replacement: &[u8], changed: &[usize]) -> Vec<u8> {
        let mut responses = hello_response();
        for offset in (0..EEPROM_BYTES).step_by(EEPROM_BLOCK_BYTES) {
            responses.extend(eeprom_response(
                offset,
                &live[offset..offset + EEPROM_BLOCK_BYTES],
            ));
        }
        for &offset in changed {
            responses.extend(eeprom_response(offset, &live[offset..offset + 8]));
            responses.extend(eeprom_response(offset, &live[offset..offset + 8]));
            responses.extend(eeprom_write_ack(offset));
            responses.extend(eeprom_response(offset, &replacement[offset..offset + 8]));
        }
        for offset in (0..EEPROM_BYTES).step_by(EEPROM_BLOCK_BYTES) {
            responses.extend(eeprom_response(
                offset,
                &replacement[offset..offset + EEPROM_BLOCK_BYTES],
            ));
        }
        responses
    }

    fn restore_prerequisites<'a>(
        restore: &'a EepromBackup,
        current: &'a EepromBackup,
        recovery: &'a ApplicationImage,
        confirmation: &'a str,
    ) -> EepromRestorePrerequisites<'a> {
        EepromRestorePrerequisites {
            restore_image: restore,
            expected_current: current,
            recovery_image: recovery,
            recovery_source_len: recovery.source_len(),
            recovery_crc32: recovery.crc32(),
            recovery_sha256: super::sha256(recovery.source_bytes()),
            expected_firmware: Box::leak(Box::new(
                FirmwareVersion::normal("F4HWN v4.3").expect("test version"),
            )),
            target_confirmation: QUALIFIED_TARGET_CONFIRMATION,
            restore_confirmation: confirmation,
        }
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
        let recovery = recovery_image();
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
    fn default_afik_flash_needs_no_recovery_but_refuses_non_afik_images() {
        let mut raw = valid_raw(0xA5);
        raw.extend_from_slice(b"AFIK-K5-2.3T");
        let image = ApplicationImage::from_raw(&raw).unwrap();
        let version = FirmwareVersion::new("2.01.23").unwrap();
        let mut responses = beacon();
        responses.extend(beacon());
        for page in 0..FLASH_PAGE_COUNT {
            responses.extend(acknowledgement(u16::try_from(page).unwrap(), 0));
        }
        let mut transport = ScriptedTransport::new(responses, 1);
        let report = flash_afik_application(
            &mut transport,
            &image,
            &version,
            TEST_TRANSACTION_ID,
            |_| {},
        )
        .unwrap();
        assert_eq!(report.pages_acknowledged, 240);
        assert_eq!(decode_requests(&transport.writes).len(), 241);

        let other = ApplicationImage::from_raw(&valid_raw(0xA5)).unwrap();
        let mut transport = ScriptedTransport::new(Vec::new(), 1);
        assert!(matches!(
            flash_afik_application(
                &mut transport,
                &other,
                &version,
                TEST_TRANSACTION_ID,
                |_| {}
            ),
            Err(FlashError::NotAfikImage)
        ));
        assert!(matches!(
            flash_afik_application(&mut transport, &image, &version, 0, |_| {}),
            Err(FlashError::InvalidTransactionId)
        ));
        assert!(transport.writes.is_empty());
    }

    #[test]
    fn prerequisites_fail_before_any_transport_access() {
        let image = ApplicationImage::from_raw(&valid_raw(0xA5)).unwrap();
        let recovery = recovery_image();
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
        let recovery = recovery_image();
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
    fn observation_reports_an_unfamiliar_version_instead_of_refusing_it() {
        // The UV-5R Plus of `EVID-K5-012`. `detect_bootloader` refuses it,
        // which is `RISK-037`'s wrong discriminator; observation must not.
        let mut payload = vec![0_u8; 36];
        payload[0..4].copy_from_slice(&[0x18, 0x05, 0x20, 0x00]);
        payload[20..27].copy_from_slice(b"4.00.01");
        let frame = encode_response_with_trailer(&payload, 0x6ED1);

        let beacon = observe_bootloader(&mut ScriptedTransport::new(frame, 1)).unwrap();
        assert_eq!(beacon.version(), "4.00.01");
        assert_eq!(beacon.command(), 0x0518);
        assert_eq!(beacon.protocol(), BeaconProtocol::PageWrite);
        assert_eq!(beacon.protocol().label(), "page-write");
    }

    #[test]
    fn observation_separates_the_aes_bootloader_by_command_not_version() {
        // `EVID-K5-015`: the command announces the protocol. A `5.*` version on
        // `0x057A` is the AES path, and it stays distinguishable without any
        // version test.
        let mut payload = vec![0_u8; 36];
        payload[0..4].copy_from_slice(&[0x7A, 0x05, 0x20, 0x00]);
        payload[20..27].copy_from_slice(b"5.00.01");
        let frame = encode_response_with_trailer(&payload, 0x6ED1);

        let beacon = observe_bootloader(&mut ScriptedTransport::new(frame, 1)).unwrap();
        assert_eq!(beacon.command(), 0x057A);
        assert_eq!(beacon.protocol(), BeaconProtocol::AesEncrypted);
        assert_eq!(beacon.version(), "5.00.01");
    }

    #[test]
    fn observation_still_refuses_a_frame_that_is_not_a_beacon() {
        let mut short = vec![0_u8; 20];
        short[0..4].copy_from_slice(&[0x18, 0x05, 0x20, 0x00]);
        assert!(matches!(
            observe_bootloader(&mut ScriptedTransport::new(
                encode_response_with_trailer(&short, 0x6ED1),
                1
            )),
            Err(FlashError::UnexpectedPacket { .. })
        ));

        let mut unprintable = vec![0_u8; 36];
        unprintable[0..4].copy_from_slice(&[0x18, 0x05, 0x20, 0x00]);
        unprintable[20..27].copy_from_slice(&[0x01; 7]);
        assert!(matches!(
            observe_bootloader(&mut ScriptedTransport::new(
                encode_response_with_trailer(&unprintable, 0x6ED1),
                1
            )),
            Err(FlashError::InvalidText("bootloader version"))
        ));
    }

    #[test]
    fn an_eeprom_backup_names_a_radio_in_bootloader_mode() {
        let mut transport = ScriptedTransport::new(beacon(), 1);
        let error = backup_eeprom(&mut transport).unwrap_err();
        assert!(matches!(error, FlashError::InBootloader), "{error}");
        assert!(error.to_string().contains("bootloader (flashing) mode"));
    }

    #[test]
    fn an_eeprom_backup_is_refused_on_afik_firmware_which_cannot_serve_it() {
        for identity in [
            "AFIK-K5-7.0.5",
            "AFIK-K5-2.3T",
            "AFIK-K1-7.0.6",
            "AFIK-K5-7.0",
        ] {
            let mut hello = vec![0_u8; 40];
            hello[0..4].copy_from_slice(&[0x15, 0x05, 0x24, 0x00]);
            hello[4..4 + identity.len()].copy_from_slice(identity.as_bytes());
            let mut transport = ScriptedTransport::new(encode_response(&hello), 1);
            let error = backup_eeprom(&mut transport).unwrap_err();
            assert!(
                matches!(&error, FlashError::EepromNotServed(version) if version == identity),
                "{identity}: {error}"
            );
            // Only the hello was sent: no read the firmware would never answer.
            assert_eq!(decode_requests(&transport.writes).len(), 1);
        }
    }

    /// A radio which is silent until it is sent a request, then answers it.
    struct AnswersWhenAsked {
        response: Vec<u8>,
        asked: bool,
        position: usize,
    }

    impl Read for AnswersWhenAsked {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if !self.asked || self.position == self.response.len() || buffer.is_empty() {
                return Ok(0);
            }
            buffer[0] = self.response[self.position];
            self.position += 1;
            Ok(1)
        }
    }

    impl Write for AnswersWhenAsked {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.asked = true;
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn identification_hears_a_bootloader_without_sending_anything() {
        let mut transport = ScriptedTransport::new(beacon(), 1);
        let presence = identify_radio(&mut transport).unwrap();
        assert!(matches!(
            presence,
            RadioPresence::Bootloader(BootloaderFamily::K5V1(ref info)) if info.version() == "2.00.06"
        ));
        assert!(
            transport.writes.is_empty(),
            "a bootloader is never written to"
        );
    }

    #[test]
    fn identification_skips_the_tail_of_a_beacon_it_joined_late() {
        // The bench saw `invalid packet footer: 8be6` from a port opened
        // mid-beacon: a header followed by the wrong bytes where a footer goes.
        let mut stream = vec![0xAB, 0xCD, 0x04, 0x00, 1, 2, 3, 4, 5, 6, 0x8B, 0xE6];
        stream.extend(beacon());
        let mut transport = ScriptedTransport::new(stream, 1);
        let presence = identify_radio(&mut transport).unwrap();
        assert!(matches!(presence, RadioPresence::Bootloader(_)));
        assert!(transport.writes.is_empty());
    }

    #[test]
    fn identification_asks_running_firmware_only_after_hearing_no_beacon() {
        let mut radio = AnswersWhenAsked {
            response: hello_response(),
            asked: false,
            position: 0,
        };
        let presence = identify_radio(&mut radio).unwrap();
        assert!(
            matches!(presence, RadioPresence::Firmware(ref info) if info.version() == "F4HWN v4.3")
        );

        // Nothing on the cable at all is the listen's timeout.
        let mut nothing = ScriptedTransport::new(Vec::new(), 1);
        assert!(matches!(
            identify_radio(&mut nothing),
            Err(FlashError::Timeout)
        ));
        assert!(serves_eeprom_reads("k5_2.01.23"));
        assert!(!serves_eeprom_reads("AFIK-K1-7.0.6"));
    }

    #[test]
    fn afik_k5_serves_eeprom_reads_from_7_0_6() {
        for (identity, serves) in [
            ("AFIK-K5-7.0.6", true),
            ("AFIK-K5-7.1.0", true),
            ("AFIK-K5-10.0.0", true),
            ("AFIK-K5-7.0.5", false),
            ("k5_2.01.23", true),
        ] {
            let info = NormalFirmwareInfo {
                version: identity.to_owned(),
            };
            assert_eq!(require_eeprom_service(&info).is_ok(), serves, "{identity}");
        }
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
    fn restore_preflight_plans_changed_blocks_and_writes_vendor_051d_shape() {
        let live = valid_backup();
        let mut replacement_bytes = live.bytes().to_vec();
        replacement_bytes[0] ^= 0x11;
        replacement_bytes[8] ^= 0x22;
        let replacement = EepromBackup::from_raw(&replacement_bytes).unwrap();
        let recovery = recovery_image();
        let confirmation = format!(
            "{QUALIFIED_TARGET_CONFIRMATION}:{:08x}",
            replacement.crc32()
        );
        let responses = restore_responses(live.bytes(), replacement.bytes(), &[0, 8]);
        let mut transport = ScriptedTransport::new(responses, 1);
        let plan = prepare_eeprom_restore(
            &mut transport,
            restore_prerequisites(&replacement, &live, &recovery, &confirmation),
        )
        .unwrap();
        assert_eq!(plan.changed_offsets(), [0, 8]);
        let report = apply_eeprom_restore(&mut transport, &plan).unwrap();
        assert_eq!(report.changed_blocks, 2);
        assert!(report.verified_read_back);
        let requests = decode_requests(&transport.writes);
        assert_eq!(
            requests.iter().filter(|p| p[0..2] == [0x1D, 0x05]).count(),
            2
        );
        assert_eq!(&requests[1][8..12], &super::SESSION_WORD.to_le_bytes());
        let write = requests.iter().find(|p| p[0..2] == [0x1D, 0x05]).unwrap();
        assert_eq!(u16::from_le_bytes([write[2], write[3]]), 16);
        assert_eq!(u16::from_le_bytes([write[4], write[5]]), 0);
        assert_eq!(write[6], 8);
        assert_eq!(write[7], 0);
        assert_eq!(&write[8..12], &super::SESSION_WORD.to_le_bytes());
        assert_eq!(&write[12..20], &replacement.bytes()[..8]);
    }

    #[test]
    fn restore_same_image_is_verified_without_a_write() {
        let live = valid_backup();
        let recovery = recovery_image();
        let confirmation = format!("{QUALIFIED_TARGET_CONFIRMATION}:{:08x}", live.crc32());
        let mut responses = hello_response();
        for offset in (0..EEPROM_BYTES).step_by(EEPROM_BLOCK_BYTES) {
            responses.extend(eeprom_response(
                offset,
                &live.bytes()[offset..offset + EEPROM_BLOCK_BYTES],
            ));
        }
        let mut transport = ScriptedTransport::new(responses, 1);
        let plan = prepare_eeprom_restore(
            &mut transport,
            restore_prerequisites(&live, &live, &recovery, &confirmation),
        )
        .unwrap();
        let report = apply_eeprom_restore(&mut transport, &plan).unwrap();
        assert_eq!(report.changed_blocks, 0);
        assert!(report.verified_read_back);
        assert!(decode_requests(&transport.writes)
            .iter()
            .all(|payload| payload[0..2] != [0x1D, 0x05]));
    }

    #[test]
    fn restore_refuses_live_mismatch_before_any_write() {
        let expected = valid_backup();
        let mut live_bytes = expected.bytes().to_vec();
        live_bytes[0] ^= 1;
        let live = EepromBackup::from_raw(&live_bytes).unwrap();
        let recovery = recovery_image();
        let confirmation = format!("{QUALIFIED_TARGET_CONFIRMATION}:{:08x}", expected.crc32());
        let mut responses = hello_response();
        for offset in (0..EEPROM_BYTES).step_by(EEPROM_BLOCK_BYTES) {
            responses.extend(eeprom_response(
                offset,
                &live.bytes()[offset..offset + EEPROM_BLOCK_BYTES],
            ));
        }
        let mut transport = ScriptedTransport::new(responses, 1);
        assert!(matches!(
            prepare_eeprom_restore(
                &mut transport,
                restore_prerequisites(&expected, &expected, &recovery, &confirmation),
            ),
            Err(FlashError::EepromCurrentMismatch)
        ));
        assert_eq!(decode_requests(&transport.writes).len(), 65);
    }

    #[test]
    fn restore_wrong_confirmation_and_recovery_are_rejected_without_transport() {
        let live = valid_backup();
        let recovery = recovery_image();
        let mut transport = ScriptedTransport::new(Vec::new(), 1);
        assert!(matches!(
            prepare_eeprom_restore(
                &mut transport,
                restore_prerequisites(&live, &live, &recovery, "UV-K5-V1-DP32G030:00000000"),
            ),
            Err(FlashError::EepromRestoreConfirmationMismatch)
        ));
        assert!(transport.writes.is_empty());
    }

    #[test]
    fn restore_checks_live_firmware_before_reading_eeprom() {
        let live = valid_backup();
        let recovery = recovery_image();
        let confirmation = format!("{QUALIFIED_TARGET_CONFIRMATION}:{:08x}", live.crc32());
        let mut prerequisites = restore_prerequisites(&live, &live, &recovery, &confirmation);
        let expected = FirmwareVersion::normal("F4HWN v4.4").unwrap();
        prerequisites.expected_firmware = Box::leak(Box::new(expected));
        let mut transport = ScriptedTransport::new(hello_response(), 1);
        assert!(matches!(
            prepare_eeprom_restore(&mut transport, prerequisites),
            Err(FlashError::EepromFirmwareMismatch)
        ));
        assert_eq!(decode_requests(&transport.writes).len(), 1);
    }

    #[test]
    fn restore_checks_recovery_crc_before_any_transport_access() {
        let live = valid_backup();
        let recovery = recovery_image();
        let confirmation = format!("{QUALIFIED_TARGET_CONFIRMATION}:{:08x}", live.crc32());
        let mut prerequisites = restore_prerequisites(&live, &live, &recovery, &confirmation);
        prerequisites.recovery_crc32 ^= 1;
        let mut transport = ScriptedTransport::new(Vec::new(), 1);
        assert!(matches!(
            prepare_eeprom_restore(&mut transport, prerequisites),
            Err(FlashError::EepromRecoveryMismatch)
        ));
        assert!(transport.writes.is_empty());
    }

    #[test]
    fn restore_requires_the_separately_supplied_recovery_sha256() {
        let live = valid_backup();
        let recovery = recovery_image();
        let confirmation = format!("{QUALIFIED_TARGET_CONFIRMATION}:{:08x}", live.crc32());
        let mut prerequisites = restore_prerequisites(&live, &live, &recovery, &confirmation);
        prerequisites.recovery_sha256[0] ^= 1;
        let mut transport = ScriptedTransport::new(Vec::new(), 1);
        assert!(matches!(
            prepare_eeprom_restore(&mut transport, prerequisites),
            Err(FlashError::EepromRecoveryMismatch)
        ));
        assert!(transport.writes.is_empty());
    }

    #[test]
    fn restore_stops_before_write_when_immediate_reads_disagree() {
        let live = valid_backup();
        let mut replacement_bytes = live.bytes().to_vec();
        replacement_bytes[0] ^= 1;
        let replacement = EepromBackup::from_raw(&replacement_bytes).unwrap();
        let recovery = recovery_image();
        let confirmation = format!(
            "{QUALIFIED_TARGET_CONFIRMATION}:{:08x}",
            replacement.crc32()
        );
        let mut responses = restore_responses(live.bytes(), replacement.bytes(), &[0]);
        // Replace the first block's second pre-write read with a changed but valid block.
        let stable = eeprom_response(0, &live.bytes()[..8]);
        let stable_len = stable.len();
        let mut changed = live.bytes()[..8].to_vec();
        changed[0] ^= 0x40;
        let changed_response = eeprom_response(0, &changed);
        let first = responses
            .windows(stable.len())
            .position(|window| window == stable.as_slice())
            .expect("pre-write response");
        responses.splice(first..first + stable_len, stable);
        let second = first + stable_len;
        responses.splice(second..second + stable_len, changed_response);
        let mut transport = ScriptedTransport::new(responses, 1);
        let plan = prepare_eeprom_restore(
            &mut transport,
            restore_prerequisites(&replacement, &live, &recovery, &confirmation),
        )
        .unwrap();
        assert!(matches!(
            apply_eeprom_restore(&mut transport, &plan),
            Err(FlashError::EepromBlockChangedDuringPreflight { offset: 0 })
        ));
        assert_eq!(decode_requests(&transport.writes).len(), 67);
        assert!(decode_requests(&transport.writes)
            .iter()
            .all(|payload| payload[0..2] != [0x1D, 0x05]));
    }

    #[test]
    fn forged_restore_plans_are_rejected_without_transport_access() {
        let live = valid_backup();
        let mut replacement_bytes = live.bytes().to_vec();
        replacement_bytes[0] ^= 1;
        let replacement = EepromBackup::from_raw(&replacement_bytes).unwrap();
        let recovery = recovery_image();
        let confirmation = format!(
            "{QUALIFIED_TARGET_CONFIRMATION}:{:08x}",
            replacement.crc32()
        );
        let responses = restore_responses(live.bytes(), replacement.bytes(), &[0]);
        let mut transport = ScriptedTransport::new(responses, 1);
        let plan = prepare_eeprom_restore(
            &mut transport,
            restore_prerequisites(&replacement, &live, &recovery, &confirmation),
        )
        .unwrap();
        let mut forged = plan.clone();
        forged.changed_offsets = vec![8, 0];
        let mut empty = ScriptedTransport::new(Vec::new(), 1);
        assert!(matches!(
            apply_eeprom_restore(&mut empty, &forged),
            Err(FlashError::InvalidEepromRestorePlan)
        ));
        forged.changed_offsets = Vec::new();
        assert!(matches!(
            apply_eeprom_restore(&mut empty, &forged),
            Err(FlashError::InvalidEepromRestorePlan)
        ));
        forged.changed_offsets = vec![0x2000];
        assert!(matches!(
            apply_eeprom_restore(&mut empty, &forged),
            Err(FlashError::InvalidEepromRestorePlan)
        ));
    }

    #[test]
    fn restore_block_timeout_or_bad_readback_stops_without_retry() {
        let live = valid_backup();
        let mut replacement_bytes = live.bytes().to_vec();
        replacement_bytes[0] ^= 1;
        let replacement = EepromBackup::from_raw(&replacement_bytes).unwrap();
        let recovery = recovery_image();
        let confirmation = format!(
            "{QUALIFIED_TARGET_CONFIRMATION}:{:08x}",
            replacement.crc32()
        );
        let mut responses = hello_response();
        for offset in (0..EEPROM_BYTES).step_by(EEPROM_BLOCK_BYTES) {
            responses.extend(eeprom_response(
                offset,
                &live.bytes()[offset..offset + EEPROM_BLOCK_BYTES],
            ));
        }
        responses.extend(eeprom_response(0, &live.bytes()[..8]));
        responses.extend(eeprom_response(0, &live.bytes()[..8]));
        responses.extend(eeprom_write_ack(0));
        responses.extend(eeprom_response(0, &live.bytes()[..8]));
        let mut transport = ScriptedTransport::new(responses, 1);
        let plan = prepare_eeprom_restore(
            &mut transport,
            restore_prerequisites(&replacement, &live, &recovery, &confirmation),
        )
        .unwrap();
        assert!(matches!(
            apply_eeprom_restore(&mut transport, &plan),
            Err(FlashError::EepromBlockVerificationFailed { offset: 0 })
        ));
        assert_eq!(decode_requests(&transport.writes).len(), 69);
    }

    #[test]
    fn restore_wrong_write_ack_stops_before_the_next_block() {
        let live = valid_backup();
        let mut replacement_bytes = live.bytes().to_vec();
        replacement_bytes[0] ^= 1;
        replacement_bytes[8] ^= 2;
        let replacement = EepromBackup::from_raw(&replacement_bytes).unwrap();
        let recovery = recovery_image();
        let confirmation = format!(
            "{QUALIFIED_TARGET_CONFIRMATION}:{:08x}",
            replacement.crc32()
        );
        let mut responses = hello_response();
        for offset in (0..EEPROM_BYTES).step_by(EEPROM_BLOCK_BYTES) {
            responses.extend(eeprom_response(
                offset,
                &live.bytes()[offset..offset + EEPROM_BLOCK_BYTES],
            ));
        }
        responses.extend(eeprom_response(0, &live.bytes()[..8]));
        responses.extend(eeprom_response(0, &live.bytes()[..8]));
        responses.extend(eeprom_write_ack(8));
        let mut transport = ScriptedTransport::new(responses, 1);
        let plan = prepare_eeprom_restore(
            &mut transport,
            restore_prerequisites(&replacement, &live, &recovery, &confirmation),
        )
        .unwrap();
        assert!(matches!(
            apply_eeprom_restore(&mut transport, &plan),
            Err(FlashError::EepromWriteAcknowledgementMismatch {
                expected_offset: 0,
                actual_offset: 8
            })
        ));
        assert_eq!(decode_requests(&transport.writes).len(), 68);
    }

    #[test]
    fn restore_missing_write_ack_times_out_without_retry() {
        let live = valid_backup();
        let mut replacement_bytes = live.bytes().to_vec();
        replacement_bytes[0] ^= 1;
        let replacement = EepromBackup::from_raw(&replacement_bytes).unwrap();
        let recovery = recovery_image();
        let confirmation = format!(
            "{QUALIFIED_TARGET_CONFIRMATION}:{:08x}",
            replacement.crc32()
        );
        let mut responses = hello_response();
        for offset in (0..EEPROM_BYTES).step_by(EEPROM_BLOCK_BYTES) {
            responses.extend(eeprom_response(
                offset,
                &live.bytes()[offset..offset + EEPROM_BLOCK_BYTES],
            ));
        }
        responses.extend(eeprom_response(0, &live.bytes()[..8]));
        responses.extend(eeprom_response(0, &live.bytes()[..8]));
        let mut transport = ScriptedTransport::new(responses, 1);
        let plan = prepare_eeprom_restore(
            &mut transport,
            restore_prerequisites(&replacement, &live, &recovery, &confirmation),
        )
        .unwrap();
        assert!(matches!(
            apply_eeprom_restore(&mut transport, &plan),
            Err(FlashError::Timeout)
        ));
        assert_eq!(decode_requests(&transport.writes).len(), 68);
    }

    #[test]
    fn restore_preflight_rejects_wrong_offset_or_length_response() {
        let live = valid_backup();
        let recovery = recovery_image();
        let confirmation = format!("{QUALIFIED_TARGET_CONFIRMATION}:{:08x}", live.crc32());
        for response in [
            eeprom_response(8, &live.bytes()[..EEPROM_BLOCK_BYTES]),
            eeprom_response(0, &live.bytes()[..EEPROM_BLOCK_BYTES - 1]),
        ] {
            let mut responses = hello_response();
            responses.extend(response);
            let mut transport = ScriptedTransport::new(responses, 1);
            let result = prepare_eeprom_restore(
                &mut transport,
                restore_prerequisites(&live, &live, &recovery, &confirmation),
            );
            assert!(matches!(
                result,
                Err(FlashError::EepromBlockMismatch { .. } | FlashError::UnexpectedPacket { .. })
            ));
        }
    }

    #[test]
    fn restore_rejects_a_final_full_image_mismatch() {
        let live = valid_backup();
        let mut replacement_bytes = live.bytes().to_vec();
        replacement_bytes[0] ^= 1;
        let replacement = EepromBackup::from_raw(&replacement_bytes).unwrap();
        let recovery = recovery_image();
        let confirmation = format!(
            "{QUALIFIED_TARGET_CONFIRMATION}:{:08x}",
            replacement.crc32()
        );
        let mut responses = hello_response();
        for offset in (0..EEPROM_BYTES).step_by(EEPROM_BLOCK_BYTES) {
            responses.extend(eeprom_response(
                offset,
                &live.bytes()[offset..offset + EEPROM_BLOCK_BYTES],
            ));
        }
        responses.extend(eeprom_response(0, &live.bytes()[..8]));
        responses.extend(eeprom_response(0, &live.bytes()[..8]));
        responses.extend(eeprom_write_ack(0));
        responses.extend(eeprom_response(0, &replacement.bytes()[..8]));
        for offset in (0..EEPROM_BYTES).step_by(EEPROM_BLOCK_BYTES) {
            responses.extend(eeprom_response(
                offset,
                &live.bytes()[offset..offset + EEPROM_BLOCK_BYTES],
            ));
        }
        let mut transport = ScriptedTransport::new(responses, 1);
        let plan = prepare_eeprom_restore(
            &mut transport,
            restore_prerequisites(&replacement, &live, &recovery, &confirmation),
        )
        .unwrap();
        assert!(matches!(
            apply_eeprom_restore(&mut transport, &plan),
            Err(FlashError::EepromRestoreVerificationFailed)
        ));
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
