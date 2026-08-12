//! Application-facing boot diagnostics, independent of either target MCU.

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

/// The only display operation the bring-up application depends on.
///
/// A K1 or K5 board adapter may implement this without exposing its GPIO, SPI,
/// controller commands, framebuffer layout, or timing to application code.
pub trait BootDisplay {
    /// Board-specific display failure.
    type Error;

    /// Replaces the screen with one bounded diagnostic stage.
    fn show(&mut self, stage: BootStage) -> Result<(), Self::Error>;

    /// Replaces the screen with bounded receive evidence.
    fn show_receive(&mut self, diagnostic: ReceiveDiagnostic) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::{BootDisplay, BootStage, ReceiveDiagnostic};

    #[derive(Default)]
    struct RecordingDisplay {
        stages: [Option<BootStage>; 3],
        length: usize,
        diagnostic: Option<ReceiveDiagnostic>,
    }

    impl BootDisplay for RecordingDisplay {
        type Error = ();

        fn show(&mut self, stage: BootStage) -> Result<(), Self::Error> {
            self.stages[self.length] = Some(stage);
            self.length += 1;
            Ok(())
        }

        fn show_receive(&mut self, diagnostic: ReceiveDiagnostic) -> Result<(), Self::Error> {
            self.diagnostic = Some(diagnostic);
            Ok(())
        }
    }

    #[test]
    fn one_interface_carries_the_whole_boot_sequence() {
        let mut display = RecordingDisplay::default();
        display.show(BootStage::Reset).unwrap();
        display.show(BootStage::BoardReady).unwrap();
        display.show(BootStage::SerialReady).unwrap();
        assert_eq!(
            display.stages,
            [
                Some(BootStage::Reset),
                Some(BootStage::BoardReady),
                Some(BootStage::SerialReady)
            ]
        );
        let diagnostic = ReceiveDiagnostic {
            bytes: 14,
            status: 0x410,
        };
        display.show_receive(diagnostic).unwrap();
        assert_eq!(display.diagnostic, Some(diagnostic));
    }
}
