//! Response types for `GET /api/v1/access/catalog` (M1.6b): the
//! authorization vocabulary as data — actions, role bundles, scope presets.

use serde::{Deserialize, Serialize};
use uptrakit_shared_macros::wire_safe_enum;
use uptrakit_shared_types::access::SelectorSupport;

/// The full access catalog: three code-defined sections, one endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AccessCatalogResponse {
    pub resources: Vec<CatalogResourceEntry>,
    pub role_bundles: Vec<RoleBundleEntry>,
    pub scope_presets: Vec<ScopePresetEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CatalogResourceEntry {
    pub resource: String,
    pub actions: Vec<CatalogActionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CatalogActionEntry {
    pub action: String,
    pub verb: String,
    pub description: String,
    pub selector_support: SelectorSupport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RoleBundleEntry {
    pub name: String,
    pub description: String,
    pub roles: Vec<String>,
}

wire_safe_enum! {
    /// How a scope preset's action list is produced.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum ScopePresetKind {
        Static        => "static",
        CallerActions => "caller_actions",
    }
    parse_error = ParseScopePresetKindError("invalid scope preset kind");
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ScopePresetEntry {
    pub name: String,
    pub description: String,
    pub kind: ScopePresetKind,
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
