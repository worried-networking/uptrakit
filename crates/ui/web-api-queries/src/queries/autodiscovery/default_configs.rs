//! Auto-creation of default plugin configs from discovery targets.

use super::{AutodiscoveryError, Result};
use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{plugin_config, prelude::*};
use uptrakit_shared_db::is_unique_constraint_violation;
use uuid::Uuid;

/// Find or create a plugin config matched by `(plugin_type, name)`.
///
/// Lookup order:
///
/// 1. **Name match, JSON match** -- returns the existing ID unchanged.
/// 2. **Name match, JSON differs** -- updates the config JSON in-place and
///    returns the same ID. This is the self-healing path for plugin updates
///    that change default command templates (e.g. adding `sudo` to a PHS
///    update command after commit `8695cbc`): existing role assignments that
///    reference the config by ID automatically pick up the new command on the
///    next discovery run without requiring manual re-linking.
/// 3. **No match** -- creates a new config row.
///
/// Idempotent and safe under concurrent calls: the `uq_plugin_configs_active_name`
/// partial unique index (`WHERE deactivated_at IS NULL`) guarantees that at most one
/// active config with a given `(tenant_id, name)` pair exists at any time. On a
/// unique-constraint violation (two concurrent auto-creates racing), the function
/// re-queries by name and returns the winner's ID.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn find_or_create_default_plugin_config(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    plugin_type: &str,
    config_json: &serde_json::Value,
    display_name: &str,
) -> Result<Uuid> {
    // Search by the natural identity key: (tenant_id, plugin_type, name).
    // Matching on name -- rather than JSON content -- means that when a plugin
    // update rewrites a default command template, the existing row is updated
    // in-place so all current role assignments automatically pick up the change.
    let existing = PluginConfig::find()
        .filter(plugin_config::Column::TenantId.eq(tenant_id))
        .filter(plugin_config::Column::PluginType.eq(plugin_type))
        .filter(plugin_config::Column::Name.eq(display_name))
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()?;

    if let Some(cfg) = existing {
        let id = cfg.id;
        if &cfg.config == config_json {
            // Config is already up-to-date.
            return Ok(id);
        }
        // Config JSON has changed. Update in-place so existing role assignments
        // referencing this ID continue to work with the new configuration.
        let now = OffsetDateTime::now_utc();
        let mut active: plugin_config::ActiveModel = cfg.into();
        active.config = Set(config_json.clone());
        active.updated_at = Set(now);
        active.update(db).await.context_to()?;
        tracing::debug!(
            %id,
            plugin_type = %plugin_type,
            name = %display_name,
            "updated auto-generated plugin config to reflect new defaults"
        );
        return Ok(id);
    }

    // None found -- try to create one.
    let now = OffsetDateTime::now_utc();
    let new_id = Uuid::now_v7();
    let record = plugin_config::ActiveModel {
        id: Set(new_id),
        tenant_id: Set(tenant_id),
        name: Set(display_name.to_string()),
        plugin_type: Set(plugin_type.to_string()),
        config: Set(config_json.clone()),
        enabled: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    };

    match PluginConfig::insert(record).exec(db).await {
        Ok(_) => Ok(new_id),
        Err(e) if is_unique_constraint_violation(&e) => {
            // A concurrent task created this config at the same time.
            // Re-query by name to get the winner's ID.
            PluginConfig::find()
                .filter(plugin_config::Column::TenantId.eq(tenant_id))
                .filter(plugin_config::Column::PluginType.eq(plugin_type))
                .filter(plugin_config::Column::Name.eq(display_name))
                .filter(plugin_config::Column::DeactivatedAt.is_null())
                .one(db)
                .await
                .context_to()?
                .map(|c| c.id)
                .ok_or_else(|| report!(AutodiscoveryError::Db(e)))
        }
        Err(e) => Err(report!(AutodiscoveryError::Db(e))),
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use crate::queries::autodiscovery::tests_common::{insert_tenant, setup_db};
    use sea_orm::PaginatorTrait;

    /// When a plugin config with the same `(plugin_type, name)` already exists
    /// but with different JSON, `find_or_create_default_plugin_config` must
    /// update the config in-place and return the original ID.
    ///
    /// This is the self-healing mechanism for the case where a plugin update
    /// changes default command templates -- e.g. `8695cbc` rewrote the PHS
    /// update command from `"env PHS_SILENT=1 /usr/bin/update"` (runs without
    /// root and fails) to `"sudo /usr/local/bin/uptrakit-phs-update"`. Without
    /// this in-place update, existing role assignments would keep pointing to
    /// the old config ID and continue executing the broken command.
    #[tokio::test]
    async fn find_or_create_updates_config_json_on_name_match() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;

        let old_config = serde_json::json!({
            "update_command": "env PHS_SILENT=1 /usr/bin/update",
        });
        let new_config = serde_json::json!({
            "update_command": "sudo /usr/local/bin/uptrakit-phs-update",
        });

        // Create the config with the old JSON.
        let first_id = find_or_create_default_plugin_config(
            &db,
            tenant_id,
            "generic_shell",
            &old_config,
            "PHS Shell",
        )
        .await
        .expect("create first");

        // Call again with the same name but updated JSON.
        let second_id = find_or_create_default_plugin_config(
            &db,
            tenant_id,
            "generic_shell",
            &new_config,
            "PHS Shell",
        )
        .await
        .expect("update in-place");

        // Must return the same ID -- no new row.
        assert_eq!(
            first_id, second_id,
            "must return the existing config ID, not create a new one"
        );

        // The stored config must reflect the new JSON.
        let stored = PluginConfig::find()
            .filter(plugin_config::Column::Id.eq(first_id))
            .one(&db)
            .await
            .expect("query config")
            .expect("config must still exist");
        assert_eq!(
            stored.config, new_config,
            "config JSON must be updated in-place"
        );

        // Exactly one active config with this name must exist.
        let count = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::Name.eq("PHS Shell"))
            .filter(plugin_config::Column::DeactivatedAt.is_null())
            .count(&db)
            .await
            .expect("count");
        assert_eq!(count, 1, "must not create a duplicate config");
    }

    /// Calling `find_or_create_default_plugin_config` twice with identical
    /// `(name, JSON)` must return the same ID and leave exactly one row.
    #[tokio::test]
    async fn find_or_create_is_idempotent_when_json_unchanged() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;

        let config = serde_json::json!({"tag_strip_prefix": "v"});

        let id1 = find_or_create_default_plugin_config(
            &db,
            tenant_id,
            "releases_github",
            &config,
            "GitHub Releases",
        )
        .await
        .expect("first call");

        let id2 = find_or_create_default_plugin_config(
            &db,
            tenant_id,
            "releases_github",
            &config,
            "GitHub Releases",
        )
        .await
        .expect("second call");

        assert_eq!(id1, id2, "must return the same ID on repeated calls");

        let count = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::Name.eq("GitHub Releases"))
            .filter(plugin_config::Column::DeactivatedAt.is_null())
            .count(&db)
            .await
            .expect("count");
        assert_eq!(count, 1, "must not create duplicate rows");
    }
}
