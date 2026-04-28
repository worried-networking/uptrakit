// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
/// Attestation status for a GitHub release as determined by the GitHub Attestations API.
///
/// Used by the controller to record results of `actions/attest`-based Sigstore
/// provenance checks and by the agent to enforce the `require_attestation` policy.
///
/// # Wire forward-compatibility
///
/// `Other(String)` is a catch-all variant for status strings received from a
/// newer peer that this binary does not yet know about. Serde deserialization
/// is infallible: an unknown string becomes `Other(s)` rather than a parse error,
/// allowing older agents and web-API clients to survive rolling upgrades.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttestationStatus {
    /// At least one release asset was verified via the GitHub Attestations API.
    Verified,
    /// GitHub Attestations API returned no attestations for the release asset digest.
    NotFound,
    /// Attestation check was not performed (no checksums file found or check disabled).
    Unverified,
    /// An unknown attestation status received from a newer peer.
    Other(String),
}
impl From<String> for AttestationStatus {
    fn from(s: String) -> Self {
        match s.as_str() {
            "Verified" => Self::Verified,
            "NotFound" => Self::NotFound,
            "Unverified" => Self::Unverified,
            _ => Self::Other(s),
        }
    }
}
impl Serialize for AttestationStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let s = match self {
            Self::Verified => "Verified",
            Self::NotFound => "NotFound",
            Self::Unverified => "Unverified",
            Self::Other(s) => s.as_str(),
        };
        serializer.serialize_str(s)
    }
}
impl<'de> Deserialize<'de> for AttestationStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(AttestationStatus::from)
    }
}
/// A downloadable asset attached to a release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseAsset {
    /// Asset filename.
    pub name: String,
    /// Direct download URL.
    pub download_url: String,
    /// File size in bytes, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// MIME content type, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// SHA-256 digest from the release checksums file, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256_digest: Option<String>,
}
/// Simplified release info for update execution context.
///
/// Contains the minimal release metadata needed by plugins to execute updates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub tag: String,
    pub release_url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<ReleaseAsset>,
    /// Attestation status as determined by the GitHub Attestations API.
    ///
    /// Set by the controller from the most recent `fetch_releases` run.
    /// `None` means the check was never performed or the source is not GitHub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation_status: Option<AttestationStatus>,
    /// When `true`, the agent must abort the update if attestation is not `Verified`.
    ///
    /// Copied from `GitHubConfig.require_attestation` by the controller at
    /// trigger time.
    #[serde(default)]
    pub require_attestation: bool,
}
