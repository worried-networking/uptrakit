// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
/// An access preset definition with its role composition.
#[derive(Serialize, Deserialize, Clone)]
pub struct AccessPresetResponse {
    pub name: String,
    pub description: String,
    pub roles: Vec<String>,
}
