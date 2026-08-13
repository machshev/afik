//! Transport-free serial control at the shared application boundary.
//!
//! Target serial tasks decode their own wire protocol and retain ownership of
//! UART, DMA, framing, persistence, and target-only commands.  Once a request
//! is known to be a common application request, the target hands it here.  The
//! resulting effects are therefore identical to effects from the keypad, and
//! a configuration change still follows the normal exact acknowledgement path.

use super::configuration_change::ConfigurationCandidate;
use super::receive_app::{Effects, Event, Key, ReceiveApp, View};

/// One capability a target may advertise at its serial boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Capability {
    /// The target runs the shared receive application.
    SharedReceive = 0,
    /// The target can answer a shared semantic-view query.
    ViewQuery = 1,
    /// The target accepts common logical-key commands.
    KeyCommands = 2,
    /// The target retains AFIK configuration across reset.
    PersistentConfiguration = 3,
    /// The target exposes the object-level configuration service.
    ObjectConfiguration = 4,
    /// The target exposes channels outside the common PMR446 examples.
    ArbitraryChannels = 5,
    /// The target exposes a tunable VFO adapter.
    Vfo = 6,
    /// The target exposes scanning through its own receive controller.
    Scanning = 7,
    /// The target exposes target-native raw receive metrics.
    RawMetrics = 8,
}

/// A bounded set of serial capabilities.
///
/// This is deliberately only a bitset.  It does not contain a storage size,
/// protocol type, UART handle, or target identity.  K1 and K5 construct their
/// sets in their own adapters because their retained configuration and
/// target-only controls are not interchangeable.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Capabilities(u16);

impl Capabilities {
    /// Creates a set from its adapter-owned bit representation.
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    /// Returns the raw bounded bit representation for a target-facing adapter.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Returns a set containing only the common receive/query/key boundary.
    pub const fn shared_receive() -> Self {
        Self::from_bits(
            Self::bit(Capability::SharedReceive)
                | Self::bit(Capability::ViewQuery)
                | Self::bit(Capability::KeyCommands),
        )
    }

    /// Creates the shared view boundary without claiming target key ownership.
    ///
    /// This is used by adapters whose physical controller lives in another
    /// task or owns a richer selection state than the common receive app.
    pub const fn shared_view() -> Self {
        Self::from_bits(Self::bit(Capability::SharedReceive) | Self::bit(Capability::ViewQuery))
    }

    /// Adds one capability to this set.
    #[must_use]
    pub const fn with(self, capability: Capability) -> Self {
        Self::from_bits(self.0 | Self::bit(capability))
    }

    /// Returns whether a capability is present.
    pub const fn contains(self, capability: Capability) -> bool {
        self.0 & Self::bit(capability) != 0
    }

    const fn bit(capability: Capability) -> u16 {
        1_u16 << capability as u8
    }
}

/// A query whose answer is owned by the shared application boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Query {
    /// Read the semantic receive view currently shown by a target adapter.
    View,
    /// Read the target's explicitly declared boundary capabilities.
    Capabilities,
}

/// A command which can enter the shared application event path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    /// Apply the receive meaning of one stable logical key.
    Key(Key),
    /// Toggle application audio without using the operator Menu key.
    ToggleAudio,
    /// Ask the target to apply one candidate, then acknowledge it through the
    /// same exact change identity used by keypad-driven activation.
    Activate(ConfigurationCandidate),
}

/// One decoded target request after its target-owned wire framing is removed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Request {
    /// Read-only shared application information.
    Query(Query),
    /// Apply one common application command.
    Command(Command),
}

/// One bounded result for a shared serial request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Result {
    /// Semantic view returned by a query.
    View(View),
    /// Capability set returned by a query.
    Capabilities(Capabilities),
    /// Ordered effects to apply through the target adapter.
    Effects(Effects),
}

