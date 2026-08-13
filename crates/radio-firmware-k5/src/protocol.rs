//! K5 identity supplied to the shared normal-mode serial service.

use radio_platform::receive_app::ReceiveApp;
pub use radio_platform::serial::{
    ApplicationIdentity, HelloService, REQUEST_BODY_BYTES, RESPONSE_FRAME_BYTES,
};
pub use radio_platform::serial_control::{
    dispatch as dispatch_shared, Capabilities as SerialCapabilities,
    Capability as SerialCapability, Command as SerialCommand, Query as SerialQuery,
    Request as SerialRequest, Result as SerialResult,
};

/// Printable identity returned by the verified AFIK K5 application.
pub const APPLICATION_VERSION: &[u8] = b"AFIK-K5-1.8U";
/// Validated identity consumed by the shared application service.
pub const APPLICATION_IDENTITY: ApplicationIdentity<'static> =
    ApplicationIdentity::new(APPLICATION_VERSION).expect("K5 identity is printable and bounded");

/// Capabilities exposed by the current K5 application boundary.
///
/// K5 has the shared receive app, semantic-view query, and common logical-key
/// command boundary. It has no AFIK persistence or object protocol, and its
/// target-native metrics remain an adapter observation rather than a shared
/// serial capability.
pub const fn serial_capabilities() -> SerialCapabilities {
    SerialCapabilities::shared_receive()
}

/// Applies one already-decoded common serial request to the K5 application.
///
/// UART and circular-DMA framing remain in the target image. This adapter is
/// the point where a decoded common command enters the same shared event,
/// effect, and configuration-acknowledgement path used by keypad input.
pub fn apply_shared_serial_request(app: &mut ReceiveApp, request: SerialRequest) -> SerialResult {
    dispatch_shared(app, serial_capabilities(), request)
}

/// Plain-text banner retained for bounded serial diagnostics.
pub const BOOT_BANNER: &[u8] = b"AFIK-K5-1.8U booted";
