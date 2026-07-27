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
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schema", derive(strum::EnumIter))]
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

impl AttestationStatus {
    /// Returns the wire-format string for this status (PascalCase, matching the
    /// hand-written `Serialize` impl).
    ///
    /// For [`AttestationStatus::Other`], returns the inner string as-is.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Verified => "Verified",
            Self::NotFound => "NotFound",
            Self::Unverified => "Unverified",
            Self::Other(s) => s.as_str(),
        }
    }
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
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AttestationStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(AttestationStatus::from)
    }
}

// ── JSON Schema impl ──────────────────────────────────────────────────────────
//
// `derive(schemars::JsonSchema)` would document Rust variant identifiers rather
// than the wire strings — a silent semantic bug (spec §1). Open string schema:
// no `"enum"` array (the `Other(String)` catch-all makes the value space open).
// Known-value list derived via `strum::EnumIter` from the same `as_str()` arm
// in the hand-written `Serialize` impl uses — a hardcoded list would drift.
//
// Note: this type uses PascalCase wire strings ("Verified", "NotFound") —
// derived from the match arms in `Serialize`, not snake_case.

#[cfg(feature = "schema")]
impl schemars::JsonSchema for AttestationStatus {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("AttestationStatus")
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        use strum::IntoEnumIterator;
        // `as_str()` mirrors the Serialize arm (PascalCase: "Verified", etc.).
        let known: Vec<String> = AttestationStatus::iter()
            .filter(|v| !matches!(v, Self::Other(_)))
            .map(|v| v.as_str().to_string())
            .collect();
        schemars::json_schema!({
            "type": "string",
            "description": format!(
                "Open wire string (unknown values are forward-compatible). Known values: {}.",
                known.join(", ")
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "schema")]
    mod schema_tests {
        use super::super::*;

        fn assert_open_string_schema<T: schemars::JsonSchema>(known: &[&str]) {
            let schema = schemars::schema_for!(T);
            let value = serde_json::to_value(&schema).expect("schema to JSON");
            assert_eq!(value["type"], "string");
            assert!(
                value.get("enum").is_none(),
                "must be an open string schema, found closed enum list: {value}"
            );
            let desc = value["description"].as_str().expect("description present");
            for k in known {
                assert!(
                    desc.contains(k),
                    "known value {k} missing from description: {desc}"
                );
            }
        }

        #[test]
        fn attestation_status_schema_is_open_string_with_known_values() {
            assert_open_string_schema::<AttestationStatus>(&["Verified", "NotFound", "Unverified"]);
        }
    }
}

/// A downloadable asset attached to a release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