/// The bounded common request vocabulary which target wire adapters may
/// encode.  Configuration candidates are deliberately absent: their exact
/// acknowledgement and persistence semantics remain target-owned.
pub mod wire {
    use super::{Capabilities, Command, Key, Query, Result as DispatchResult, View};

    /// Logical keys representable on the common serial wire. Menu is omitted:
    /// it belongs to the operator shell and cannot be an audio shortcut.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum KeyCode {
        /// Upper side key.
        Side1,
        /// Lower side monitor key.
        Side2,
        /// Push-to-talk input label.
        Ptt,
        /// Previous-channel key.
        Up,
        /// Next-channel key.
        Down,
        /// Shell back key.
        Exit,
        /// Star key.
        Star,
        /// Function key.
        Function,
    }

    impl KeyCode {
        const fn into_shared(self) -> Key {
            match self {
                Self::Side1 => Key::Side1,
                Self::Side2 => Key::Side2,
                Self::Ptt => Key::Ptt,
                Self::Up => Key::Up,
                Self::Down => Key::Down,
                Self::Exit => Key::Exit,
                Self::Star => Key::Star,
                Self::Function => Key::Function,
            }
        }
    }

    /// Bytes in one common request payload.
    pub const REQUEST_BYTES: usize = 2;
    /// Maximum bytes in one common response payload.
    pub const MAX_RESPONSE_BYTES: usize = 4;

    const OP_VIEW: u8 = 0x01;
    const OP_CAPABILITIES: u8 = 0x02;
    const OP_KEY: u8 = 0x03;
    const OP_TOGGLE_AUDIO: u8 = 0x04;

    const RESPONSE_VIEW: u8 = 0x81;
    const RESPONSE_CAPABILITIES: u8 = 0x82;
    const RESPONSE_KEY: u8 = 0x83;

    /// A request representable on the common wire boundary.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum Request {
        /// Query the semantic receive view.
        View,
        /// Query the adapter-declared shared capability bits.
        Capabilities,
        /// Apply one canonical logical key through the shared app.
        Key(KeyCode),
        /// Toggle application audio explicitly.
        ToggleAudio,
    }

    /// A response representable on the common wire boundary.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum Response {
        /// Semantic view returned by a view query or accepted key command.
        View(View),
        /// Explicit target capability bits.
        Capabilities(Capabilities),
    }

    /// Failure while decoding or encoding the fixed common payload.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum Error {
        /// The payload was not exactly the fixed request length.
        InvalidRequestLength,
        /// The operation or key code is not in the bounded vocabulary.
        UnknownCode,
        /// The response variant cannot be represented by the common wire.
        UnsupportedResult,
        /// The caller's response buffer is too short.
        ResponseTooSmall,
    }

    /// Encodes a common request into its fixed two-byte payload.
    pub const fn encode_request(request: Request, output: &mut [u8; REQUEST_BYTES]) {
        output[0] = match request {
            Request::View => OP_VIEW,
            Request::Capabilities => OP_CAPABILITIES,
            Request::Key(_) => OP_KEY,
            Request::ToggleAudio => OP_TOGGLE_AUDIO,
        };
        output[1] = match request {
            Request::Key(key) => encode_key(key),
            Request::View | Request::Capabilities | Request::ToggleAudio => 0,
        };
    }

    /// Decodes the fixed common request payload and maps it to the shared
    /// transport-free contract.
    pub const fn decode_request(input: &[u8]) -> core::result::Result<Request, Error> {
        if input.len() != REQUEST_BYTES {
            return Err(Error::InvalidRequestLength);
        }
        match input[0] {
            OP_VIEW if input[1] == 0 => Ok(Request::View),
            OP_CAPABILITIES if input[1] == 0 => Ok(Request::Capabilities),
            OP_TOGGLE_AUDIO if input[1] == 0 => Ok(Request::ToggleAudio),
            OP_KEY => match decode_key(input[1]) {
                Some(key) => Ok(Request::Key(key)),
                None => Err(Error::UnknownCode),
            },
            _ => Err(Error::UnknownCode),
        }
    }

    /// Encodes a shared dispatch result. `Effects` are not wire results: the
    /// target must apply them first and answer with the resulting view.
    pub fn encode_response(
        result: DispatchResult,
        output: &mut [u8],
    ) -> core::result::Result<usize, Error> {
        match result {
            DispatchResult::View(view) => {
                let bytes = output.get_mut(..4).ok_or(Error::ResponseTooSmall)?;
                bytes[0] = RESPONSE_VIEW;
                encode_view(view, &mut bytes[1..4]);
                Ok(4)
            }
            DispatchResult::Capabilities(capabilities) => {
                let bytes = output.get_mut(..3).ok_or(Error::ResponseTooSmall)?;
                bytes[0] = RESPONSE_CAPABILITIES;
                bytes[1..3].copy_from_slice(&capabilities.bits().to_le_bytes());
                Ok(3)
            }
            DispatchResult::Effects(_) => Err(Error::UnsupportedResult),
        }
    }

    /// Encodes the view returned after a target applies a logical key.
    pub fn encode_key_response(
        view: View,
        output: &mut [u8],
    ) -> core::result::Result<usize, Error> {
        let bytes = output.get_mut(..4).ok_or(Error::ResponseTooSmall)?;
        bytes[0] = RESPONSE_KEY;
        encode_view(view, &mut bytes[1..4]);
        Ok(4)
    }

    /// Decodes a response payload for deterministic host/simulator fixtures.
    pub fn decode_response(input: &[u8]) -> core::result::Result<Response, Error> {
        match input {
            [RESPONSE_VIEW | RESPONSE_KEY, channel, audio, flags] => {
                decode_view(*channel, *audio, *flags)
                    .map(Response::View)
                    .ok_or(Error::UnknownCode)
            }
            [RESPONSE_CAPABILITIES, low, high] => {
                let bits = u16::from_le_bytes([*low, *high]);
                if bits & !0x01FF != 0 {
                    return Err(Error::UnknownCode);
                }
                Ok(Response::Capabilities(Capabilities::from_bits(bits)))
            }
            _ => Err(Error::UnknownCode),
        }
    }

    /// Maps a wire request to the transport-free shared request.
    pub const fn into_request(request: Request) -> super::Request {
        match request {
            Request::View => super::Request::Query(Query::View),
            Request::Capabilities => super::Request::Query(Query::Capabilities),
            Request::Key(key) => super::Request::Command(Command::Key(key.into_shared())),
            Request::ToggleAudio => super::Request::Command(Command::ToggleAudio),
        }
    }

    /// Encodes the three semantic view flags into one bounded byte.
    const fn encode_view(view: View, output: &mut [u8]) {
        output[0] = view.channel;
        output[1] = if view.audio { 1 } else { 0 };
        output[2] = (if view.squelch_open { 1 } else { 0 })
            | ((if view.receiver_ok { 1 } else { 0 }) << 1)
            | ((if view.monitoring { 1 } else { 0 }) << 2);
    }

    const fn decode_view(channel: u8, audio: u8, flags: u8) -> Option<View> {
        if audio > 1 || flags & !0x07 != 0 {
            return None;
        }
        Some(View {
            channel,
            audio: audio != 0,
            monitoring: flags & 4 != 0,
            squelch_open: flags & 1 != 0,
            receiver_ok: flags & 2 != 0,
        })
    }

    const fn encode_key(key: KeyCode) -> u8 {
        match key {
            KeyCode::Side1 => 0,
            KeyCode::Side2 => 1,
            KeyCode::Ptt => 2,
            KeyCode::Up => 4,
            KeyCode::Down => 5,
            KeyCode::Exit => 6,
            KeyCode::Star => 0x1A,
            KeyCode::Function => 0x1B,
        }
    }

    const fn decode_key(code: u8) -> Option<KeyCode> {
        Some(match code {
            0 => KeyCode::Side1,
            1 => KeyCode::Side2,
            2 => KeyCode::Ptt,
            4 => KeyCode::Up,
            5 => KeyCode::Down,
            6 => KeyCode::Exit,
            0x1A => KeyCode::Star,
            0x1B => KeyCode::Function,
            _ => return None,
        })
    }
}

