//! Auto-creation of default plugin configs from discovery targets.

use super::{AutodiscoveryError, Result};
use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use time::OffsetDateTime;
use uptrakit_plugin_infrastructure_registry::PluginConfigOps;
use uptrakit_shared_db::encrypted_columns::EncryptedPluginConfig;
use uptrakit_shared_db::entity::{plugin_config, prelude::*};
use uptrakit_shared_db::is_unique_constraint_violation;
use uptrakit_shared_types::PluginTypeId;
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
///
/// `credential_updated_at` is stamped honestly on both paths: on overwrite,
/// only when a sensitive path actually differs from the stored value; on
/// insert, only when `config_json` holds a live secret. This path has NO
/// restore step -- there is nothing to restore from on a fresh discovery
/// report, and on overwrite `config_json` fully replaces the stored value.
/// An agent report that omits a previously-reported secret therefore drops
/// the credential from the stored config, and that removal is honestly
/// stamped like any other credential change (spec §8) -- there is no masked
/// echo-back path here the way there is for the REST writers.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn find_or_create_default_plugin_config(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    plugin_type: &str,
    config_json: &serde_json::Value,
    display_name: &str,
    ops: &dyn PluginConfigOps,
) -> Result<Uuid> {
    let type_id = PluginTypeId::new(plugin_type);

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
        if cfg.config.as_json() == config_json {
            // Config is already up-to-date.
            return Ok(id);
        }
        // Config JSON has changed. Update in-place so existing role assignments
        // referencing this ID continue to work with the new configuration.
        let stored_config = cfg.config.as_json().clone();
        let now = OffsetDateTime::now_utc();
        let mut active: plugin_config::ActiveModel = cfg.into();
        active.config = Set(EncryptedPluginConfig::from_json(config_json)
            .map_err(|e| report!(AutodiscoveryError::Encryption(e.to_string())))?);
        active.updated_at = Set(now);
        if ops.sensitive_value_changed_for(&type_id, config_json, &stored_config) {
            active.credential_updated_at = Set(Some(now));
        }
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
        config: Set(EncryptedPluginConfig::from_json(config_json)
            .map_err(|e| report!(AutodiscoveryError::Encryption(e.to_string())))?),
        enabled: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
        credential_updated_at: Set(ops.has_live_secret_in(&type_id, config_json).then_some(now)),
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
    use crate::queries::autodiscovery::tests_common::{
        NoopOps, insert_tenant, real_plugin_ops, setup_db,
    };
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
            "generic.shell",
            &old_config,
            "PHS Shell",
            &NoopOps,
        )
        .await
        .expect("create first");

        // Call again with the same name but updated JSON.
        let second_id = find_or_create_default_plugin_config(
            &db,
            tenant_id,
            "generic.shell",
            &new_config,
            "PHS Shell",
            &NoopOps,
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
            "releases.github",
            &config,
            "GitHub Releases",
            &NoopOps,
        )
        .await
        .expect("first call");

        let id2 = find_or_create_default_plugin_config(
            &db,
            tenant_id,
            "releases.github",
            &config,
            "GitHub Releases",
            &NoopOps,
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

    /// `credential_updated_at` must be stamped honestly across all three
    /// shapes this writer can take: insert-with-secret, insert-without,
    /// and overwrite-with-changed-secret vs. overwrite-with-unchanged-config.
    #[tokio::test]
    async fn default_config_writer_stamps_changes() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        let ops = real_plugin_ops();

        // Insert without a live secret -> not stamped.
        let no_secret = serde_json::json!({"tag_strip_prefix": "v"});
        let id_no_secret = find_or_create_default_plugin_config(
            &db,
            tenant_id,
            "releases.github",
            &no_secret,
            "GitHub No Token",
            &ops,
        )
        .await
        .expect("insert without secret");
        let stored = PluginConfig::find()
            .filter(plugin_config::Column::Id.eq(id_no_secret))
            .one(&db)
            .await
            .expect("query")
            .expect("row exists");
        assert_eq!(
            stored.credential_updated_at, None,
            "insert without a live secret must not stamp"
        );

        // Insert with a live secret -> stamped.
        let with_secret = serde_json::json!({"auth_token": "ghp_live"});
        let id_with_secret = find_or_create_default_plugin_config(
            &db,
            tenant_id,
            "releases.github",
            &with_secret,
            "GitHub With Token",
            &ops,
        )
        .await
        .expect("insert with secret");
        let stored = PluginConfig::find()
            .filter(plugin_config::Column::Id.eq(id_with_secret))
            .one(&db)
            .await
            .expect("query")
            .expect("row exists");
        let first_stamp = stored
            .credential_updated_at
            .expect("insert with a live secret must stamp");

        // Overwrite with an unchanged config -> timestamp untouched. Note:
        // `find_or_create_default_plugin_config` short-circuits on an exact
        // JSON match before reaching the update branch, so this call is a
        // pure no-op -- the assertion still pins the "unchanged" behavior.
        find_or_create_default_plugin_config(
            &db,
            tenant_id,
            "releases.github",
            &with_secret,
            "GitHub With Token",
            &ops,
        )
        .await
        .expect("re-call with identical config");
        let stored = PluginConfig::find()
            .filter(plugin_config::Column::Id.eq(id_with_secret))
            .one(&db)
            .await
            .expect("query")
            .expect("row exists");
        assert_eq!(
            stored.credential_updated_at,
            Some(first_stamp),
            "unchanged config must not re-stamp"
        );

        // Overwrite with a changed secret -> newly stamped.
        let changed_secret = serde_json::json!({"auth_token": "ghp_rotated"});
        find_or_create_default_plugin_config(
            &db,
            tenant_id,
            "releases.github",
            &changed_secret,
            "GitHub With Token",
            &ops,
        )
        .await
        .expect("overwrite with changed secret");
        let stored = PluginConfig::find()
            .filter(plugin_config::Column::Id.eq(id_with_secret))
            .one(&db)
            .await
            .expect("query")
            .expect("row exists");
        assert_ne!(
            stored.credential_updated_at,
            Some(first_stamp),
            "changed secret must produce a new credential_updated_at stamp, not reuse the prior one"
        );
        assert_ne!(
            stored.config.as_json(),
            &with_secret,
            "sanity: config must actually have changed"
        );
    }
}
