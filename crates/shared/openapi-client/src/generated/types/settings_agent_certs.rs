// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
pub struct AgentCertificateSettingsResponse {
    /// Certificate lifetime in hours (max 17520).
    pub lifetime_hours: u32,
    /// Admin-configured renewal window override in hours.
    ///
    /// `null` means automatic mode: the window is `min(14 days, lifetime / 5)`.
    pub renewal_window_hours_override: Option<u16>,
    /// Effective renewal window in hours.
    ///
    /// In automatic mode this equals `min(14 days, lifetime_hours / 5)`.
    /// When an override is set this equals `renewal_window_hours_override`.
    pub effective_renewal_window_hours: u16,
}
#[derive(Serialize, Deserialize)]
pub struct UpdateAgentCertificateSettingsRequest {
    /// Certificate lifetime in hours (max 17520).
    pub lifetime_hours: Option<u32>,
    /// Renewal window override in hours.
    ///
    /// Set to `0` to reset to automatic mode (`min(14 days, lifetime / 5)`).
    /// Omit to leave the current value unchanged.
    pub renewal_window_hours: Option<u16>,
}
