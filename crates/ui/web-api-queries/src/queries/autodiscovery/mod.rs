//! Database query helpers for the autodiscovery feature.
//!
//! Covers:
//! - Ignore rule management (create / list / delete)
//! - Auto-creation of default plugin configs from discovery targets
//! - Processing incoming `DiscoveryResults` payloads (creating pending software
//!   items and upserting host-software-item links)
//!
//! The controller is completely generic: plugins return structured
//! [`DiscoveryTarget`](uptrakit_shared_types::DiscoveryTarget) values that
//! specify exactly which plugin configs and roles to create -- no plugin-specific
//! synthesis logic lives here.

mod discovery_items;
mod ignore_rules;
mod reconcile;

pub mod default_configs;

// Re-export all public items so callers see the same API as before.
pub use default_configs::find_or_create_default_plugin_config;
pub use ignore_rules::{
    SoftwareIgnoreView, batch_delete_ignore_rules, create_or_ignore_ignore_rule,
    create_or_ignore_ignore_rule_in_tx, delete_ignore_rule, delete_ignore_rule_in_tx,
    list_ignore_rules,
};

use std::collections::HashSet;
use time::OffsetDateTime;
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_wire::{DiscoveryPluginResult, DiscoveryResultsPayload};
use uuid::Uuid;

/// Error returned by autodiscovery query helpers.
#[derive(Debug, thiserror::Error)]
pub enum AutodiscoveryError {
    #[error("database error: {0}")]
    Db(sea_orm::DbErr),
    #[error("audit log error: {0}")]
    Audit(uptrakit_audit_log::AuditLogError),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<AutodiscoveryError>>;
impl_report_conversion!(sea_orm::DbErr => AutodiscoveryError::Db);
impl_report_conversion!(uptrakit_audit_log::AuditLogError => AutodiscoveryError::Audit);

// -- Process discovery results --

/// Compute the effective `(package_identifier, qualifier)` set reported by a
/// single plugin result, ahead of any ignore-list filtering.
///
/// Reuses the same identifier-resolution rules `discovery_items` applies when
/// writing rows, so the set exactly mirrors what will end up assigned:
/// - Target-based items: mirrors the stored key exactly --
///   `host_software_item.package_identifier` ends up set to
///   `target_item.effective_plugin_pkg_id()`, i.e. the item's own
///   `plugin_package_identifier` (if set) takes precedence OVER the target's
///   `package_identifier` override, which in turn falls back to the item's own
///   `package_identifier` -- mirrors `discovery_items::process_targets_discovery`
///   composed with `DiscoveredItemInfo::effective_plugin_pkg_id()`.
/// - Plain items (no targets): `DiscoveredItemInfo::effective_plugin_pkg_id()`
///   (the `plugin_package_identifier` override, or `package_identifier`).
///
/// The qualifier is always the discovered item's own `qualifier` -- targets do
/// not override it. Qualifier-bearing links (e.g. Docker containers) reconcile
/// within their qualifier, so a snapshot reporting only qualifier "a" does not
/// affect a link keyed to qualifier "b" even when both share a
/// `package_identifier`.
fn effective_identifier_set(result: &DiscoveryPluginResult) -> HashSet<(String, Option<String>)> {
    let mut ids = HashSet::new();
    for item in &result.discoveries {
        if item.targets.is_empty() {
            let effective = item
                .plugin_package_identifier
                .as_deref()
                .unwrap_or(&item.package_identifier);
            ids.insert((effective.to_string(), item.qualifier.clone()));
        } else {
            for target in &item.targets {
                // Mirror the stored key exactly: host_software_item.package_identifier =
                // target_item.effective_plugin_pkg_id() = plugin_package_identifier (if set),
                // else the target's package_identifier override, else the item's own
                // package_identifier. Omitting the plugin_package_identifier layer makes
                // Docker links (plugin_package_identifier set, target package_identifier None)
                // read as absent every cycle -> false deactivation of installed software.
                let base = target
                    .package_identifier
                    .as_deref()
                    .unwrap_or(&item.package_identifier);
                let effective = item.plugin_package_identifier.as_deref().unwrap_or(base);
                ids.insert((effective.to_string(), item.qualifier.clone()));
            }
        }
    }
    ids
}

/// Process a `DiscoveryResultsPayload` received from an agent.
///
/// For each plugin result, delegates to one of two generic processing paths:
///
/// 1. **Target-based**: Items with non-empty `targets` are processed via
///    [`process_targets_discovery`] -- each target drives plugin-config
///    find-or-create and role-assignment creation.
/// 2. **Config-ID-based**: Items with empty `targets` use the pre-existing
///    `plugin_config_id` from the result for all three standard roles.
#[tracing::instrument(skip_all, fields(%tenant_id, %host_id))]
pub async fn process_discovery_results(
    db: &sea_orm::DatabaseConnection,
    audit: &uptrakit_audit_log::AuditEmitter,
    agent_id: Uuid,
    tenant_id: Uuid,
    host_id: Uuid,
    payload: DiscoveryResultsPayload,
) -> Result<()> {
    let now = OffsetDateTime::now_utc();

    // Load the tenant-wide ignore set once for the entire discovery run.
    let ignore_set = discovery_items::load_ignore_set(db, tenant_id).await?;

    for result in payload.results {
        if let Some(ref err) = result.error {
            tracing::warn!(
                %agent_id,
                plugin_type = %result.plugin_type,
                error = %err,
                "discovery plugin reported an error, skipping"
            );
            continue;
        }

        // Built from the RAW result (pre-ignore-filter) so reconcile sees every
        // identifier the snapshot reported, regardless of the tenant-wide
        // ignore list.
        let ids = effective_identifier_set(&result);

        if !result.discoveries.is_empty() {
            discovery_items::process_plugin_result(
                db,
                audit,
                tenant_id,
                host_id,
                now,
                &result,
                &ignore_set,
            )
            .await?;
        } else {
            tracing::debug!(
                %agent_id,
                plugin_type = %result.plugin_type,
                "discovery plugin returned no items"
            );
        }

        reconcile::reconcile_plugin_result(db, audit, tenant_id, host_id, now, &result, &ids)
            .await?;
    }

    Ok(())
}

// -- Shared test helpers --

#[expect(
    unreachable_pub,
    reason = "publicly visible for cross-module test access but not crate-public"
)]
#[cfg_attr(
    all(test, feature = "db-sqlite"),
    expect(
        clippy::expect_used,
        reason = "test helpers: panics on setup failure are acceptable"
    )
)]
#[cfg(all(test, feature = "db-sqlite"))]
pub(crate) mod tests_common {
    use std::sync::Arc;

