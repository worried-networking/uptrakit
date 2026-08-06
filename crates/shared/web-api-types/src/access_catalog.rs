//! Response types for `GET /api/v1/access/catalog` (M1.6b): the
//! authorization vocabulary as data — actions, role bundles, scope presets.

use serde::{Deserialize, Serialize};
use uptrakit_shared_macros::wire_safe_enum;
use uptrakit_shared_types::access::SelectorSupport;

/// The full access catalog: three code-defined sections, one endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AccessCatalogResponse {
    /// Actions grouped by resource — built-in catalog plus live-registered
    /// dynamic (`surface.*`) entries.
    pub resources: Vec<CatalogResourceEntry>,
    /// Advisory role bundles (the demoted access-preset tiers); applied by
    /// clients via standard role assignment.
    pub role_bundles: Vec<RoleBundleEntry>,
    /// Advisory scope presets for token-creation UX (consumed in M4.2).
    pub scope_presets: Vec<ScopePresetEntry>,
}

/// One resource and its valid actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CatalogResourceEntry {
    /// Resource path, e.g. `hosts`, `settings.auth`, `surface.proxmox.hosts`.
    pub resource: String,
    pub actions: Vec<CatalogActionEntry>,
}

/// One valid action with its catalog metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CatalogActionEntry {
    /// The action string, e.g. `hosts:read`.
    pub action: String,
    /// The verb, e.g. `read`.
    pub verb: String,
    pub description: String,
    pub selector_support: SelectorSupport,
}

/// One advisory role bundle: tier name, description, seed-role names.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RoleBundleEntry {
    pub name: String,
    pub description: String,
    pub roles: Vec<String>,
}

wire_safe_enum! {
    /// How a scope preset's action list is produced.
    ///
    /// Client contract: an unknown kind (the `Other` fallback) means "do
    /// not offer this preset" — never treat it as `caller_actions` (the
    /// broadest interpretation); drop it from any picker.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum ScopePresetKind {
        /// `actions` carries the concrete list.
        Static        => "static",
        /// No list: the client expands against the caller's effective
        /// action list (`me`) at consumption time.
        CallerActions => "caller_actions",
    }
    parse_error = ParseScopePresetKindError("invalid scope preset kind");
}

/// One advisory scope preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ScopePresetEntry {
    pub name: String,
    pub description: String,
    pub kind: ScopePresetKind,
    /// Present when `kind` is `static`; absent for `caller_actions`;
    /// unspecified for unknown kinds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_kind_deserializes_to_other_never_caller_actions() {
        let kind: ScopePresetKind =
            serde_json::from_str(r#""some-future-kind""#).expect("deserialise must succeed");
        assert!(matches!(kind, ScopePresetKind::Other(_)));
        assert!(kind != ScopePresetKind::CallerActions);
    }
}
