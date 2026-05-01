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

pub mod default_configs;

// Re-export all public items so callers see the same API as before.
pub use default_configs::find_or_create_default_plugin_config;
pub use ignore_rules::{
    batch_delete_ignore_rules, create_or_ignore_ignore_rule, delete_ignore_rule, list_ignore_rules,
};

use time::OffsetDateTime;
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_wire::DiscoveryResultsPayload;
use uuid::Uuid;

/// Error returned by autodiscovery query helpers.
#[derive(Debug, thiserror::Error)]
pub enum AutodiscoveryError {
    #[error("database error: {0}")]
    Db(sea_orm::DbErr),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<AutodiscoveryError>>;
impl_report_conversion!(sea_orm::DbErr => AutodiscoveryError::Db);

// -- Process discovery results --

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

        if result.discoveries.is_empty() {
            tracing::debug!(
                %agent_id,
                plugin_type = %result.plugin_type,
                "discovery plugin returned no items"
            );
            continue;
        }

        discovery_items::process_plugin_result(db, tenant_id, host_id, now, &result, &ignore_set)
            .await?;
    }

    Ok(())
}

// -- Shared test helpers --

#[cfg(all(test, feature = "db-sqlite"))]
#[allow(unreachable_pub)]
pub(crate) mod tests_common {
    use sea_orm::{
        ActiveModelTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait, Set,
    };
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
            plugin_type: Set("package_manager_homebrew".to_string()),
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
                plugin_type: Set("package_manager_homebrew".to_string()),
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

        process_discovery_results(&db, agent_id, tenant_id, host_id, payload)
            .await
            .expect("process must succeed");

        // No plugin_config should be created for package manager types.
        let configs = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::PluginType.eq("package_manager_homebrew"))
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
    /// the plugin config is reused and the host_software_items version is updated in place.
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
        process_discovery_results(&db, agent_id, tenant_id, host_id, make_payload("1.24.4"))
            .await
            .expect("first run");

        // Second run with an updated version.
        process_discovery_results(&db, agent_id, tenant_id, host_id, make_payload("1.24.5"))
            .await
            .expect("second run");

        // No plugin_config should be created for package manager types.
        let config_count = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::PluginType.eq("package_manager_homebrew"))
            .count(&db)
            .await
            .expect("count configs");
        assert_eq!(
            config_count, 0,
            "package managers no longer create plugin_configs"
        );

        // Still exactly one host_software_items link, but with the updated version.
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
            Some("1.24.5"),
            "installed version must be updated on second run"
        );
    }
}