/// Dispatches one decoded request into the shared application.
///
/// This function performs no transport I/O and does not apply target effects.
/// For [`Command::Activate`], the returned effects include the identified
/// [`Effect::ConfigurationChange`].  The target must perform those effects and
/// feed its applied/rejected acknowledgement back as
/// [`Event::ConfigurationAcknowledged`], exactly as it does for keypad input.
pub fn dispatch(app: &mut ReceiveApp, capabilities: Capabilities, request: Request) -> Result {
    match request {
        Request::Query(Query::View) => Result::View(app.view()),
        Request::Query(Query::Capabilities) => Result::Capabilities(capabilities),
        Request::Command(Command::Key(key)) => Result::Effects(app.apply(Event::KeyPress(key))),
        Request::Command(Command::ToggleAudio) => Result::Effects(app.apply(Event::ToggleAudio)),
        Request::Command(Command::Activate(candidate)) => {
            Result::Effects(app.apply(Event::ConfigurationCandidate(candidate)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{dispatch, Capabilities, Capability, Command, Query, Request, Result};
    use crate::configuration_change::{ConfigurationAcknowledgement, ConfigurationCandidate};
    use crate::receive_app::{Effect, Event, Key, ReceiveApp};

    #[test]
    fn simulator_key_command_has_the_same_effects_as_keypad_input() {
        let capabilities = Capabilities::shared_receive();
        let mut serial = ReceiveApp::new();
        let mut keypad = ReceiveApp::new();

        assert_eq!(
            dispatch(
                &mut serial,
                capabilities,
                Request::Command(Command::Key(Key::Up)),
            ),
            Result::Effects(keypad.apply(Event::KeyPress(Key::Up)))
        );
        assert_eq!(serial.view(), keypad.view());
    }

    #[test]
    fn host_activation_uses_the_same_candidate_acknowledgement_boundary() {
        let mut app = ReceiveApp::new();
        let candidate = ConfigurationCandidate::from_channel(0, 8, false).expect("candidate");
        let Result::Effects(request) = dispatch(
            &mut app,
            Capabilities::shared_receive(),
            Request::Command(Command::Activate(candidate)),
        ) else {
            panic!("activation is an effect-producing command");
        };
        let change = request
            .iter()
            .find_map(|effect| match effect {
                Effect::ConfigurationChange(change) => Some(change),
                _ => None,
            })
            .expect("identified change");
        assert_eq!(app.view().channel, 1);
        let applied = app.apply(Event::ConfigurationAcknowledged(
            ConfigurationAcknowledgement::applied(change),
        ));
        assert_eq!(app.view().channel, 8);
        assert!(applied.iter().any(|effect| {
            matches!(effect, Effect::ConfigurationAcknowledged(ack) if ack.is_applied())
        }));
    }

    #[test]
    fn k5_receive_only_capabilities_do_not_claim_k1_storage_or_controls() {
        let k5 = Capabilities::shared_receive();
        assert!(k5.contains(Capability::SharedReceive));
        assert!(k5.contains(Capability::ViewQuery));
        assert!(k5.contains(Capability::KeyCommands));
        for capability in [
            Capability::PersistentConfiguration,
            Capability::ObjectConfiguration,
            Capability::ArbitraryChannels,
            Capability::Vfo,
            Capability::Scanning,
            Capability::RawMetrics,
        ] {
            assert!(!k5.contains(capability));
        }
    }

    #[test]
    fn capability_query_is_read_only_and_preserves_the_app() {
        let mut app = ReceiveApp::new();
        let before = app.view();
        let capabilities = Capabilities::shared_receive().with(Capability::RawMetrics);
        assert_eq!(
            dispatch(&mut app, capabilities, Request::Query(Query::Capabilities),),
            Result::Capabilities(capabilities)
        );
        assert_eq!(
            dispatch(&mut app, capabilities, Request::Query(Query::View)),
            Result::View(before)
        );
    }

    #[test]
    fn common_wire_requests_and_responses_are_fixed_and_deterministic() {
        use super::wire::{
            decode_request, decode_response, encode_key_response, encode_request, encode_response,
            KeyCode, Request as WireRequest, Response as WireResponse, REQUEST_BYTES,
        };
        use crate::receive_app::View;

        let requests = [
            (WireRequest::View, [0x01, 0x00]),
            (WireRequest::Capabilities, [0x02, 0x00]),
            (WireRequest::Key(KeyCode::Up), [0x03, 0x04]),
            (WireRequest::ToggleAudio, [0x04, 0x00]),
        ];
        for (request, expected) in requests {
            let mut encoded = [0_u8; REQUEST_BYTES];
            encode_request(request, &mut encoded);
            assert_eq!(encoded, expected);
            assert_eq!(decode_request(&encoded), Ok(request));
        }

        let view = View {
            channel: 7,
            audio: true,
            monitoring: false,
            squelch_open: false,
            receiver_ok: true,
        };
        let mut response = [0_u8; super::wire::MAX_RESPONSE_BYTES];
        let length = encode_response(Result::View(view), &mut response).expect("view response");
        assert_eq!(&response[..length], &[0x81, 7, 1, 2]);
        assert_eq!(
            decode_response(&response[..length]),
            Ok(WireResponse::View(view))
        );

        let capabilities = Capabilities::shared_receive();
        let length = encode_response(Result::Capabilities(capabilities), &mut response)
            .expect("capability response");
        assert_eq!(&response[..length], &[0x82, 7, 0]);
        assert_eq!(
            decode_response(&response[..length]),
            Ok(WireResponse::Capabilities(capabilities))
        );

        let length = encode_key_response(view, &mut response).expect("key response");
        assert_eq!(&response[..length], &[0x83, 7, 1, 2]);
        assert_eq!(
            decode_response(&response[..length]),
            Ok(WireResponse::View(view))
        );
    }

    #[test]
    fn common_wire_rejects_candidates_and_malformed_key_payloads() {
        use super::wire::{decode_request, encode_response, Error};
        use crate::configuration_change::ConfigurationCandidate;

        assert_eq!(decode_request(&[0x03, 0xFF]), Err(Error::UnknownCode));
        assert_eq!(
            decode_request(&[0x03, 3]),
            Err(Error::UnknownCode),
            "Menu is shell-owned and has no common wire encoding"
        );
        assert_eq!(
            decode_request(&[0x03, 0x10]),
            Err(Error::UnknownCode),
            "unsupported digit key cannot enter the common wire domain"
        );
        assert_eq!(decode_request(&[0x01]), Err(Error::InvalidRequestLength));
        assert_eq!(
            super::wire::decode_response(&[0x81, 1, 2, 0]),
            Err(Error::UnknownCode)
        );
        assert_eq!(
            super::wire::decode_response(&[0x82, 0, 2]),
            Err(Error::UnknownCode)
        );
        let candidate = ConfigurationCandidate::from_channel(0, 2, true).expect("candidate");
        let effects = dispatch(
            &mut ReceiveApp::new(),
            Capabilities::shared_receive(),
            Request::Command(Command::Activate(candidate)),
        );
        assert_eq!(
            encode_response(effects, &mut [0_u8; super::wire::MAX_RESPONSE_BYTES]),
            Err(Error::UnsupportedResult)
        );
    }
}
