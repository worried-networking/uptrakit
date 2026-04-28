// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Debug, Serialize, Deserialize)]
pub struct MessageResponse {
    pub message: String,
}
#[derive(Serialize, Deserialize)]
pub struct MergeAgentRequest {
    pub source_id: Uuid,
}
