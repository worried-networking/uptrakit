//! Release-plz config invariants. Catches drift that would block GitHub
//! release uploads for binaries or re-introduce the `git_only + publish`
//! wedge that stalled releases since v0.0.2.
//!
//! Complementary to the per-publishable-crate blacklist tests in
//! `crates/shared/{service-sdk,openapi-client}/tests/no_workspace_db_deps.rs`
//! — those guard the cargo resolve graph; this file guards the
//! release-plz.toml config the resolve graph is meaningless without.

#![expect(
    clippy::expect_used,
    reason = "test assertions: panic on parse failure is the desired outcome \
              — this file's whole purpose is to fail loudly on malformed \
              release-plz.toml"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

const BINARY_TARGETS: &[&str] = &[
    "uptrakit-controller",
    "uptrakit-controller-standalone",
    "uptrakit-agent",
    "uptrakit-agent-ssh",
    "uptrakit-mqtt",
    "uptrakit-scheduler",
    "uptrakit-cli",
];

#[derive(Debug, Deserialize)]
struct ReleasePlz {
    #[serde(default)]
    package: Vec<RpPackage>,
}

#[derive(Debug, Deserialize)]
struct RpPackage {
    name: String,
    #[serde(default)]
    publish: Option<bool>,
    #[serde(default)]
    git_only: Option<bool>,
    #[serde(default)]
    git_release_enable: Option<bool>,
    #[serde(default)]
    git_tag_enable: Option<bool>,
    #[serde(default)]
    changelog_update: Option<bool>,
}

fn workspace_root() -> PathBuf {
    // functional-tests lives at crates/core/functional-tests; walk up three
    // levels to reach the workspace root. Moving the crate would be a
    // workspace-level refactor that also updates `[workspace] members` in
    // the root Cargo.toml.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..3 {
        p.pop();
    }
    p
}

fn load_release_plz() -> ReleasePlz {
    let body = std::fs::read_to_string(workspace_root().join("release-plz.toml"))
        .expect("release-plz.toml");
    toml::from_str(&body).expect("release-plz.toml parse")
}

#[test]
fn binary_crates_are_github_releasable() {
    let rp = load_release_plz();
    let by: BTreeMap<&str, &RpPackage> = rp.package.iter().map(|p| (p.name.as_str(), p)).collect();
    let mut errors = Vec::<String>::new();
    for &bin in BINARY_TARGETS {
        match by.get(bin) {
            None => errors.push(format!("[binary] {bin}: missing from release-plz.toml.")),
            Some(e) => {
                for (flag, val) in [
                    ("git_release_enable", e.git_release_enable),
                    ("git_tag_enable", e.git_tag_enable),
                    ("changelog_update", e.changelog_update),
                ] {
                    if val != Some(true) {
                        errors.push(format!(
                            "[binary] {bin}: release-plz.toml `{flag}` must be \
                             `true` so the binary produces a GitHub release on \
                             each cycle."
                        ));
                    }
                }
            }
        }
    }
    assert!(
        errors.is_empty(),
        "binary-crate releasability violations:\n{}",
        errors
            .iter()
            .map(|e| format!("  - {e}\n"))
            .collect::<String>()
    );
}

#[test]
fn release_plz_config_is_self_consistent() {
    let rp = load_release_plz();
    let errors: Vec<String> = rp
        .package
        .iter()
        .filter(|e| e.git_only == Some(true) && e.publish == Some(true))
        .map(|e| {
            format!(
                "[sanity] {}: `git_only=true` + `publish=true` contradict. \
                 git_only silently wins and disables publish — the exact wedge \
                 this PR fixes. Drop one.",
                e.name
            )
        })
        .collect();
    assert!(
        errors.is_empty(),
        "release-plz.toml self-consistency violations:\n{}",
        errors
            .iter()
            .map(|e| format!("  - {e}\n"))
            .collect::<String>()
    );
}