    use sea_orm::{
        ActiveModelTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait, Set,
    };
    use uptrakit_audit_log::{AuditEmitter, AuditLogDispatcher, NoopBackend};
    use uptrakit_shared_db::entity::{
        host, host_software_item, host_software_item_plugin, plugin_config, prelude::*,
        software_item, tenant,
    };
    use uptrakit_wire::{
        DiscoveredSoftware as WireDiscoveredSoftware, DiscoveryPluginResult, DiscoveryTarget,
        PluginRole, plugin_ids,
    };
    use uuid::Uuid;

    pub async fn setup_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");
        db
    }

    /// Shared no-op `AuditEmitter` for `process_discovery_results` test callers.
    ///
    /// Backed only by a `NoopBackend`-wrapped dispatcher, so `emit_event`/
    /// `emit_stateful` calls are inert -- reconciliation tests (4c) share this
    /// so they don't each need to construct their own emitter.
    pub fn test_emitter() -> AuditEmitter {
        AuditEmitter::new(AuditLogDispatcher::new(Arc::new(NoopBackend)))
    }

    // -- FK-setup helpers --

    pub async fn insert_tenant(db: &DatabaseConnection, id: Uuid) {
        let now = time::OffsetDateTime::now_utc();
        tenant::ActiveModel {
            id: Set(id),
            name: Set("Test Tenant".to_string()),
            slug: Set(id.to_string()),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert tenant");
    }

    pub async fn insert_host(db: &DatabaseConnection, id: Uuid, tenant_id: Uuid) {
        let now = time::OffsetDateTime::now_utc();
        host::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            machine_id: Set(id.to_string()),
            hostname: Set("test-host".to_string()),
            friendly_name: Set("Test Host".to_string()),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            host_features: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert host");
    }

    pub async fn insert_plugin_config(db: &DatabaseConnection, id: Uuid, tenant_id: Uuid) {
        let now = time::OffsetDateTime::now_utc();
        plugin_config::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            name: Set(format!("Test Plugin Config {id}")),
            plugin_type: Set("package-manager.homebrew".to_string()),
            config: Set(serde_json::json!({})),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert plugin_config");
    }

    // -- query helpers --

    pub async fn insert_software_item(
        db: &DatabaseConnection,
        id: Uuid,
        tenant_id: Uuid,
        name: &str,
        deactivated_at: Option<time::OffsetDateTime>,
    ) {
        let now = time::OffsetDateTime::now_utc();
        let model = software_item::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            name: Set(name.to_string()),
            featured: Set(false),
            icon_url: Set(None),
            last_checked_at: Set(None),
            awaiting_restart_timeout: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(deactivated_at),
        };
        SoftwareItem::insert(model)
            .exec(db)
            .await
            .expect("insert software_item");
    }

    pub async fn insert_host_link(
        db: &DatabaseConnection,
        host_id: Uuid,
        software_item_id: Uuid,
        plugin_config_id: Uuid,
        package_identifier: &str,
    ) {
        let now = time::OffsetDateTime::now_utc();
        let hsi_id = Uuid::now_v7();
        let link = host_software_item::ActiveModel {
            id: Set(hsi_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            qualifier: Set(None),
            plugin_config_id: Set(Some(plugin_config_id)),
            package_identifier: Set(Some(package_identifier.to_string())),
            installed_version: Set(Some("1.0.0".to_string())),
            installed_version_detected_at: Set(Some(now)),
            installed_display_version: Set(None),
            latest_version: Set(None),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("unknown".to_string()),
            deactivated_at: Set(None),
            last_discovered_at: Set(None),
            discovery_source: Set(None),
            missing_since: Set(None),
        };
        HostSoftwareItem::insert(link)
            .exec(db)
            .await
            .expect("insert host_software_item");

        // Also create plugin link rows for all three roles to match the new schema.
        for role in ["detect_version", "fetch_releases", "execute_update"] {
            let plugin_link = host_software_item_plugin::ActiveModel {
                id: Set(Uuid::now_v7()),
                host_id: Set(host_id),
                software_item_id: Set(software_item_id),
                host_software_item_id: Set(hsi_id),
                plugin_config_id: Set(Some(plugin_config_id)),
                plugin_type: Set("package-manager.homebrew".to_string()),
                role: Set(role.to_string()),
                ordinal: Set(0),
                package_identifier: Set(package_identifier.to_string()),
                config: Set(None),
                execution_site: Set("auto".to_string()),
                created_at: Set(now),
                updated_at: Set(now),
            };
            HostSoftwareItemPlugin::insert(plugin_link)
                .exec(db)
                .await
                .expect("insert host_software_item_plugin");
        }
    }

    /// Like [`insert_host_link`] but sets discovery provenance columns
    /// (`discovery_source`, `last_discovered_at`, `missing_since`) and an
    /// optional `qualifier`, so reconciliation's candidate filter picks the
    /// row up. Returns the inserted link id.
    #[expect(
        clippy::too_many_arguments,
        reason = "test helper: explicit over builder for a fixed, well-understood set of reconciliation setup fields"
    )]
    pub async fn insert_discovered_host_link(
        db: &DatabaseConnection,
        host_id: Uuid,
        software_item_id: Uuid,
        plugin_config_id: Uuid,
        package_identifier: &str,
        qualifier: Option<&str>,
        discovery_source: &str,
        last_discovered_at: Option<time::OffsetDateTime>,
        missing_since: Option<time::OffsetDateTime>,
        deactivated_at: Option<time::OffsetDateTime>,
    ) -> Uuid {
        let now = time::OffsetDateTime::now_utc();
        let hsi_id = Uuid::now_v7();
        let link = host_software_item::ActiveModel {
            id: Set(hsi_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            qualifier: Set(qualifier.map(str::to_string)),
            plugin_config_id: Set(Some(plugin_config_id)),
            package_identifier: Set(Some(package_identifier.to_string())),
            installed_version: Set(Some("1.0.0".to_string())),
            installed_version_detected_at: Set(Some(now)),
            installed_display_version: Set(None),
            latest_version: Set(None),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("unknown".to_string()),
            deactivated_at: Set(deactivated_at),
            last_discovered_at: Set(last_discovered_at),
            discovery_source: Set(Some(discovery_source.to_string())),
            missing_since: Set(missing_since),
        };
        HostSoftwareItem::insert(link)
            .exec(db)
            .await
            .expect("insert host_software_item");
        hsi_id
    }

    // -- Helper: make DiscoveryPluginResult with targets --

    pub fn all_roles() -> Vec<PluginRole> {
        vec![
            PluginRole::DetectVersion,
            PluginRole::FetchReleases,
            PluginRole::ExecuteUpdate,
        ]
    }

    pub fn phs_result_with_github_target(
        pkg_id: &str,
        name: &str,
        version: &str,
        owner: &str,
        repo: &str,
    ) -> DiscoveryPluginResult {
        DiscoveryPluginResult {
            plugin_type: plugin_ids::DISCOVERY_PROXMOX_HELPER_SCRIPTS.clone(),
            plugin_config_id: None,
            error: None,
            discoveries: vec![WireDiscoveredSoftware {
                package_identifier: pkg_id.to_string(),
                name: name.to_string(),
                installed_version: version.to_string(),
                targets: vec![DiscoveryTarget {
                    plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
                    plugin_config: serde_json::json!({
                        "owner": owner,
                        "repo": repo,
                        "tag_strip_prefix": "v",
                        "include_prereleases": false,
                        "asset_patterns": [],
                        "detect_installed_version_command":
                            r#"cat -- "${HOME}/.{package_identifier}""#,
                        "install_command": "env PHS_SILENT=1 /usr/bin/update",
                    }),
                    plugin_config_name: format!("{owner}/{repo}"),
                    roles: all_roles(),
                    package_identifier: None,
                    config_override: None,
                    execution_site: None,
                }],
                extra: None,
                featured: false,
                qualifier: None,
                plugin_package_identifier: None,
                installed_display_version: None,
            }],
        }
    }

    pub fn phs_result_with_apt_target(
        pkg_id: &str,
        name: &str,
        version: &str,
    ) -> DiscoveryPluginResult {
        DiscoveryPluginResult {
            plugin_type: plugin_ids::DISCOVERY_PROXMOX_HELPER_SCRIPTS.clone(),
            plugin_config_id: None,
            error: None,
            discoveries: vec![WireDiscoveredSoftware {
                package_identifier: pkg_id.to_string(),
                name: name.to_string(),
                installed_version: version.to_string(),
                targets: vec![DiscoveryTarget {
                    plugin_type: plugin_ids::PACKAGE_MANAGER_APT.clone(),
                    plugin_config: serde_json::json!({}),
                    plugin_config_name: "APT (auto)".to_string(),
                    roles: all_roles(),
                    package_identifier: None,
                    config_override: None,
                    execution_site: None,
                }],
                extra: None,
                featured: false,
                qualifier: None,
                plugin_package_identifier: None,
                installed_display_version: None,
            }],
        }
    }

    pub fn phs_result_no_targets(pkg_id: &str) -> DiscoveryPluginResult {
        DiscoveryPluginResult {
            plugin_type: plugin_ids::DISCOVERY_PROXMOX_HELPER_SCRIPTS.clone(),
            plugin_config_id: None,
            error: None,
            discoveries: vec![WireDiscoveredSoftware {
                package_identifier: pkg_id.to_string(),
                name: pkg_id.to_string(),
                installed_version: "1.0.0".to_string(),
                targets: vec![],
                extra: None,
                featured: false,
                qualifier: None,
                plugin_package_identifier: None,
                installed_display_version: None,
            }],
        }
    }

    /// Mirrors the *actual* PHS plugin output for a GitHub-managed LXC container:
    /// - Target 1: `ReleasesGithub`, `FetchReleases` only,
    ///   `package_identifier = Some("owner/repo")`
    /// - Target 2: `GenericShell`, `[DetectVersion, ExecuteUpdate]`,
    ///   `package_identifier = None` (falls back to `pkg_id`)
    pub fn phs_result_with_two_targets(
        pkg_id: &str,
        name: &str,
        version: &str,
        owner: &str,
        repo: &str,
    ) -> DiscoveryPluginResult {
        DiscoveryPluginResult {
            plugin_type: plugin_ids::DISCOVERY_PROXMOX_HELPER_SCRIPTS.clone(),
            plugin_config_id: None,
            error: None,
            discoveries: vec![WireDiscoveredSoftware {
                package_identifier: pkg_id.to_string(),
                name: name.to_string(),
                installed_version: version.to_string(),
                targets: vec![
                    DiscoveryTarget {
                        plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
                        plugin_config: serde_json::json!({
                            "tag_strip_prefix": "v",
                            "include_prereleases": false,
                            "asset_patterns": [],
                        }),
                        plugin_config_name: "GitHub Releases".to_string(),
                        roles: vec![PluginRole::FetchReleases],
                        package_identifier: Some(format!("{owner}/{repo}")),
                        config_override: None,
                        execution_site: None,
                    },
                    DiscoveryTarget {
                        plugin_type: plugin_ids::GENERIC_SHELL.clone(),
                        plugin_config: serde_json::json!({
                            "version_command": "phs-app --version",
                            "update_command": "phs-update",
                        }),
                        plugin_config_name: "PHS Shell".to_string(),
                        roles: vec![PluginRole::DetectVersion, PluginRole::ExecuteUpdate],
                        package_identifier: None,
                        config_override: None,
                        execution_site: None,
                    },
                ],
                extra: None,
                featured: false,
                qualifier: None,
                plugin_package_identifier: None,
                installed_display_version: None,
            }],
        }
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use tests_common::*;
    use uptrakit_shared_db::entity::{host_software_item, plugin_config, prelude::*};
    use uptrakit_wire::plugin_ids;

    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

    /// `effective_identifier_set` must key on `(effective_package_identifier, qualifier)`,
    /// reusing the same resolution rules as `discovery_items`:
    /// - target-based items: `item.plugin_package_identifier.unwrap_or(target.package_identifier.unwrap_or(item.package_identifier))`
    /// - plain items (no targets): `item.effective_plugin_pkg_id()`
    ///
    /// A target's `package_identifier` override must replace the raw discovery-level id
    /// in the set (not add alongside it), and a plain item's qualifier must be preserved
    /// as part of the key so qualifier-bearing links reconcile independently.
    #[tokio::test]
    async fn effective_identifier_set_uses_target_override_and_qualifier() {
        let result = uptrakit_wire::DiscoveryPluginResult {
            plugin_type: plugin_ids::DISCOVERY_PROXMOX_HELPER_SCRIPTS.clone(),
            plugin_config_id: None,
            error: None,
            discoveries: vec![
                // Target-based item: the target overrides package_identifier.
                uptrakit_wire::DiscoveredSoftware {
                    package_identifier: "raw-discovery-id".to_string(),
                    name: "BookLore".to_string(),
                    installed_version: "1.18.5".to_string(),
                    targets: vec![uptrakit_wire::DiscoveryTarget {
                        plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
                        plugin_config: serde_json::json!({}),
                        plugin_config_name: "BookLore/BookLore".to_string(),
                        roles: all_roles(),
                        package_identifier: Some("BookLore/BookLore".to_string()),
                        config_override: None,
                        execution_site: None,
                    }],
                    extra: None,
                    featured: false,
                    qualifier: None,
                    plugin_package_identifier: None,
                    installed_display_version: None,
                },
                // Plain item (no targets) carrying a qualifier (e.g. Docker container).
                uptrakit_wire::DiscoveredSoftware {
                    package_identifier: "nginx:latest".to_string(),
                    name: "nginx".to_string(),
                    installed_version: "1.24.0".to_string(),
                    targets: vec![],
                    extra: None,
                    featured: false,
                    qualifier: Some("web-server".to_string()),
                    plugin_package_identifier: None,
                    installed_display_version: None,
                },
            ],
        };

        let ids = effective_identifier_set(&result);

        assert!(
            ids.contains(&("BookLore/BookLore".to_string(), None)),
            "expected the target's package_identifier override in the set"
        );
        assert!(
            !ids.contains(&("raw-discovery-id".to_string(), None)),
            "raw discovery-level id must NOT appear once a target overrides it"
        );
        assert!(
            ids.contains(&("nginx:latest".to_string(), Some("web-server".to_string()))),
            "expected the plain item's effective_plugin_pkg_id() paired with its qualifier"
        );
        assert_eq!(ids.len(), 2, "expected exactly two identifier-set entries");
    }

    /// Regression test for a false-deactivation bug: Docker discovery emits a
    /// target-based item (non-empty `targets`, target `package_identifier: None`)
    /// that also carries `plugin_package_identifier: Some(..)`. The write path
    /// (`discovery_items::process_targets_discovery` + `find_or_create_software_item`)
    /// stores `host_software_item.package_identifier` as
    /// `target_item.effective_plugin_pkg_id()`, which prefers
    /// `plugin_package_identifier` over the target's `package_identifier`
    /// override. `effective_identifier_set` must mirror that same precedence so
    /// the stored key is recognized as present on every reconciliation pass,
    /// instead of reading as absent and eventually deactivating a still-installed
    /// container.
    #[tokio::test]
    async fn effective_identifier_set_docker_link_stays_present() {
        let result = uptrakit_wire::DiscoveryPluginResult {
            plugin_type: plugin_ids::RELEASES_DOCKER.clone(),
            plugin_config_id: None,
            error: None,
            discoveries: vec![uptrakit_wire::DiscoveredSoftware {
                package_identifier: "nginx:latest".to_string(),
                name: "nginx".to_string(),
                installed_version: "sha256:deadbeef".to_string(),
                targets: vec![uptrakit_wire::DiscoveryTarget {
                    plugin_type: plugin_ids::RELEASES_DOCKER.clone(),
                    plugin_config: serde_json::json!({}),
                    plugin_config_name: "Docker".to_string(),
                    roles: all_roles(),
                    package_identifier: None,
                    config_override: None,
                    execution_site: None,
                }],
                extra: Some(serde_json::json!({ "container": "web-server" })),
                featured: true,
                qualifier: Some("web-server".to_string()),
                plugin_package_identifier: Some("nginx:latest#web-server".to_string()),
                installed_display_version: None,
            }],
        };

        let ids = effective_identifier_set(&result);

        assert!(
            ids.contains(&(
                "nginx:latest#web-server".to_string(),
                Some("web-server".to_string())
            )),
            "expected plugin_package_identifier to take precedence over the target's \
             None package_identifier override, matching the stored host_software_item key"
        );
        assert!(
            !ids.contains(&("nginx:latest".to_string(), Some("web-server".to_string()))),
            "the bare package_identifier (dropping plugin_package_identifier) must NOT \
             appear -- that shape is what caused every Docker container to read as absent"
        );
        assert_eq!(ids.len(), 1, "expected exactly one identifier-set entry");
    }

    /// When an item arrives with `plugin_config_id: None` but carries a
    /// `DiscoveryTarget`, the server must auto-create the plugin config and
    /// create a `host_software_items` link row.
    ///
    /// This covers the first-run Homebrew / APT discovery path where no plugin
    /// config exists for the tenant yet.
    #[tokio::test]
    async fn target_based_auto_creates_config_and_host_link() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let agent_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;

        let payload = uptrakit_wire::DiscoveryResultsPayload {
            host_machine_id: "test-machine".to_string(),
            results: vec![uptrakit_wire::DiscoveryPluginResult {
                plugin_type: plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone(),
                plugin_config_id: None,
                error: None,
                discoveries: vec![uptrakit_wire::DiscoveredSoftware {
                    package_identifier: "wget".to_string(),
                    name: "wget".to_string(),
                    installed_version: "1.24.4".to_string(),
                    targets: vec![uptrakit_wire::DiscoveryTarget {
                        plugin_type: plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone(),
                        plugin_config: serde_json::json!({"package_type": "formula"}),
                        plugin_config_name: "Homebrew (Formulae)".to_string(),
                        roles: all_roles(),
                        package_identifier: None,
                        config_override: None,
                        execution_site: None,
                    }],
                    extra: None,
                    featured: false,
                    qualifier: None,
                    plugin_package_identifier: None,
                    installed_display_version: None,
                }],
            }],
        };

        process_discovery_results(&db, &test_emitter(), agent_id, tenant_id, host_id, payload)
            .await
            .expect("process must succeed");

        // No plugin_config should be created for package manager types.
        let configs = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::PluginType.eq("package-manager.homebrew"))
            .all(&db)
            .await
            .expect("query plugin configs");
        assert!(
            configs.is_empty(),
            "package managers no longer create plugin_configs"
        );

        // host_software_items link must have been created.
        let hsi_links = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .all(&db)
            .await
            .expect("query host software items");
        assert_eq!(
            hsi_links.len(),
            1,
            "exactly one host_software_items row must exist"
        );
        assert_eq!(
            hsi_links[0].installed_version.as_deref(),
            Some("1.24.4"),
            "installed version must be recorded"
        );
    }

    /// Repeated discoveries with the same target are idempotent:
    /// the plugin config is reused and the first-run version is preserved.
    #[tokio::test]
    async fn target_based_idempotent_on_second_run() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let agent_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;

        let make_payload = |version: &str| {
            let version = version.to_string();
            uptrakit_wire::DiscoveryResultsPayload {
                host_machine_id: "test-machine".to_string(),
                results: vec![uptrakit_wire::DiscoveryPluginResult {
                    plugin_type: plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone(),
                    plugin_config_id: None,
                    error: None,
                    discoveries: vec![uptrakit_wire::DiscoveredSoftware {
                        package_identifier: "wget".to_string(),
                        name: "wget".to_string(),
                        installed_version: version,
                        targets: vec![uptrakit_wire::DiscoveryTarget {
                            plugin_type: plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone(),
                            plugin_config: serde_json::json!({"package_type": "formula"}),
                            plugin_config_name: "Homebrew (Formulae)".to_string(),
                            roles: all_roles(),
                            package_identifier: None,
                            config_override: None,
                            execution_site: None,
                        }],
                        extra: None,
                        featured: false,
                        qualifier: None,
                        plugin_package_identifier: None,
                        installed_display_version: None,
                    }],
                }],
            }
        };

        // First run.
        process_discovery_results(
            &db,
            &test_emitter(),
            agent_id,
            tenant_id,
            host_id,
            make_payload("1.24.4"),
        )
        .await
        .expect("first run");

        // Second run with an updated version.
        process_discovery_results(
            &db,
            &test_emitter(),
            agent_id,
            tenant_id,
            host_id,
            make_payload("1.24.5"),
        )
        .await
        .expect("second run");

        // No plugin_config should be created for package manager types.
        let config_count = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::PluginType.eq("package-manager.homebrew"))
            .count(&db)
            .await
            .expect("count configs");
        assert_eq!(
            config_count, 0,
            "package managers no longer create plugin_configs"
        );

        // Still exactly one host_software_items link, with the first-run version preserved.
        let hsi_links = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .all(&db)
            .await
            .expect("query host software items");
        assert_eq!(
            hsi_links.len(),
            1,
            "must not create duplicate host_software_items rows"
        );
        assert_eq!(
            hsi_links[0].installed_version.as_deref(),
            Some("1.24.4"),
            "the first-run version must be preserved on the second run"
        );
    }
}
