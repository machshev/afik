//! Application-facing display contract shared by target adapters.

/// A milestone the application can expose before its normal interface exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootStage {
    /// Reset reached Rust and RAM startup completed.
    Reset,
    /// Board clocks and pins were configured.
    BoardReady,
    /// The serial service is about to run.
    SerialReady,
}

/// Bounded serial receive evidence suitable for an on-screen diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceiveDiagnostic {
    /// Bytes removed from the board receive adapter.
    pub bytes: u32,
    /// Raw board UART status bits, retained for evidence rather than decoded.
    pub status: u32,
}

/// The display operations shared application behavior may request.
pub trait BootDisplay {
    /// Board-specific display failure.
    type Error;

    /// Replaces the screen with one bounded diagnostic stage.
    fn show(&mut self, stage: BootStage) -> Result<(), Self::Error>;

    /// Replaces the screen with bounded receive evidence.
    fn show_receive(&mut self, diagnostic: ReceiveDiagnostic) -> Result<(), Self::Error>;
}

/// Runs the target-independent boot presentation in its fixed order.
pub fn show_boot_sequence<D: BootDisplay>(display: &mut D) -> Result<(), D::Error> {
    display.show(BootStage::Reset)?;
    display.show(BootStage::BoardReady)?;
    display.show(BootStage::SerialReady)
}

#[cfg(test)]
mod tests {
    use super::{show_boot_sequence, BootDisplay, BootStage, ReceiveDiagnostic};

    #[derive(Default)]
    struct RecordingDisplay {
        stages: [Option<BootStage>; 3],
        length: usize,
    }

    impl BootDisplay for RecordingDisplay {
        type Error = ();

        fn show(&mut self, stage: BootStage) -> Result<(), Self::Error> {
            self.stages[self.length] = Some(stage);
            self.length += 1;
            Ok(())
        }

        fn show_receive(&mut self, _diagnostic: ReceiveDiagnostic) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[test]
    fn application_boot_behavior_has_one_order() {
        let mut display = RecordingDisplay::default();
        show_boot_sequence(&mut display).unwrap();
        assert_eq!(
            display.stages,
            [
                Some(BootStage::Reset),
                Some(BootStage::BoardReady),
                Some(BootStage::SerialReady),
            ]
        );
    }
}
