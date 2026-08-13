//! K5 identity supplied to the shared normal-mode serial service.

pub use radio_platform::serial::{
    ApplicationIdentity, HelloService, REQUEST_BODY_BYTES, RESPONSE_FRAME_BYTES,
};

/// Printable identity returned by the verified AFIK K5 application.
pub const APPLICATION_VERSION: &[u8] = b"AFIK-K5-1.8U";
/// Validated identity consumed by the shared application service.
pub const APPLICATION_IDENTITY: ApplicationIdentity<'static> =
    ApplicationIdentity::new(APPLICATION_VERSION).expect("K5 identity is printable and bounded");

/// Plain-text banner retained for bounded serial diagnostics.
pub const BOOT_BANNER: &[u8] = b"AFIK-K5-1.8U booted";
