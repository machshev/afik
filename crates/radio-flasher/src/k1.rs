//! Independently implemented UV-K1 recovery protocol.

use std::{fmt, io::Read, io::Write};

use crate::{receive_packet, send_packet, FlashError, Packet};

/// Exact operator phrase for the validated K1 recovery protocol.
pub const K1_RECOVERY_TARGET_CONFIRMATION: &str = "UV-K1-F4HWN-7.03.01";
/// K1 application flash origin from the pinned source linker contract.
pub const K1_APPLICATION_ORIGIN: u32 = 0x0800_2800;
/// Exclusive K1 application flash end from the pinned source linker contract.
pub const K1_APPLICATION_END: u32 = 0x0802_0000;
/// K1 initial SRAM address.
pub const K1_SRAM_ORIGIN: u32 = 0x2000_0000;
/// K1 exclusive SRAM end for the pinned 16 KiB source contract.
pub const K1_SRAM_END: u32 = 0x2000_4000;
/// K1 bootloader page size observed in the recovery experiment.
pub const K1_FLASH_PAGE_BYTES: usize = 256;
const K1_BEACON: u16 = 0x0518;
const K1_VERSION_REQUEST: u16 = 0x0530;
const K1_PAGE_REQUEST: u16 = 0x0519;
const K1_PAGE_RESPONSE: u16 = 0x051A;

/// Rejection from bounded K1 recovery-image validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum K1ImageError {
    /// The raw image does not contain both vector words.
    TooShort(usize),
    /// The raw image exceeds the pinned K1 application capacity.
    TooLong(usize),
    /// The vector-table stack pointer is outside the pinned SRAM range.
    InvalidStack(u32),
    /// The vector-table reset pointer is outside the pinned application range.
    InvalidReset(u32),
}

impl fmt::Display for K1ImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort(length) => write!(formatter, "K1 image is too short: {length}"),
            Self::TooLong(length) => write!(formatter, "K1 image is too long: {length}"),
            Self::InvalidStack(value) => {
                write!(formatter, "invalid K1 stack pointer: 0x{value:08x}")
            }
            Self::InvalidReset(value) => {
                write!(formatter, "invalid K1 reset vector: 0x{value:08x}")
            }
        }
    }
}

impl std::error::Error for K1ImageError {}

/// Rejection from the bounded K1 recovery workflow.
#[derive(Debug)]
pub enum K1FlashError {
    /// The shared legacy frame transport rejected a packet.
    Transport(FlashError),
    /// The operator did not confirm the exact K1 recovery target.
    TargetNotConfirmed,
    /// The selected bootloader version is not the pinned K1 shape.
    UnsupportedBootloader(String),
    /// The raw recovery image is outside the pinned K1 vector/range contract.
    InvalidImage(K1ImageError),
    /// The per-run transaction identifier was zero.
    InvalidTransactionId,
    /// A response command or payload length was not the expected K1 response.
    UnexpectedPacket {
        /// Decoded response command.
        command: u16,
        /// Decoded response payload length.
        length: usize,
    },
    /// A page response did not echo the transaction identifier or page index.
    PageAcknowledgementMismatch {
        /// Page that was sent.
        expected_page: u16,
        /// Page named by the response.
        actual_page: u16,
        /// Transaction identifier that was sent.
        expected_transaction: u32,
        /// Transaction identifier named by the response.
        actual_transaction: u32,
    },
    /// The bootloader rejected a page with a nonzero result.
    PageRejected {
        /// Page named by the response.
        page: u16,
        /// Nonzero bootloader result.
        result: u16,
    },
}

impl fmt::Display for K1FlashError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => write!(formatter, "K1 transport failed: {error}"),
            Self::TargetNotConfirmed => formatter.write_str("K1 recovery target not confirmed"),
            Self::UnsupportedBootloader(version) => {
                write!(formatter, "unsupported K1 bootloader version: {version}")
            }
            Self::InvalidImage(error) => write!(formatter, "invalid K1 recovery image: {error}"),
            Self::InvalidTransactionId => formatter.write_str("K1 transaction identifier is zero"),
            Self::UnexpectedPacket { command, length } => write!(
                formatter,
                "unexpected K1 response: command=0x{command:04x}, length={length}"
            ),
            Self::PageAcknowledgementMismatch {
                expected_page,
                actual_page,
                expected_transaction,
                actual_transaction,
            } => write!(
                formatter,
                "K1 page acknowledgement mismatch: page {expected_page} / {actual_page}, transaction 0x{expected_transaction:08x} / 0x{actual_transaction:08x}"
            ),
            Self::PageRejected { page, result } => {
                write!(formatter, "K1 bootloader rejected page {page}: result={result}")
            }
        }
    }
}

