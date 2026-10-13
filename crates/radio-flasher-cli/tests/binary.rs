//! Process-level smoke tests for stable flasher help and usage behavior.

use std::process::Command;

#[test]
fn binary_help_version_and_usage_smoke() {
    let binary = env!("CARGO_BIN_EXE_afik-flasher");
    let help = Command::new(binary).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert_eq!(help.stdout, radio_flasher_cli::HELP.as_bytes());
    assert!(help.stderr.is_empty());

    let version = Command::new(binary).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert_eq!(
        version.stdout,
        concat!("afik-flasher ", env!("CARGO_PKG_VERSION"), "\n").as_bytes()
    );
    assert!(version.stderr.is_empty());

    let unknown = Command::new(binary).arg("probe").output().unwrap();
    assert_eq!(unknown.status.code(), Some(2));
    assert!(unknown.stdout.is_empty());
    assert!(unknown.stderr.starts_with(b"error: "));
}
