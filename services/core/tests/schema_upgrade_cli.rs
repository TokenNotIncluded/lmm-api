//! Real CLI argument/input checks. No configuration or database is opened.
use std::{
    io::Write,
    process::{Command, Output, Stdio},
};
fn invoke(args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lmm-core"))
        .args(args)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if !input.is_empty() {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    child.wait_with_output().unwrap()
}
#[test]
fn upgrade_cli_requires_explicit_config_and_never_legacy_environment() {
    for args in [
        vec!["upgrade-db"],
        vec!["upgrade-db", "--legacy-env"],
        vec!["--legacy-env", "upgrade-db"],
    ] {
        let output = invoke(&args, b"");
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("upgrade-db requires explicit"));
        assert!(output.stdout.is_empty());
    }
}
#[test]
fn upgrade_cli_rejects_untrusted_json_before_reading_configuration() {
    let marker = "private-input-do-not-echo";
    for input in [
        format!("{{\"arbitrary_sql\":\"{marker}\"}}"),
        format!("{{\"password\":\"{marker}\""),
    ] {
        let output = invoke(
            &["upgrade-db", "--config", "/missing-must-not-be-read.toml"],
            input.as_bytes(),
        );
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("UPGRADE_DB_INPUT_INVALID"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(marker));
        assert!(output.stdout.is_empty());
    }
}
#[test]
fn upgrade_help_is_explicit_and_has_no_startup_side_effect() {
    let output = invoke(&["upgrade-db", "--help"], b"");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("Explicit offline compiled additive 25->26 upgrade"));
    assert!(
        help.contains("Legacy 24->25 trust-pricing requires the preserved 25-component installer.")
    );
}
