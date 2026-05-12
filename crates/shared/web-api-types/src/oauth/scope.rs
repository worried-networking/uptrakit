//! OAuth scope enum for MCP. Wire-safe per the standards-snapshot rule
//! "Use `wire_safe_enum!` macro to generate `Other(String)`, `as_str`, ...".
//!
//! [`McpScope`] is transmitted on the wire (token responses, authorization
//! requests, metadata documents). The macro emits the `Other(String)` catch-all
//! so an older AS or RS build can decode a token issued with a scope value it
//! does not yet recognise instead of failing the whole request.

use uptrakit_shared_macros::wire_safe_enum;

wire_safe_enum! {
    /// OAuth scope values supported by the MCP authorization server.
    ///
    /// # Wire forward-compatibility
    ///
    /// `Other(String)` is a catch-all for scope strings received from a newer
    /// peer that this build does not yet recognise. Serde deserialization is
    /// infallible: an unknown string becomes `Other(...)` rather than a parse
    /// error, allowing older clients/servers to survive rolling upgrades
    /// without dropping the enclosing token or authorization response.
    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    pub enum McpScope {
        Read => "mcp:read",
        Write => "mcp:write",
    }
    parse_error = ParseMcpScopeError("invalid mcp scope");
}

impl McpScope {
    /// All `McpScope` variants except [`McpScope::Other`].
    ///
    /// Used by tests to enumerate the known variants without `strum::EnumIter`,
    /// which is incompatible with tuple variants like `Other(String)`.
    pub const KNOWN_VARIANTS: &'static [McpScope] = &[McpScope::Read, McpScope::Write];
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
        let scope: McpScope = serde_json::from_str(json).expect("deserialise must succeed");
        assert!(matches!(scope, McpScope::Other(_)));
    }
}
