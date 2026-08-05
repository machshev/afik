//! Fail-closed central transmit authorisation.

#![no_std]
#![forbid(unsafe_code)]

use core::fmt;
use radio_domain::TxClass;

/// Current persisted permission encoding version.
pub const PERMISSION_FORMAT_VERSION: u8 = 1;
/// Exact byte length of a persisted permission record.
pub const PERMISSION_RECORD_LEN: usize = 9;

const KNOWN_PERMISSION_BITS: u8 = 0b0011_1111;

/// A set of centrally controlled transmit permissions.
#[must_use]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PermissionSet(u8);

impl PermissionSet {
    /// Returns an empty, default-deny permission set.
    pub const fn none() -> Self {
        Self(0)
    }

    /// Returns a copy with one class enabled or disabled.
    ///
    /// `TxClass::Never` remains disabled regardless of `enabled`.
    pub const fn with(self, class: TxClass, enabled: bool) -> Self {
        let Some(bit) = permission_bit(class) else {
            return self;
        };
        if enabled {
            Self(self.0 | bit)
        } else {
            Self(self.0 & !bit)
        }
    }

    /// Reports whether one class is enabled.
    pub const fn allows(self, class: TxClass) -> bool {
        match permission_bit(class) {
            Some(bit) => self.0 & bit != 0,
            None => false,
        }
    }

    const fn bits(self) -> u8 {
        self.0
    }

    const fn from_valid_bits(bits: u8) -> Self {
        Self(bits)
    }
}

const fn permission_bit(class: TxClass) -> Option<u8> {
    match class {
        TxClass::Never => None,
        TxClass::LicenceFreePlan => Some(1 << 0),
        TxClass::Amateur => Some(1 << 1),
        TxClass::Marine => Some(1 << 2),
        TxClass::Aeronautical => Some(1 << 3),
        TxClass::Business => Some(1 << 4),
        TxClass::Experimental => Some(1 << 5),
    }
}

/// A validated persisted TX-permission record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoredPermissions {
    permissions: PermissionSet,
    generation: u32,
}

impl StoredPermissions {
    /// Constructs a record for the hidden physical-presence settings path.
    pub const fn new(permissions: PermissionSet, generation: u32) -> Self {
        Self {
            permissions,
            generation,
        }
    }

    /// Returns the validated permission set.
    pub const fn permissions(self) -> PermissionSet {
        self.permissions
    }

    /// Returns the monotonic settings generation.
    pub const fn generation(self) -> u32 {
        self.generation
    }

    /// Encodes version, permissions, inversion, generation, and CRC.
    pub fn encode(self) -> [u8; PERMISSION_RECORD_LEN] {
        let mut bytes = [0_u8; PERMISSION_RECORD_LEN];
        bytes[0] = PERMISSION_FORMAT_VERSION;
        bytes[1] = self.permissions.bits();
        bytes[2] = !self.permissions.bits();
        bytes[3..7].copy_from_slice(&self.generation.to_le_bytes());
        let crc = crc16_ccitt_false(&bytes[..7]);
        bytes[7..9].copy_from_slice(&crc.to_le_bytes());
        bytes
    }

    /// Decodes a record only when every redundant check passes.
    pub fn decode(bytes: &[u8]) -> Result<Self, PermissionError> {
        if bytes.len() != PERMISSION_RECORD_LEN {
            return Err(PermissionError::InvalidLength);
        }
        if bytes[0] != PERMISSION_FORMAT_VERSION {
            return Err(PermissionError::UnsupportedVersion);
        }
        if bytes[1] & !KNOWN_PERMISSION_BITS != 0 {
            return Err(PermissionError::ReservedBits);
        }
        if bytes[2] != !bytes[1] {
            return Err(PermissionError::InversionMismatch);
        }
        let expected_crc = u16::from_le_bytes([bytes[7], bytes[8]]);
        if crc16_ccitt_false(&bytes[..7]) != expected_crc {
            return Err(PermissionError::CrcMismatch);
        }
        Ok(Self {
            permissions: PermissionSet::from_valid_bits(bytes[1]),
            generation: u32::from_le_bytes([bytes[3], bytes[4], bytes[5], bytes[6]]),
        })
    }
}

