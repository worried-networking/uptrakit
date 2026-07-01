//! Runs the real check against the committed spec + client. Fails on drift.

use std::process::Command;

#[test]
fn client_tracks_openapi_spec_in_workspace() {
    // xtask/ -> workspace root (1 level up).
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    let status = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("openapi-client-check")
        .current_dir(workspace_root)
        .status()
        .expect("run xtask");
    assert!(status.success(), "openapi-client drift detected; run `cargo xtask openapi-client-check`");
}
