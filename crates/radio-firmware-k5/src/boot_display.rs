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

/// The only display operation the bring-up application depends on.
///
/// A K1 or K5 board adapter may implement this without exposing its GPIO, SPI,
/// controller commands, framebuffer layout, or timing to application code.
pub trait BootDisplay {
    /// Board-specific display failure.
    type Error;

    /// Replaces the screen with one bounded diagnostic stage.
    fn show(&mut self, stage: BootStage) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::{BootDisplay, BootStage};

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
    }
}