impl std::error::Error for K1FlashError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::InvalidImage(error) => Some(error),
            _ => None,
        }
    }
}

impl From<FlashError> for K1FlashError {
    fn from(error: FlashError) -> Self {
        Self::Transport(error)
    }
}

/// A vector-checked raw K1 recovery application.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct K1RecoveryImage {
    bytes: Vec<u8>,
}

impl K1RecoveryImage {
    /// Validates raw vectors and the pinned K1 application address range.
    ///
    /// # Panics
    ///
    /// This does not panic for the fixed compile-time K1 address range.
    pub fn from_raw(raw: &[u8]) -> Result<Self, K1ImageError> {
        if raw.len() < 8 {
            return Err(K1ImageError::TooShort(raw.len()));
        }
        let capacity = usize::try_from(K1_APPLICATION_END - K1_APPLICATION_ORIGIN)
            .expect("K1 application capacity is bounded");
        if raw.len() > capacity {
            return Err(K1ImageError::TooLong(raw.len()));
        }
        let stack = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
        if !(K1_SRAM_ORIGIN..=K1_SRAM_END).contains(&stack) || stack % 4 != 0 {
            return Err(K1ImageError::InvalidStack(stack));
        }
        let reset = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]);
        if reset & 1 == 0 || !(K1_APPLICATION_ORIGIN..K1_APPLICATION_END).contains(&(reset & !1)) {
            return Err(K1ImageError::InvalidReset(reset));
        }
        Ok(Self {
            bytes: raw.to_vec(),
        })
    }

    /// Returns the complete raw application bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the bounded number of 256-byte writes required by the protocol.
    ///
    /// # Panics
    ///
    /// This does not panic for an image accepted by [`Self::from_raw`].
    pub fn page_count(&self) -> u16 {
        u16::try_from(self.bytes.len().div_ceil(K1_FLASH_PAGE_BYTES))
            .expect("K1 image page count is bounded")
    }
}

/// Result after every K1 application page returned an exact zero-result ack.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct K1FlashReport {
    /// Bootloader version observed during the write.
    pub bootloader_version: String,
    /// Number of sequential pages acknowledged.
    pub pages_acknowledged: u16,
    /// Per-run transaction identifier echoed by every page response.
    pub transaction_id: u32,
}

/// Writes a validated raw recovery image through the pinned K1 bootloader.
///
/// The caller must have consumed the first K1 beacon while classifying the
/// device. Three additional beacons are required for the version handshakes.
/// Missing, malformed, mismatched, and rejected pages stop immediately; this
/// function never retries a page.
pub fn flash_recovery<T, F>(
    transport: &mut T,
    image: &K1RecoveryImage,
    bootloader_version: &str,
    target_confirmation: &str,
    transaction_id: u32,
    mut page_acknowledged: F,
) -> Result<K1FlashReport, K1FlashError>
where
    T: Read + Write,
    F: FnMut(u16),
{
    if target_confirmation != K1_RECOVERY_TARGET_CONFIRMATION {
        return Err(K1FlashError::TargetNotConfirmed);
    }
    if !is_supported_bootloader_version(bootloader_version) {
        return Err(K1FlashError::UnsupportedBootloader(
            bootloader_version.to_owned(),
        ));
    }
    if transaction_id == 0 {
        return Err(K1FlashError::InvalidTransactionId);
    }

    for _ in 0..3 {
        let beacon = receive_packet(transport)?;
        let actual_version = parse_k1_beacon(&beacon)?;
        if actual_version != bootloader_version {
            return Err(K1FlashError::UnsupportedBootloader(actual_version));
        }
        send_packet(transport, &version_request())?;
    }

    let page_count = image.page_count();
    for page in 0..page_count {
        send_packet(
            transport,
            &page_request(image, transaction_id, page, page_count),
        )?;
        receive_page_ack(transport, transaction_id, page)?;
        page_acknowledged(page);
    }

    Ok(K1FlashReport {
        bootloader_version: bootloader_version.to_owned(),
        pages_acknowledged: page_count,
        transaction_id,
    })
}

