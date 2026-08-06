//! `GET /api/v1/access/catalog` (M1.6b): the authorization vocabulary as
//! data. Authenticated-but-ungoverned — the built-in vocabulary is
//! code-defined and the live `surface.*` disclosure is an accepted v1
//! reconnaissance surface (spec §1; corpus 07 §Catalog introspection).

use axum::{Extension, Json, extract::State};
use uptrakit_shared_types::RoleBundle;
use uptrakit_shared_types::access::CATALOG;

use crate::app_state::AccessState;
use crate::middleware::require_auth::AuthenticatedUser;

pub use uptrakit_web_api_types::access_catalog::{
    AccessCatalogResponse, CatalogActionEntry, CatalogResourceEntry, RoleBundleEntry,
    ScopePresetEntry, ScopePresetKind,
};

/// The `all-reads` scope preset: an explicit REVIEWED list, not a lexical
/// derivation — a new `read` verb in `CATALOG` must be consciously added
/// here (or to `ALL_READS_EXCLUDED`); the guard test reds either way.
/// System-plane reads are included deliberately: a scope is a ceiling over
/// the caller's grants, never a grant (spec §1).
const ALL_READS_ACTIONS: &[&str] = &[
    "services:read",
    "system.services:read",
    "software:read",
    "hosts:read",
    "settings:read",
    "notifications:read",
    "audit:read",
    "system.audit:read",
    "system.config-state:read",
];

/// `read`-verb catalog actions reviewed OUT of `all-reads` (none today).
/// Test-only: referenced solely by the guard test below, which is the only
/// consumer of a reviewed-out list.
#[cfg(test)]
const ALL_READS_EXCLUDED: &[&str] = &[];

/// Get the access catalog
#[utoipa::path(
    get,
    path = "/api/v1/access/catalog",
    responses(
        (status = 200, description = "The access catalog", body = AccessCatalogResponse),
        (status = 401, description = "Not authenticated")
    ),
    tag = "Access",
    security(("oauth2" = []), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_access_catalog(
    State(access): State<AccessState>,
    Extension(_auth_user): Extension<AuthenticatedUser>,
) -> Json<AccessCatalogResponse> {
    let mut resources: Vec<CatalogResourceEntry> = CATALOG
        .iter()
        .map(|entry| CatalogResourceEntry {
            resource: entry.resource_str.to_string(),
            actions: entry
                .verbs
                .iter()
                .map(|verb| CatalogActionEntry {
                    action: verb.action_str.to_string(),
                    verb: verb.verb.as_str().to_string(),
                    description: verb.description.to_string(),
                    selector_support: verb.selector_support,
                })
                .collect(),
        })
        .collect();

    for action in access.0.dynamic_actions() {
        let entry = CatalogActionEntry {
            action: action.to_string(),
            verb: action.verb().as_str().to_string(),
            description: format!("Use the {} surface", action.resource().as_str()),
            selector_support: uptrakit_shared_types::access::SelectorSupport::None,
        };
        resources.push(CatalogResourceEntry {
            resource: action.resource().as_str().to_string(),
            actions: vec![entry],
        });
    }

    let role_bundles = RoleBundle::all()
        .iter()
        .map(|bundle| RoleBundleEntry {
            name: bundle.as_str().to_string(),
            description: bundle.description().to_string(),
            roles: bundle
                .roles()
                .iter()
                .map(|role| (*role).to_string())
                .collect(),
        })
        .collect();

    let scope_presets = vec![
        ScopePresetEntry {
            name: "all-reads".to_string(),
            description: "Read-only access: every built-in read action".to_string(),
            kind: ScopePresetKind::Static,
            actions: Some(ALL_READS_ACTIONS.iter().map(|a| (*a).to_string()).collect()),
        },
        ScopePresetEntry {
            name: "all-my-current-actions".to_string(),
            description: "Everything the caller can currently do; the client \
                          expands this against the caller's effective action \
                          list at token-creation time"
                .to_string(),
            kind: ScopePresetKind::CallerActions,
            actions: None,
        },
    ];

    Json(AccessCatalogResponse {
        resources,
        role_bundles,
        scope_presets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_shared_types::access::Verb;

    /// Bidirectional reviewed-list guard (spec §1)
    #[test]
    fn all_reads_list_matches_catalog_read_actions_bidirectionally() {
        let catalog_reads: Vec<&str> = CATALOG
            .iter()
            .flat_map(|entry| entry.verbs.iter())
            .filter(|verb| verb.verb == Verb::Read)
            .map(|verb| verb.action_str)
            .collect();
        for action in &catalog_reads {
            assert!(
                ALL_READS_ACTIONS.contains(action) || ALL_READS_EXCLUDED.contains(action),
                "new read action {action} in CATALOG: review it into ALL_READS_ACTIONS or ALL_READS_EXCLUDED"
            );
        }
        for listed in ALL_READS_ACTIONS.iter().chain(ALL_READS_EXCLUDED) {
            assert!(
                catalog_reads.contains(listed),
                "stale entry {listed}: no longer a read action in CATALOG"
            );
        }
    }
}
