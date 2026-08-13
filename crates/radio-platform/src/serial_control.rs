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
}
