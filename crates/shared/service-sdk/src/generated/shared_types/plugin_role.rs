// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
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
    /// Runs a lifecycle plugin before the update (ordered by `ordinal`).
    PreUpdateHook,
    /// Runs a lifecycle plugin after the update (ordered by `ordinal`).
    PostUpdateHook,
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
            Self::PreUpdateHook => "pre_update_hook",
            Self::PostUpdateHook => "post_update_hook",
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
            "pre_update_hook" => Ok(Self::PreUpdateHook),
            "post_update_hook" => Ok(Self::PostUpdateHook),
            _ => Err(ParsePluginRoleError::Invalid),
        }
    }
}
impl fmt::Display for PluginRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl From<String> for PluginRole {
    /// Converts a snake_case string to a plugin role.
    ///
    /// Unknown strings map to [`PluginRole::Other`] rather than failing.
    fn from(s: String) -> Self {
        match s.as_str() {
            "detect_version" => Self::DetectVersion,
            "fetch_releases" => Self::FetchReleases,
            "execute_update" => Self::ExecuteUpdate,
            "pre_update_hook" => Self::PreUpdateHook,
            "post_update_hook" => Self::PostUpdateHook,
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
            PluginRole::PreUpdateHook => "pre_update_hook".to_string(),
            PluginRole::PostUpdateHook => "post_update_hook".to_string(),
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
        String::deserialize(deserializer).map(PluginRole::from)
    }
}
