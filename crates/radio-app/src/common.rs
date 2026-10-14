//! Common receive-application requests carried inside the object protocol.
//!
//! The request arrived through `DeviceService`, so framing, flags, sequence
//! replay, and error responses remain owned by `radio-device`. This module only
//! decodes the bounded common payload and answers view-only queries. Key and
//! audio mutations are refused, because the banked controller and the operator
//! shell own those and a detached shared instance must not be changed instead.

use radio_device::ControlAnswer;
use radio_platform::receive_app::ReceiveApp;
use radio_platform::serial_control::wire as shared_wire;
use radio_platform::serial_control::{dispatch, Result as SharedResult};
use radio_protocol::DeviceErrorCode;

use crate::host_control::serial_capabilities;

/// Applies one decoded common request, refusing key and audio mutations.
pub fn apply_common_request(
    app: &mut ReceiveApp,
    request: shared_wire::Request,
) -> Result<SharedResult, DeviceErrorCode> {
    if matches!(
        request,
        shared_wire::Request::Key(_) | shared_wire::Request::ToggleAudio
    ) {
        return Err(DeviceErrorCode::UnsupportedCommand);
    }
    Ok(dispatch(
        app,
        serial_capabilities(),
        shared_wire::into_request(request),
    ))
}

/// Answers one common payload against the shared receive view, if one is
/// active.
pub(crate) fn answer(payload: [u8; 2], app: Option<&mut ReceiveApp>) -> ControlAnswer {
    let Ok(request) = shared_wire::decode_request(&payload) else {
        return ControlAnswer::Refused(DeviceErrorCode::MalformedPayload);
    };
    if request == shared_wire::Request::Capabilities {
        return encode(dispatch(
            &mut ReceiveApp::new(),
            serial_capabilities(),
            shared_wire::into_request(request),
        ));
    }
    let Some(app) = app else {
        return ControlAnswer::Refused(DeviceErrorCode::InvalidState);
    };
    match apply_common_request(app, request) {
        Ok(result) if request == shared_wire::Request::View => encode(result),
        Ok(_) => ControlAnswer::Refused(DeviceErrorCode::Internal),
        Err(error) => ControlAnswer::Refused(error),
    }
}

/// Encodes a query result into the runtime-control response payload.
fn encode(result: SharedResult) -> ControlAnswer {
    let mut bytes = [0_u8; 4];
    let Ok(length) = shared_wire::encode_response(result, &mut bytes) else {
        return ControlAnswer::Refused(DeviceErrorCode::Internal);
    };
    ControlAnswer::Common {
        payload: bytes,
        length: u8::try_from(length).unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::{answer, apply_common_request};
    use radio_device::ControlAnswer;
    use radio_platform::receive_app::ReceiveApp;
    use radio_platform::serial_control::wire as shared_wire;
    use radio_platform::serial_control::Capability;
    use radio_protocol::DeviceErrorCode;

    #[test]
    fn common_wire_key_is_decoded_but_capability_is_view_only() {
        let request = shared_wire::decode_request(&[0x03, 0x04]).expect("common key");
        assert_eq!(request, shared_wire::Request::Key(shared_wire::KeyCode::Up));
        assert!(!crate::host_control::serial_capabilities().contains(Capability::KeyCommands));
    }

    #[test]
    fn common_key_and_audio_requests_are_refused_without_app_mutation() {
        let mut app = ReceiveApp::new();
        let before = app.view();
        for payload in [[0x03, 0x01], [0x03, 0x04], [0x03, 0x05], [0x04, 0x00]] {
            let request = shared_wire::decode_request(&payload).expect("bounded common request");
            assert_eq!(
                apply_common_request(&mut app, request),
                Err(DeviceErrorCode::UnsupportedCommand)
            );
            assert_eq!(app.view(), before);
        }
    }

    #[test]
    fn malformed_or_unknown_common_payloads_are_rejected_without_app_mutation() {
        let mut app = ReceiveApp::new();
        let before = app.view();
        assert!(shared_wire::decode_request(&[0x03]).is_err());
        assert!(shared_wire::decode_request(&[0x03, 0xFF]).is_err());
        assert!(shared_wire::decode_request(&[0x01, 0x01]).is_err());
        assert_eq!(
            answer([0x01, 0x01], Some(&mut app)),
            ControlAnswer::Refused(DeviceErrorCode::MalformedPayload)
        );
        assert_eq!(app.view(), before);
        assert_eq!(shared_wire::REQUEST_BYTES, 2);
    }

    #[test]
    fn capabilities_answer_without_an_active_view_but_the_view_needs_one() {
        assert!(matches!(
            answer([0x02, 0x00], None),
            ControlAnswer::Common { .. }
        ));
        assert_eq!(
            answer([0x01, 0x00], None),
            ControlAnswer::Refused(DeviceErrorCode::InvalidState)
        );
    }
}
