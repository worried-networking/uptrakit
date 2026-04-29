// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
/// The kind of configuration test to perform on the agent.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigTestKind {
    /// Execute `detect_installed_version()` and return output + detected version.
    VersionDetection,
    /// Validate update_command syntax (sh -n check, do NOT execute).
    UpdateCommandValidation,
    /// Execute pre-update hook with mock context.
    PreUpdateHook,
    /// Execute post-update hook with mock context.
    PostUpdateHook,
    /// Test connectivity for controller-side plugins (`fetch_releases`).
    Connectivity,
}
