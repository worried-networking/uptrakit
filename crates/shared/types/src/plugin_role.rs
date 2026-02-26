use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Roles that a plugin can fulfill in the version-check / update lifecycle.
///
/// Each `(host_id, software_item_id)` pair may have up to one plugin
/// assignment per role, enabling mix-and-match plugin configurations
/// (e.g., APT for detection, GitHub for release fetching).
///
/// # Wire forward-compatibility
///
/// `Other(String)` is a catch-all variant for role strings received from a
/// newer peer that this binary does not yet know about.  Serde deserialization
/// is infallible: an unknown string such as `"pre_update_hook"` becomes
/// `Other("pre_update_hook")` rather than a parse error, allowing older agents
/// and web-API clients to survive rolling upgrades without dropping entire
/// messages.
///
/// `FromStr` retains its original error behaviour for *known-role* contexts
/// (API validation, URL parameters, database columns) where a caller
/// explicitly needs to distinguish known variants from unknown ones.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[non_exhaustive]
pub enum PluginRole {
    /// Detects the installed version on the agent host.
    DetectVersion,
    /// Fetches the latest available version (controller-side for API plugins,
    /// agent-side for local package-index plugins).
    FetchReleases,
    /// Executes the actual software update on the agent host.
    ExecuteUpdate,
    /// An unknown plugin role received from a newer peer.
    ///
    /// The inner string is the raw snake_case value as it appeared on the wire.
    Other(String),
}

impl PluginRole {
    /// Returns the snake_case string representation of this plugin role.
    ///
    /// For [`PluginRole::Other`], returns the inner string as-is.
    pub fn as_str(&self) -> &str {
        match self {
            Self::DetectVersion => "detect_version",
            Self::FetchReleases => "fetch_releases",
            Self::ExecuteUpdate => "execute_update",
            Self::Other(s) => s.as_str(),
        }
    }
}

/// Error returned when parsing a string that does not match any *known*
/// [`PluginRole`] variant.
///
/// Note: serde deserialization is *infallible* — unknown strings are mapped to
/// [`PluginRole::Other`] rather than returning this error.  `ParsePluginRoleError`
/// is only returned from the [`FromStr`] implementation, which is used in
/// contexts where the caller must distinguish known from unknown roles
/// (API validation, URL query parameters, etc.).
#[derive(Debug, Error)]
pub enum ParsePluginRoleError {
    /// The input string does not match any known plugin role.
    #[error("invalid plugin role value")]
    Invalid,
}

impl FromStr for PluginRole {
    type Err = ParsePluginRoleError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "detect_version" => Ok(Self::DetectVersion),
            "fetch_releases" => Ok(Self::FetchReleases),
            "execute_update" => Ok(Self::ExecuteUpdate),
            _ => Err(ParsePluginRoleError::Invalid),
        }
    }
}

impl fmt::Display for PluginRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Serde: infallible string-based serialization ─────────────────────────────
//
// Custom Serialize/Deserialize are implemented manually rather than via derive
// so that unknown strings deserialize to `Other(String)` rather than failing.
// This makes rolling upgrades wire-safe: a message containing a new plugin
// role from a newer server can be fully parsed by an older client without
// dropping the enclosing struct.

impl From<String> for PluginRole {
    /// Converts a snake_case string to a plugin role.
    ///
    /// Unknown strings map to [`PluginRole::Other`] rather than failing.
    fn from(s: String) -> Self {
        match s.as_str() {
            "detect_version" => Self::DetectVersion,
            "fetch_releases" => Self::FetchReleases,
            "execute_update" => Self::ExecuteUpdate,
            _ => Self::Other(s),
        }
    }
}

impl From<PluginRole> for String {
    fn from(pr: PluginRole) -> String {
        match pr {
            PluginRole::DetectVersion => "detect_version".to_string(),
            PluginRole::FetchReleases => "fetch_releases".to_string(),
            PluginRole::ExecuteUpdate => "execute_update".to_string(),
            PluginRole::Other(s) => s,
        }
    }
}

