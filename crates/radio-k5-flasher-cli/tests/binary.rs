//! Process-level smoke tests for stable K5 flasher help and usage behavior.

use std::process::Command;

#[test]
fn binary_help_version_and_missing_device_smoke() {
    let binary = env!("CARGO_BIN_EXE_afik-k5");
    let help = Command::new(binary).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert_eq!(help.stdout, radio_k5_flasher_cli::HELP.as_bytes());
    assert!(help.stderr.is_empty());

    let version = Command::new(binary).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert_eq!(version.stdout, b"afik-k5 0.1.0\n");
    assert!(version.stderr.is_empty());

    let probe = Command::new(binary).arg("probe").output().unwrap();
    assert_eq!(probe.status.code(), Some(2));
    assert!(probe.stdout.is_empty());
    assert!(probe.stderr.starts_with(b"error: "));
}
