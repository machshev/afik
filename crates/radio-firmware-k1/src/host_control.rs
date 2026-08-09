//! Performing host runtime-control requests against the receive controller.
//!
//! The host is a second peer beside the keypad, not an owner of the radio.
//! Every request here reaches the same controller method the interface task
//! calls after decoding a key press, and produces the same
//! [`ReceiveUpdate`] for the caller to apply. There is no host mode, nothing to
//! release, and no way for a host to stop the operator's keypad working.
//!
//! This module is pure. It performs no input or output, holds no state, and
//! never touches the configuration store, so a host-driven tune is live state
//! on exactly the same terms as the operator's own.

use radio_channel_control::{
    BankedReceiveController, BankedScanPhase, ChannelSource, ReceiveError,
    ReceiveMode as TuningMode, ReceiveState, ReceiveUpdate,
};
use radio_device::ControlAnswer;
use radio_domain::Frequency;
use radio_protocol::{
    ControlRequest, DeviceErrorCode, ReceiveMetricsReport, ReceiveMode, ReceiveStateReport,
    ScanActivity,
};

/// The result of performing one host request.
pub struct Performed {
    /// The controller update to apply, absent when nothing changed.
    ///
    /// A query changes nothing and produces none. Anything which moves the
    /// receiver produces one, and it must be applied exactly as the equivalent
    /// key press's update is, or the radio's state and its hardware disagree.
    pub update: Option<ReceiveUpdate>,
    /// The answer to encode for the host.
    pub answer: ControlAnswer,
}

/// Describes what the receiver is doing, as the display would show it.
pub fn report<S: ChannelSource>(controller: &BankedReceiveController<S>) -> ReceiveStateReport {
    let selection = controller.setup();
    ReceiveStateReport {
        mode: match controller.mode() {
            TuningMode::Memory => ReceiveMode::Memory,
            TuningMode::Vfo => ReceiveMode::Vfo,
        },
        scan: match controller.state() {
            ReceiveState::Idle => ScanActivity::Idle,
            ReceiveState::Scanning(BankedScanPhase::Dwell) => ScanActivity::Dwell,
            ReceiveState::Scanning(BankedScanPhase::Hold) => ScanActivity::Hold,
        },
        bank: controller.bank().map(radio_domain::BankId::get),
        index: controller.index(),
        // A VFO frequency belongs to no channel, so it is reported as naming
        // none rather than as whichever channel the operator last left.
        channel_id: match controller.mode() {
            TuningMode::Memory => controller.channel().id().get(),
            TuningMode::Vfo => 0,
        },
        visible_channels: controller.visible_channels(),
        frequency_hz: selection.frequency.as_hz(),
    }
}

/// Performs one host request against the controller.
///
/// `metrics` is the most recent sample the interface task took, absent before
/// the receiver has produced one.
pub fn perform<S: ChannelSource>(
    controller: &mut BankedReceiveController<S>,
    request: ControlRequest,
    metrics: Option<ReceiveMetricsReport>,
) -> Performed {
    match request {
        ControlRequest::GetState => Performed {
            update: None,
            answer: ControlAnswer::State(report(controller)),
        },
        ControlRequest::GetMetrics => Performed {
            update: None,
            // A radio which has not sampled yet has nothing to report, and
            // reporting zeroes would be indistinguishable from a real reading
            // of zero.
            answer: metrics.map_or(
                ControlAnswer::Refused(DeviceErrorCode::InvalidState),
                ControlAnswer::Metrics,
            ),
        },
        ControlRequest::StopScan => settled(controller, BankedReceiveController::stop_scanning),
        ControlRequest::StartScan => settled(controller, BankedReceiveController::start_scanning),
        ControlRequest::EnterVfo => settled(controller, BankedReceiveController::enter_vfo),
        ControlRequest::EnterMemory => settled(controller, BankedReceiveController::enter_memory),
        ControlRequest::SelectChannel { index } => {
            settled(controller, |controller| controller.select(index))
        }
        ControlRequest::TuneTo { frequency_hz } => {
            let Ok(frequency) = Frequency::from_hz(frequency_hz) else {
                return Performed {
                    update: None,
                    answer: ControlAnswer::Refused(DeviceErrorCode::OutOfRange),
                };
            };
            settled(controller, |controller| controller.tune_to(frequency))
        }
    }
}

