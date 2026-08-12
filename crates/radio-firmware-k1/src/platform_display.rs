//! K1 adapter for the shared application-facing boot display contract.

use crate::display::{
    render_boot_stage, render_receive_diagnostic, write_frame, DisplayBus, FRAME_BYTES,
};
use radio_platform::display::{BootDisplay, BootStage, ReceiveDiagnostic};

/// Presents shared boot requests through the K1's existing display bus.
pub struct K1BootDisplay<B> {
    bus: B,
    frame: [u8; FRAME_BYTES],
}

impl<B> K1BootDisplay<B> {
    /// Wraps a target-specific K1 display bus.
    pub const fn new(bus: B) -> Self {
        Self {
            bus,
            frame: [0; FRAME_BYTES],
        }
    }

    /// Returns the target bus after the application service is finished.
    pub fn into_inner(self) -> B {
        self.bus
    }
}

impl<B: DisplayBus> BootDisplay for K1BootDisplay<B> {
    type Error = B::Error;

    fn show(&mut self, stage: BootStage) -> Result<(), Self::Error> {
        render_boot_stage(&mut self.frame, stage);
        write_frame(&mut self.bus, &self.frame)
    }

    fn show_receive(&mut self, diagnostic: ReceiveDiagnostic) -> Result<(), Self::Error> {
        render_receive_diagnostic(&mut self.frame, diagnostic);
        write_frame(&mut self.bus, &self.frame)
    }
}

#[cfg(test)]
mod tests {
    use super::K1BootDisplay;
    use crate::display::{DisplayBus, TransferKind};
    use radio_platform::display::{show_boot_sequence, BootDisplay, ReceiveDiagnostic};

    #[derive(Default)]
    struct CountingBus {
        data_transfers: usize,
    }

    impl DisplayBus for CountingBus {
        type Error = ();

        fn write(&mut self, kind: TransferKind, _bytes: &[u8]) -> Result<(), Self::Error> {
            if kind == TransferKind::Data {
                self.data_transfers += 1;
            }
            Ok(())
        }

        fn delay_ms(&mut self, _milliseconds: u8) {}
    }

    #[test]
    fn k1_adapter_runs_the_shared_boot_behavior() {
        let mut display = K1BootDisplay::new(CountingBus::default());
        show_boot_sequence(&mut display).unwrap();
        display
            .show_receive(ReceiveDiagnostic {
                bytes: 16,
                status: 0,
            })
            .unwrap();
        assert_eq!(display.into_inner().data_transfers, 4 * 8);
    }
}