/// Returns whether a version string is the pinned K1 bootloader shape.
pub fn is_supported_bootloader_version(version: &str) -> bool {
    version.starts_with("7.03.")
        && version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
}

fn parse_k1_beacon(packet: &Packet) -> Result<String, K1FlashError> {
    let payload = packet.as_slice();
    if payload.len() != 36 || u16::from_le_bytes([payload[0], payload[1]]) != K1_BEACON {
        return Err(K1FlashError::UnexpectedPacket {
            command: packet_command(payload),
            length: payload.len(),
        });
    }
    let end = payload[20..36]
        .iter()
        .position(|byte| *byte == 0)
        .map_or(16, |index| index);
    let version = String::from_utf8_lossy(&payload[20..20 + end]).into_owned();
    if !is_supported_bootloader_version(&version) {
        return Err(K1FlashError::UnsupportedBootloader(version));
    }
    Ok(version)
}

fn version_request() -> [u8; 8] {
    let mut payload = [0_u8; 8];
    payload[0..2].copy_from_slice(&K1_VERSION_REQUEST.to_le_bytes());
    payload[2..4].copy_from_slice(&4_u16.to_le_bytes());
    payload[4..8].copy_from_slice(b"7.03");
    payload
}

fn page_request(image: &K1RecoveryImage, transaction: u32, page: u16, page_count: u16) -> Vec<u8> {
    let mut payload = vec![0_u8; 272];
    payload[0..2].copy_from_slice(&K1_PAGE_REQUEST.to_le_bytes());
    payload[2..4].copy_from_slice(&268_u16.to_le_bytes());
    payload[4..8].copy_from_slice(&transaction.to_le_bytes());
    payload[8..10].copy_from_slice(&page.to_le_bytes());
    payload[10..12].copy_from_slice(&page_count.to_le_bytes());
    let start = usize::from(page) * K1_FLASH_PAGE_BYTES;
    let end = (start + K1_FLASH_PAGE_BYTES).min(image.bytes.len());
    payload[16..16 + end - start].copy_from_slice(&image.bytes[start..end]);
    payload
}

fn receive_page_ack<T: Read>(
    transport: &mut T,
    transaction: u32,
    expected_page: u16,
) -> Result<(), K1FlashError> {
    loop {
        let packet = receive_packet(transport)?;
        if packet_command(packet.as_slice()) == K1_BEACON {
            parse_k1_beacon(&packet)?;
            continue;
        }
        let payload = packet.as_slice();
        if packet_command(payload) != K1_PAGE_RESPONSE || payload.len() != 12 {
            return Err(K1FlashError::UnexpectedPacket {
                command: packet_command(payload),
                length: payload.len(),
            });
        }
        let actual_transaction =
            u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
        let actual_page = u16::from_le_bytes([payload[8], payload[9]]);
        let result = u16::from_le_bytes([payload[10], payload[11]]);
        if actual_transaction != transaction || actual_page != expected_page {
            return Err(K1FlashError::PageAcknowledgementMismatch {
                expected_page,
                actual_page,
                expected_transaction: transaction,
                actual_transaction,
            });
        }
        if result != 0 {
            return Err(K1FlashError::PageRejected {
                page: expected_page,
                result,
            });
        }
        return Ok(());
    }
}