impl Serialize for PluginRole {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PluginRole {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Deserialize as a plain string, then convert via From<String>.
        // Unknown strings become Other(s) — this conversion is infallible.
        String::deserialize(deserializer).map(PluginRole::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_role_serialization_roundtrip() {
        let dv = PluginRole::DetectVersion;
        let json = serde_json::to_string(&dv).expect("serialize");
        assert_eq!(json, r#""detect_version""#);

        let deserialized: PluginRole = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, dv);
    }

    #[test]
    fn plugin_role_fetch_releases_serialization() {
        let fr = PluginRole::FetchReleases;
        let json = serde_json::to_string(&fr).expect("serialize");
        assert_eq!(json, r#""fetch_releases""#);

        let deserialized: PluginRole = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, fr);
    }

    #[test]
    fn plugin_role_execute_update_serialization() {
        let eu = PluginRole::ExecuteUpdate;
        let json = serde_json::to_string(&eu).expect("serialize");
        assert_eq!(json, r#""execute_update""#);

        let deserialized: PluginRole = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, eu);
    }

    /// Unknown strings from a newer peer must deserialize to `Other(String)`
    /// rather than failing.  This is the core forward-compatibility guarantee.
    #[test]
    fn plugin_role_unknown_deserializes_to_other() {
        let deserialized: PluginRole =
            serde_json::from_str(r#""pre_update_hook""#).expect("deserialize unknown");
        assert_eq!(
            deserialized,
            PluginRole::Other("pre_update_hook".to_string())
        );

        let deserialized: PluginRole =
            serde_json::from_str(r#""post_update_hook""#).expect("deserialize unknown");
        assert_eq!(
            deserialized,
            PluginRole::Other("post_update_hook".to_string())
        );
    }

    /// `Other(String)` must serialize back to its inner string.
    #[test]
    fn plugin_role_other_serializes_to_inner_string() {
        let pr = PluginRole::Other("pre_update_hook".to_string());
        let json = serde_json::to_string(&pr).expect("serialize");
        assert_eq!(json, r#""pre_update_hook""#);
    }

    /// Full serde roundtrip for `Other`: deserialize then re-serialize produces
    /// the original JSON string unchanged.
    #[test]
    fn plugin_role_other_roundtrip() {
        let original = r#""custom_role""#;
        let deserialized: PluginRole = serde_json::from_str(original).expect("deserialize");
        assert_eq!(
            deserialized,
            PluginRole::Other("custom_role".to_string())
        );
        let reserialized = serde_json::to_string(&deserialized).expect("serialize");
        assert_eq!(reserialized, original);
    }

    /// `From<String>` maps known strings to known variants and unknown strings
    /// to `Other`.
    #[test]
    fn plugin_role_from_string() {
        assert_eq!(
            PluginRole::from("detect_version".to_string()),
            PluginRole::DetectVersion
        );
        assert_eq!(
            PluginRole::from("fetch_releases".to_string()),
            PluginRole::FetchReleases
        );
        assert_eq!(
            PluginRole::from("execute_update".to_string()),
            PluginRole::ExecuteUpdate
        );
        assert_eq!(
            PluginRole::from("custom_role".to_string()),
            PluginRole::Other("custom_role".to_string())
        );
    }

    #[test]
    fn plugin_role_display() {
        assert_eq!(PluginRole::DetectVersion.to_string(), "detect_version");
        assert_eq!(PluginRole::FetchReleases.to_string(), "fetch_releases");
        assert_eq!(PluginRole::ExecuteUpdate.to_string(), "execute_update");
        assert_eq!(
            PluginRole::Other("custom_role".to_string()).to_string(),
            "custom_role"
        );
    }

    #[test]
    fn plugin_role_from_str_valid() {
        assert_eq!(
            "detect_version".parse::<PluginRole>().ok(),
            Some(PluginRole::DetectVersion)
        );
        assert_eq!(
            "fetch_releases".parse::<PluginRole>().ok(),
            Some(PluginRole::FetchReleases)
        );
        assert_eq!(
            "execute_update".parse::<PluginRole>().ok(),
            Some(PluginRole::ExecuteUpdate)
        );
    }

    /// `FromStr` still rejects unknown strings to preserve the API's
    /// ability to distinguish known from unknown roles in validation contexts.
    #[test]
    fn plugin_role_from_str_invalid_returns_err() {
        assert!("unknown".parse::<PluginRole>().is_err());
        assert!("".parse::<PluginRole>().is_err());
        assert!("DETECT_VERSION".parse::<PluginRole>().is_err());
        assert!("DetectVersion".parse::<PluginRole>().is_err());
    }

    #[test]
    fn plugin_role_from_str_error_display() {
        let err = "bad_value".parse::<PluginRole>().unwrap_err();
        assert_eq!(err.to_string(), "invalid plugin role value");
    }

    /// Known variants round-trip through `FromStr`.
    #[test]
    fn plugin_role_display_round_trips_through_from_str() {
        let variants = [
            PluginRole::DetectVersion,
            PluginRole::FetchReleases,
            PluginRole::ExecuteUpdate,
        ];
        for pr in &variants {
            let s = pr.to_string();
            let parsed: PluginRole = s
                .parse()
                .expect("from_str should succeed for Display output of known variants");
            assert_eq!(&parsed, pr);
        }
    }

    #[test]
    fn plugin_role_as_str_matches_display() {
        let variants = [
            PluginRole::DetectVersion,
            PluginRole::FetchReleases,
            PluginRole::ExecuteUpdate,
            PluginRole::Other("my_role".to_string()),
        ];
        for pr in &variants {
            assert_eq!(pr.as_str(), pr.to_string());
        }
    }
}
