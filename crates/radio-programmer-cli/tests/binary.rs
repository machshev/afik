//! Process-level smoke tests for stable CLI output and exit status.

use std::process::Command;

#[test]
fn binary_help_version_and_usage_smoke() {
    let binary = env!("CARGO_BIN_EXE_afik-programmer");
    let help = Command::new(binary).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert_eq!(help.stdout, radio_programmer_cli::HELP.as_bytes());
    assert!(help.stderr.is_empty());

    let version = Command::new(binary).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert_eq!(version.stdout, b"afik-programmer 0.1.0\n");

    let info = Command::new(binary)
        .args(["--sim", "info"])
        .output()
        .unwrap();
    assert!(info.status.success());
    assert!(info.stdout.starts_with(b"protocol_version=1\n"));
    assert!(info.stderr.is_empty());

    let invalid = Command::new(binary).arg("info").output().unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(invalid.stdout.is_empty());
    assert!(invalid.stderr.starts_with(b"error: "));
}
