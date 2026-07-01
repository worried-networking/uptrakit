use std::process::Command;

fn fixture_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/audit_fixtures")
        .join(name)
}

#[test]
fn fails_on_missing_catalog_entry() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("audit-coverage-check")
        .current_dir(fixture_path("missing_entry"))
        .output()
        .map_err(|e| format!("failed to run binary: {e}"))
        .unwrap();
    assert!(
        !output.status.success(),
        "expected failure, got success. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("missing catalog entry"),
        "expected 'missing catalog entry' in stderr, got: {stderr}"
    );
}

#[test]
fn passes_when_clean() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("audit-coverage-check")
        .current_dir(fixture_path("clean"))
        .output()
        .map_err(|e| format!("failed to run binary: {e}"))
        .unwrap();
    assert!(
        output.status.success(),
        "expected success, got failure. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