/// Runs one controller operation and reports the state it left behind.
///
/// A refused operation changes nothing, so the host is told why rather than
/// being handed a state report which would look like success.
fn settled<S, F>(controller: &mut BankedReceiveController<S>, operation: F) -> Performed
where
    S: ChannelSource,
    F: FnOnce(&mut BankedReceiveController<S>) -> Result<ReceiveUpdate, ReceiveError>,
{
    match operation(controller) {
        Ok(update) => Performed {
            update: Some(update),
            answer: ControlAnswer::State(report(controller)),
        },
        Err(error) => Performed {
            update: None,
            answer: ControlAnswer::Refused(refusal(error)),
        },
    }
}

/// Maps a controller refusal onto the stable wire code which describes it.
const fn refusal(error: ReceiveError) -> DeviceErrorCode {
    match error {
        // Well formed, implemented, and not possible from where the radio is.
        ReceiveError::InvalidState(_) | ReceiveError::InvalidMode(_) => {
            DeviceErrorCode::InvalidState
        }
        // The request named something the radio does not have.
        ReceiveError::NoEligibleChannel
        | ReceiveError::IndexOutOfRange
        | ReceiveError::TuningLimit => DeviceErrorCode::OutOfRange,
        // The radio's own bookkeeping failed, which is nothing the host did.
        ReceiveError::TimerTokenExhausted | ReceiveError::InvalidConfig(_) => {
            DeviceErrorCode::Internal
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{perform, report, Performed};
    use radio_channel_control::{
        BankedReceiveController, BankedScanPhase, ChannelMemory, ReceiveState,
    };
    use radio_channel_plan::{
        BankMask, ChannelDefinition, ChannelFlags, ChannelName, ChannelRecord,
    };
    use radio_device::ControlAnswer;
    use radio_domain::{
        Bandwidth, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel, RadioConfig,
        SquelchLevel, Tone, TxClass,
    };
    use radio_protocol::{ControlRequest, DeviceErrorCode, ReceiveMode, ScanActivity};

    const CHANNELS: usize = 4;

    fn channel(id: u16, hz: u32) -> ChannelRecord {
        let receive = Frequency::from_hz(hz).expect("frequency");
        ChannelRecord::new(ChannelDefinition {
            id: ChannelId::new(id),
            name: ChannelName::new("TEST").expect("name"),
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
            banks: BankMask::default(),
            tx_class: TxClass::Never,
        })
        .expect("channel")
    }

    fn controller() -> BankedReceiveController<ChannelMemory<CHANNELS>> {
        let mut memory = ChannelMemory::<CHANNELS>::new();
        for (offset, id) in (1..=3_u16).enumerate() {
            let hz = 145_000_000 + 12_500 * u32::try_from(offset).expect("offset");
            memory.insert(channel(id, hz)).expect("insert");
        }
        BankedReceiveController::activate(memory, RadioConfig::conservative(), None)
            .expect("activation")
            .0
    }

    #[test]
    fn a_query_reports_the_state_and_changes_nothing() {
        let mut controller = controller();
        let before = report(&controller);
        let Performed { update, answer } = perform(&mut controller, ControlRequest::GetState, None);

        assert!(update.is_none());
        assert_eq!(answer, ControlAnswer::State(before));
        assert_eq!(report(&controller), before);
    }

    #[test]
    fn a_host_tune_lands_where_the_operator_tuning_to_the_same_frequency_lands() {
        // The equivalence the whole design rests on: a host operation and the
        // keypad reaching the same controller method leave identical state.
        let frequency = Frequency::from_hz(145_512_500).expect("frequency");

        let mut by_key = controller();
        by_key.enter_vfo().expect("vfo");
        by_key.tune_to(frequency).expect("tuned");

        let mut by_host = controller();
        perform(&mut by_host, ControlRequest::EnterVfo, None);
        perform(
            &mut by_host,
            ControlRequest::TuneTo {
                frequency_hz: 145_512_500,
            },
            None,
        );

        assert_eq!(report(&by_host), report(&by_key));
        assert_eq!(report(&by_host).mode, ReceiveMode::Vfo);
        assert_eq!(report(&by_host).frequency_hz, 145_512_500);
    }

    #[test]
    fn a_host_scan_stop_lands_where_the_keypad_stopping_it_lands() {
        let mut by_key = controller();
        by_key.start_scanning().expect("scanning");
        by_key.stop_scanning().expect("stopped");

        let mut by_host = controller();
        perform(&mut by_host, ControlRequest::StartScan, None);
        perform(&mut by_host, ControlRequest::StopScan, None);

        assert_eq!(report(&by_host), report(&by_key));
        assert_eq!(report(&by_host).scan, ScanActivity::Idle);
    }

    #[test]
    fn a_running_scan_is_reported_as_running() {
        let mut controller = controller();
        let Performed { update, answer } =
            perform(&mut controller, ControlRequest::StartScan, None);

        assert!(update.is_some());
        assert_eq!(
            controller.state(),
            ReceiveState::Scanning(BankedScanPhase::Dwell)
        );
        assert_eq!(answer, ControlAnswer::State(report(&controller)));
        assert_eq!(report(&controller).scan, ScanActivity::Dwell);
    }

    #[test]
    fn tuning_a_memory_channel_is_refused_as_a_state_the_radio_is_not_in() {
        // The keypad cannot tune a memory channel either. The host is told why
        // rather than being silently moved into the VFO.
        let mut controller = controller();
        let Performed { update, answer } = perform(
            &mut controller,
            ControlRequest::TuneTo {
                frequency_hz: 145_512_500,
            },
            None,
        );

        assert!(update.is_none());
        assert_eq!(
            answer,
            ControlAnswer::Refused(DeviceErrorCode::InvalidState)
        );
        assert_eq!(report(&controller).mode, ReceiveMode::Memory);
    }

    #[test]
    fn a_frequency_the_domain_refuses_is_reported_as_out_of_range() {
        let mut controller = controller();
        perform(&mut controller, ControlRequest::EnterVfo, None);
        let Performed { update, answer } = perform(
            &mut controller,
            ControlRequest::TuneTo { frequency_hz: 0 },
            None,
        );

        assert!(update.is_none());
        assert_eq!(answer, ControlAnswer::Refused(DeviceErrorCode::OutOfRange));
    }

    #[test]
    fn a_frequency_no_receiver_could_reach_is_still_accepted_here() {
        // Recorded rather than asserted as desirable. `Frequency` refuses only
        // zero and the controller stores what it is given, so nothing in this
        // path knows the fitted receiver's band. A host sweeping outside it is
        // answered successfully and the tune fails later, at the driver. The
        // controller is the wrong place to invent a band limit, and the right
        // one is not yet evidenced for this board.
        let mut controller = controller();
        perform(&mut controller, ControlRequest::EnterVfo, None);
        let Performed { update, answer } = perform(
            &mut controller,
            ControlRequest::TuneTo {
                frequency_hz: u32::MAX,
            },
            None,
        );

        assert!(update.is_some());
        assert_eq!(answer, ControlAnswer::State(report(&controller)));
        assert_eq!(report(&controller).frequency_hz, u32::MAX);
    }

    #[test]
    fn a_channel_index_which_no_channel_occupies_is_refused() {
        let mut controller = controller();
        let Performed { update, answer } = perform(
            &mut controller,
            ControlRequest::SelectChannel { index: 900 },
            None,
        );

        assert!(update.is_none());
        assert_eq!(answer, ControlAnswer::Refused(DeviceErrorCode::OutOfRange));
    }

    #[test]
    fn metrics_are_refused_until_the_receiver_has_taken_a_sample() {
        let mut controller = controller();
        let Performed { answer, .. } = perform(&mut controller, ControlRequest::GetMetrics, None);

        assert_eq!(
            answer,
            ControlAnswer::Refused(DeviceErrorCode::InvalidState)
        );
    }
}