/// A persisted TX-permission validation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionError {
    /// The byte length does not match the versioned record.
    InvalidLength,
    /// The version is unsupported.
    UnsupportedVersion,
    /// Unknown permission bits were set.
    ReservedBits,
    /// The redundant inverted permission byte did not match.
    InversionMismatch,
    /// The record checksum did not match.
    CrcMismatch,
}

impl fmt::Display for PermissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength => formatter.write_str("invalid TX permission record length"),
            Self::UnsupportedVersion => formatter.write_str("unsupported TX permission version"),
            Self::ReservedBits => formatter.write_str("reserved TX permission bits are set"),
            Self::InversionMismatch => formatter.write_str("TX permission inversion mismatch"),
            Self::CrcMismatch => formatter.write_str("TX permission CRC mismatch"),
        }
    }
}

/// Whether stored policy was accepted or replaced by fail-closed defaults.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadStatus {
    /// A valid persisted record was loaded.
    Valid,
    /// Persisted bytes were invalid and all TX classes were disabled.
    DefaultedDenied(PermissionError),
}

/// Central policy authority used by every transmit request path.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TxPolicy {
    permissions: PermissionSet,
}

impl TxPolicy {
    /// Constructs the default policy with every class denied.
    pub const fn deny_all() -> Self {
        Self {
            permissions: PermissionSet::none(),
        }
    }

    /// Loads policy from persisted bytes, failing closed on every error.
    pub fn load(bytes: &[u8]) -> (Self, LoadStatus) {
        match StoredPermissions::decode(bytes) {
            Ok(stored) => (
                Self {
                    permissions: stored.permissions(),
                },
                LoadStatus::Valid,
            ),
            Err(error) => (Self::deny_all(), LoadStatus::DefaultedDenied(error)),
        }
    }

    /// Attempts to mint a token required by the hardware TX boundary.
    pub fn authorise(&self, class: TxClass) -> Result<TxAuthorisation, TxDenied> {
        if self.permissions.allows(class) {
            Ok(TxAuthorisation { _private: () })
        } else {
            Err(TxDenied)
        }
    }
}

/// Capability token required to enter hardware transmit state.
///
/// Its constructor is private to this crate. Drivers should accept a borrowed
/// token for the shortest possible scope.
#[derive(Debug)]
pub struct TxAuthorisation {
    _private: (),
}

/// Transmit authorisation was denied by central policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TxDenied;

impl fmt::Display for TxDenied {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("transmit denied by policy")
    }
}

fn crc16_ccitt_false(bytes: &[u8]) -> u16 {
    let mut crc = 0xffff_u16;
    for byte in bytes {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::{LoadStatus, PermissionSet, StoredPermissions, TxPolicy};
    use radio_domain::TxClass;

    #[test]
    fn default_policy_denies_every_class() {
        let policy = TxPolicy::default();
        for class in [
            TxClass::Never,
            TxClass::LicenceFreePlan,
            TxClass::Amateur,
            TxClass::Marine,
            TxClass::Aeronautical,
            TxClass::Business,
            TxClass::Experimental,
        ] {
            assert!(policy.authorise(class).is_err());
        }
    }

    #[test]
    fn valid_record_enables_only_selected_class() {
        let permissions = PermissionSet::none().with(TxClass::LicenceFreePlan, true);
        let bytes = StoredPermissions::new(permissions, 8).encode();
        let (policy, status) = TxPolicy::load(&bytes);
        assert_eq!(status, LoadStatus::Valid);
        assert!(policy.authorise(TxClass::LicenceFreePlan).is_ok());
        assert!(policy.authorise(TxClass::Amateur).is_err());
        assert!(policy.authorise(TxClass::Never).is_err());
    }

    #[test]
    fn corruption_fails_closed() {
        let permissions = PermissionSet::none()
            .with(TxClass::LicenceFreePlan, true)
            .with(TxClass::Amateur, true);
        let mut bytes = StoredPermissions::new(permissions, 3).encode();
        bytes[1] ^= 1;
        let (policy, status) = TxPolicy::load(&bytes);
        assert!(matches!(status, LoadStatus::DefaultedDenied(_)));
        assert!(policy.authorise(TxClass::LicenceFreePlan).is_err());
        assert!(policy.authorise(TxClass::Amateur).is_err());
    }
}
