//! Host contract proof for the K5 application-facing display adapter.

use radio_firmware_k5::k5_display::K5BootDisplay;
use radio_platform::display::{show_boot_sequence, BootDisplay, BootStage, ReceiveDiagnostic};

#[derive(Default)]
struct K5AdapterContract {
    stages: [Option<BootStage>; 3],
    length: usize,
    receive: Option<ReceiveDiagnostic>,
}

impl BootDisplay for K5AdapterContract {
    type Error = ();

    fn show(&mut self, stage: BootStage) -> Result<(), Self::Error> {
        self.stages[self.length] = Some(stage);
        self.length += 1;
        Ok(())
    }

    fn show_receive(&mut self, diagnostic: ReceiveDiagnostic) -> Result<(), Self::Error> {
        self.receive = Some(diagnostic);
        Ok(())
    }
}

#[test]
fn k5_adapter_runs_the_shared_boot_behavior() {
    fn implements_shared_contract<D: BootDisplay>() {}
    implements_shared_contract::<K5BootDisplay>();

    let mut display = K5AdapterContract::default();
    show_boot_sequence(&mut display).unwrap();
    let diagnostic = ReceiveDiagnostic {
        bytes: 16,
        status: 0,
    };
    display.show_receive(diagnostic).unwrap();
    assert_eq!(display.length, 3);
    assert_eq!(display.receive, Some(diagnostic));
}
