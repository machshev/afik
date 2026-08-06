//! Process-level smoke tests for the GUI launcher contract.

use std::process::Command;

#[test]
fn binary_help_version_and_non_loopback_rejection_smoke() {
    let binary = env!("CARGO_BIN_EXE_afik-programmer-gui");
    let help = Command::new(binary).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert_eq!(help.stdout, radio_programmer_gui::HELP.as_bytes());
    assert!(help.stderr.is_empty());

    let version = Command::new(binary).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert_eq!(version.stdout, b"afik-programmer-gui 0.1.0\n");
    assert!(version.stderr.is_empty());

    let invalid = Command::new(binary)
        .args(["--sim", "--listen", "0.0.0.0:9000"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(invalid.stdout.is_empty());
    assert_eq!(
        invalid.stderr,
        b"error: listen address must be loopback: 0.0.0.0:9000\n"
    );
}