fn packet_command(payload: &[u8]) -> u16 {
    if payload.len() < 2 {
        return 0;
    }
    u16::from_le_bytes([payload[0], payload[1]])
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read, Write};

    use super::{
        flash_recovery, K1FlashError, K1ImageError, K1RecoveryImage,
        K1_RECOVERY_TARGET_CONFIRMATION,
    };
    use crate::codec::encode_response;

    struct ScriptedTransport {
        input: Cursor<Vec<u8>>,
        output: Vec<u8>,
    }

    const XOR_KEY: [u8; 16] = [
        0x16, 0x6C, 0x14, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5, 0x40, 0x13, 0x03, 0xE9,
        0x80,
    ];

    impl Read for ScriptedTransport {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(buffer)
        }
    }

    impl Write for ScriptedTransport {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.output.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn beacon() -> Vec<u8> {
        let mut payload = vec![0_u8; 36];
        payload[0..2].copy_from_slice(&0x0518_u16.to_le_bytes());
        payload[2..4].copy_from_slice(&32_u16.to_le_bytes());
        payload[20..27].copy_from_slice(b"7.03.01");
        encode_response(&payload)
    }

    fn ack(transaction: u32, page: u16, result: u16) -> Vec<u8> {
        let mut payload = vec![0_u8; 12];
        payload[0..2].copy_from_slice(&0x051A_u16.to_le_bytes());
        payload[2..4].copy_from_slice(&8_u16.to_le_bytes());
        payload[4..8].copy_from_slice(&transaction.to_le_bytes());
        payload[8..10].copy_from_slice(&page.to_le_bytes());
        payload[10..12].copy_from_slice(&result.to_le_bytes());
        encode_response(&payload)
    }

    fn raw_image(length: usize) -> Vec<u8> {
        let mut raw = vec![0_u8; length];
        raw[0..4].copy_from_slice(&0x2000_4000_u32.to_le_bytes());
        raw[4..8].copy_from_slice(&0x0800_2D49_u32.to_le_bytes());
        raw
    }

    #[test]
    fn image_validates_vectors_and_page_count() {
        let image = K1RecoveryImage::from_raw(&raw_image(257)).unwrap();
        assert_eq!(image.page_count(), 2);
        assert!(matches!(
            K1RecoveryImage::from_raw(&[0; 4]),
            Err(K1ImageError::TooShort(4))
        ));
        let mut bad = raw_image(256);
        bad[0..4].copy_from_slice(&0x1000_u32.to_le_bytes());
        assert!(matches!(
            K1RecoveryImage::from_raw(&bad),
            Err(K1ImageError::InvalidStack(_))
        ));
    }

    #[test]
    fn recovery_handshakes_and_acknowledges_zero_padded_pages() {
        let transaction = 0x1234_5678;
        let image = K1RecoveryImage::from_raw(&raw_image(257)).unwrap();
        let mut input = Vec::new();
        input.extend(beacon());
        input.extend(beacon());
        input.extend(beacon());
        input.extend(ack(transaction, 0, 0));
        input.extend(ack(transaction, 1, 0));
        let mut transport = ScriptedTransport {
            input: Cursor::new(input),
            output: Vec::new(),
        };
        let report = flash_recovery(
            &mut transport,
            &image,
            "7.03.01",
            K1_RECOVERY_TARGET_CONFIRMATION,
            transaction,
            |_| {},
        )
        .unwrap();
        assert_eq!(report.pages_acknowledged, 2);
        assert_eq!(transport.output.len(), 3 * 16 + 2 * 280);
        let page_one_start = 3 * 16 + 280;
        let mut page_one_body =
            transport.output[page_one_start + 4..page_one_start + 4 + 274].to_vec();
        for (index, byte) in page_one_body.iter_mut().enumerate() {
            *byte ^= XOR_KEY[index % XOR_KEY.len()];
        }
        assert_eq!(&page_one_body[0..4], &[0x19, 0x05, 0x0C, 0x01]);
        assert!(page_one_body[16..272].iter().all(|byte| *byte == 0));
        assert_eq!(transport.output.last().copied(), Some(0xBA));
    }

    #[test]
    fn rejected_page_stops_without_retry() {
        let transaction = 7;
        let image = K1RecoveryImage::from_raw(&raw_image(256)).unwrap();
        let mut input = Vec::new();
        input.extend(beacon());
        input.extend(beacon());
        input.extend(beacon());
        input.extend(ack(transaction, 0, 9));
        let mut transport = ScriptedTransport {
            input: Cursor::new(input),
            output: Vec::new(),
        };
        let result = flash_recovery(
            &mut transport,
            &image,
            "7.03.01",
            K1_RECOVERY_TARGET_CONFIRMATION,
            transaction,
            |_| {},
        );
        assert!(matches!(
            result,
            Err(K1FlashError::PageRejected { page: 0, result: 9 })
        ));
        assert_eq!(transport.output.len(), 3 * 16 + 280);
    }
}
