//! OAuth scope enum for MCP. Wire-safe per `crates/shared/wire/src/lib.rs` convention.
//!
//! [`McpScope`] is transmitted on the wire (token responses, authorization
//! requests, metadata documents). It uses the `Other(String)` catch-all so an
//! older AS or RS build can decode a token issued with a scope value it does
//! not yet recognise instead of failing the whole request.

use std::fmt;
use std::str::FromStr;

/// OAuth scope values supported by the MCP authorization server.
///
/// # Wire forward-compatibility
///
/// `Other(String)` is a catch-all for scope strings received from a newer
/// peer that this build does not yet recognise. Serde deserialization is
/// infallible: an unknown string becomes `Other(...)` rather than a parse
/// error, allowing older clients/servers to survive rolling upgrades without
/// dropping the enclosing token or authorization response.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum McpScope {
    /// Read-only access to MCP resources.
    Read,
    /// Read-write access to MCP resources.
    Write,
    /// An unknown scope received from a newer peer.
    ///
    /// The inner string is the raw value as it appeared on the wire.
    Other(String),
}

impl McpScope {
    /// All `McpScope` variants except [`McpScope::Other`].
    ///
    /// Used by tests to enumerate the known variants without `strum::EnumIter`,
    /// which is incompatible with tuple variants like `Other(String)`.
    pub const KNOWN_VARIANTS: &'static [McpScope] = &[McpScope::Read, McpScope::Write];

    /// Returns the string representation.
    ///
    /// For [`McpScope::Other`], returns the inner string as-is.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            McpScope::Read => "mcp:read",
            McpScope::Write => "mcp:write",
            McpScope::Other(s) => s.as_str(),
        }
    }
}

impl fmt::Display for McpScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for McpScope {
    /// Converts a scope string to an [`McpScope`].
    ///
    /// Unknown strings map to [`McpScope::Other`] rather than failing.
    fn from(s: String) -> Self {
        match s.as_str() {
            "mcp:read" => McpScope::Read,
            "mcp:write" => McpScope::Write,
            _ => {
                tracing::debug!(scope = s, "received unknown mcp scope from peer");
                McpScope::Other(s)
            }
        }
    }
}

impl FromStr for McpScope {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(McpScope::from(s.to_string()))
    }
}

impl serde::Serialize for McpScope {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for McpScope {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(McpScope::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_variants_round_trip_via_as_str() {
        for v in McpScope::KNOWN_VARIANTS {
            let s = v.as_str();
            let parsed = McpScope::from(s.to_string());
            assert_eq!(*v, parsed);
        }
    }

    #[test]
    fn unknown_scope_round_trips_via_other() {
        let s = "mcp:custom:future";
        let scope = McpScope::from(s.to_string());
        assert_eq!(scope, McpScope::Other(s.to_string()));
        assert_eq!(scope.as_str(), s);
    }

    #[test]
    fn deserialize_infallible_for_unknown_string() {
        let json = r#""mcp:future_scope""#;
        let scope: McpScope = serde_json::from_str(json).unwrap();
        assert!(matches!(scope, McpScope::Other(_)));
    }
}
