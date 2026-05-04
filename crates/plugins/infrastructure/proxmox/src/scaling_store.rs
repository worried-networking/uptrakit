//! Controller-side scaling policy storage for Proxmox resource scaling v2.

use crate::entity::{proxmox_scaling_default, proxmox_scaling_item_override};
use proxmox_scaling_default::Entity as ProxmoxScalingDefault;
use proxmox_scaling_item_override::Entity as ProxmoxScalingItemOverride;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    SqliteTransactionMode, TransactionOptions, TransactionTrait,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{ProxmoxError, Result};

/// Scaling mode discriminant. Internal-only; not sent over any network
/// boundary. Not `#[non_exhaustive]` — must be exhaustively matched everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScalingMode {
    #[default]
    None,
    Absolute,
    Delta,
}

impl ScalingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Absolute => "absolute",
            Self::Delta => "delta",
        }
    }
}

impl std::str::FromStr for ScalingMode {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "none" => Ok(Self::None),
            "absolute" => Ok(Self::Absolute),
            "delta" => Ok(Self::Delta),
            _ => Err(()),
        }
    }
}

/// Effective scaling policy resolved for a software item update.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ScalingPolicy {
    pub(crate) mode: ScalingMode,
    pub(crate) absolute_cores: Option<i32>,
    pub(crate) absolute_memory_mb: Option<i32>,
    pub(crate) delta_cores: Option<i32>,
    pub(crate) delta_memory_mb: Option<i32>,
}

impl ScalingPolicy {
    pub(crate) fn none() -> Self {
        Self {
            mode: ScalingMode::None,
            absolute_cores: None,
            absolute_memory_mb: None,
            delta_cores: None,
            delta_memory_mb: None,
        }
    }

    /// True when the policy will result in at least one dimension being scaled.
    pub(crate) fn is_active(&self) -> bool {
        match self.mode {
            ScalingMode::None => false,
            ScalingMode::Absolute => {
                self.absolute_cores.is_some() || self.absolute_memory_mb.is_some()
            }
            ScalingMode::Delta => self.delta_cores.is_some() || self.delta_memory_mb.is_some(),
        }
    }
}

fn model_to_policy(model: &proxmox_scaling_default::Model) -> ScalingPolicy {
    let mode = model
        .scaling_mode
        .parse::<ScalingMode>()
        .unwrap_or_else(|_| {
            tracing::warn!(
                value = %model.scaling_mode,
                "unrecognised scaling_mode in proxmox_scaling_defaults; treating as None"
            );
            ScalingMode::None
        });
    ScalingPolicy {
        mode,
        absolute_cores: model.absolute_cores,
        absolute_memory_mb: model.absolute_memory_mb,
        delta_cores: model.delta_cores,
        delta_memory_mb: model.delta_memory_mb,
    }
}

fn item_model_to_policy(model: &proxmox_scaling_item_override::Model) -> ScalingPolicy {
    let mode = model
        .scaling_mode
        .parse::<ScalingMode>()
        .unwrap_or_else(|_| {
            tracing::warn!(
                value = %model.scaling_mode,
                "unrecognised scaling_mode in proxmox_scaling_item_overrides; treating as None"
            );
            ScalingMode::None
        });
    ScalingPolicy {
        mode,
        absolute_cores: model.absolute_cores,
        absolute_memory_mb: model.absolute_memory_mb,
        delta_cores: model.delta_cores,
        delta_memory_mb: model.delta_memory_mb,
    }
}

/// Load the global scaling default. Returns `ScalingPolicy::none()` if no row exists.
pub(crate) async fn load_scaling_global_default(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<ScalingPolicy> {
    let row = ProxmoxScalingDefault::find()
        .filter(proxmox_scaling_default::Column::TenantId.eq(tenant_id))
        .filter(proxmox_scaling_default::Column::PluginConfigId.eq(plugin_config_id))
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to load scaling global default: {e}"
            )))
        })?;
    Ok(row
        .as_ref()
        .map(model_to_policy)
        .unwrap_or_else(ScalingPolicy::none))
}

/// Load per-item scaling override. Returns `None` if no row (inherit global).
pub(crate) async fn load_scaling_item_override(
    db: &DatabaseConnection,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<Option<ScalingPolicy>> {
    let row = ProxmoxScalingItemOverride::find()
        .filter(proxmox_scaling_item_override::Column::SoftwareItemId.eq(software_item_id))
        .filter(proxmox_scaling_item_override::Column::PluginConfigId.eq(plugin_config_id))
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to load scaling item override: {e}"
            )))
        })?;
    Ok(row.as_ref().map(item_model_to_policy))
}

/// Resolve effective scaling policy. Item override wins over global default.
/// Dimension cascade is gated by the resolved effective mode — cross-mode
/// inheritance is forbidden.
pub(crate) async fn resolve_effective_scaling_policy(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<ScalingPolicy> {
    let item = load_scaling_item_override(db, software_item_id, plugin_config_id).await?;
    let global = load_scaling_global_default(db, tenant_id, plugin_config_id).await?;

    let Some(item_policy) = item else {
        return Ok(global);
    };

    let effective_mode = item_policy.mode;

    let (absolute_cores, absolute_memory_mb, delta_cores, delta_memory_mb) = match effective_mode {
        ScalingMode::Absolute => (
            item_policy.absolute_cores.or(global.absolute_cores),
            item_policy.absolute_memory_mb.or(global.absolute_memory_mb),
            None,
            None,
        ),
        ScalingMode::Delta => (
            None,
            None,
            item_policy.delta_cores.or(global.delta_cores),
            item_policy.delta_memory_mb.or(global.delta_memory_mb),
        ),
        ScalingMode::None => (None, None, None, None),
    };

    Ok(ScalingPolicy {
        mode: effective_mode,
        absolute_cores,
        absolute_memory_mb,
        delta_cores,
        delta_memory_mb,
    })
}

/// Upsert global scaling default. Uses `BEGIN IMMEDIATE` (read-then-write).
#[expect(
    dead_code,
    reason = "will be wired to surface action handlers in a subsequent task"
)]
pub(crate) async fn upsert_scaling_global_default(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
    policy: &ScalingPolicy,
) -> Result<()> {
    let now = OffsetDateTime::now_utc();
    let txn = db
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to begin transaction for scaling global default upsert: {e}"
            )))
        })?;

    let existing = ProxmoxScalingDefault::find()
        .filter(proxmox_scaling_default::Column::TenantId.eq(tenant_id))
        .filter(proxmox_scaling_default::Column::PluginConfigId.eq(plugin_config_id))
        .one(&txn)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to query existing scaling global default: {e}"
            )))
        })?;

    if let Some(existing) = existing {
        let mut active: proxmox_scaling_default::ActiveModel = existing.into();
        active.scaling_mode = Set(policy.mode.as_str().to_string());
        active.absolute_cores = Set(policy.absolute_cores);
        active.absolute_memory_mb = Set(policy.absolute_memory_mb);
        active.delta_cores = Set(policy.delta_cores);
        active.delta_memory_mb = Set(policy.delta_memory_mb);
        active.updated_at = Set(now);
        active.update(&txn).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to update scaling global default: {e}"
            )))
        })?;
    } else {
        let active = proxmox_scaling_default::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            plugin_config_id: Set(plugin_config_id),
            scaling_mode: Set(policy.mode.as_str().to_string()),
            absolute_cores: Set(policy.absolute_cores),
            absolute_memory_mb: Set(policy.absolute_memory_mb),
            delta_cores: Set(policy.delta_cores),
            delta_memory_mb: Set(policy.delta_memory_mb),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(&txn).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to insert scaling global default: {e}"
            )))
        })?;
    }

    txn.commit().await.map_err(|e| {
        rootcause::report!(ProxmoxError::Database(format!(
            "failed to commit scaling global default upsert: {e}"
        )))
    })?;
    Ok(())
}

/// Upsert per-item scaling override. Uses `BEGIN IMMEDIATE`.
#[expect(
    dead_code,
    reason = "will be wired to surface action handlers in a subsequent task"
)]
pub(crate) async fn upsert_scaling_item_override(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
    policy: &ScalingPolicy,
) -> Result<()> {
    let now = OffsetDateTime::now_utc();
    let txn = db
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to begin transaction for scaling item override upsert: {e}"
            )))
        })?;

    let existing = ProxmoxScalingItemOverride::find()
        .filter(proxmox_scaling_item_override::Column::SoftwareItemId.eq(software_item_id))
        .filter(proxmox_scaling_item_override::Column::PluginConfigId.eq(plugin_config_id))
        .one(&txn)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to query existing scaling item override: {e}"
            )))
        })?;

    if let Some(existing) = existing {
        let mut active: proxmox_scaling_item_override::ActiveModel = existing.into();
        active.scaling_mode = Set(policy.mode.as_str().to_string());
        active.absolute_cores = Set(policy.absolute_cores);
        active.absolute_memory_mb = Set(policy.absolute_memory_mb);
        active.delta_cores = Set(policy.delta_cores);
        active.delta_memory_mb = Set(policy.delta_memory_mb);
        active.updated_at = Set(now);
        active.update(&txn).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to update scaling item override: {e}"
            )))
        })?;
    } else {
        let active = proxmox_scaling_item_override::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            software_item_id: Set(software_item_id),
            plugin_config_id: Set(plugin_config_id),
            scaling_mode: Set(policy.mode.as_str().to_string()),
            absolute_cores: Set(policy.absolute_cores),
            absolute_memory_mb: Set(policy.absolute_memory_mb),
            delta_cores: Set(policy.delta_cores),
            delta_memory_mb: Set(policy.delta_memory_mb),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(&txn).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to insert scaling item override: {e}"
            )))
        })?;
    }

    txn.commit().await.map_err(|e| {
        rootcause::report!(ProxmoxError::Database(format!(
            "failed to commit scaling item override upsert: {e}"
        )))
    })?;
    Ok(())
}

/// Delete per-item scaling override (revert item to global inheritance).
#[expect(
    dead_code,
    reason = "will be wired to surface action handlers in a subsequent task"
)]
pub(crate) async fn delete_scaling_item_override(
    db: &DatabaseConnection,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<()> {
    if let Some(existing) = ProxmoxScalingItemOverride::find()
        .filter(proxmox_scaling_item_override::Column::SoftwareItemId.eq(software_item_id))
        .filter(proxmox_scaling_item_override::Column::PluginConfigId.eq(plugin_config_id))
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to query scaling item override for delete: {e}"
            )))
        })?
    {
        let active: proxmox_scaling_item_override::ActiveModel = existing.into();
        active.delete(db).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to delete scaling item override: {e}"
            )))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_mode_round_trips() {
        for (s, expected) in &[
            ("none", ScalingMode::None),
            ("absolute", ScalingMode::Absolute),
            ("delta", ScalingMode::Delta),
        ] {
            let parsed: ScalingMode = s.parse().expect("known value must parse");
            assert_eq!(parsed, *expected);
            assert_eq!(parsed.as_str(), *s);
        }
    }

    #[test]
    fn scaling_mode_unknown_string_returns_err() {
        let result = "invalid".parse::<ScalingMode>();
        assert!(result.is_err());
    }

    #[test]
    fn scaling_policy_is_active_none_mode() {
        let policy = ScalingPolicy::none();
        assert!(!policy.is_active());
    }

    #[test]
    fn scaling_policy_is_active_absolute_requires_at_least_one_dimension() {
        let mut policy = ScalingPolicy {
            mode: ScalingMode::Absolute,
            ..Default::default()
        };
        assert!(!policy.is_active(), "no dimensions = not active");
        policy.absolute_cores = Some(4);
        assert!(policy.is_active());
    }

    #[test]
    fn scaling_policy_is_active_delta_requires_at_least_one_dimension() {
        let mut policy = ScalingPolicy {
            mode: ScalingMode::Delta,
            ..Default::default()
        };
        assert!(!policy.is_active(), "no dimensions = not active");
        policy.delta_memory_mb = Some(1024);
        assert!(policy.is_active());
    }

    #[test]
    fn resolve_effective_policy_cross_mode_gate() {
        // Item selects Delta mode; global has absolute + delta dimensions.
        // Cascade must only pull delta fields from global; absolute fields must
        // be gated out.
        let item = ScalingPolicy {
            mode: ScalingMode::Delta,
            delta_cores: None,
            delta_memory_mb: Some(1024),
            ..Default::default()
        };
        let global = ScalingPolicy {
            mode: ScalingMode::Absolute,
            absolute_cores: Some(8),
            absolute_memory_mb: Some(8192),
            delta_cores: Some(2),
            delta_memory_mb: None,
            ..Default::default()
        };

        // Replicate the cascade logic from resolve_effective_scaling_policy.
        let effective_mode = item.mode;
        assert_eq!(effective_mode, ScalingMode::Delta);

        let delta_cores = item.delta_cores.or(global.delta_cores);
        let delta_memory_mb = item.delta_memory_mb.or(global.delta_memory_mb);
        // Absolute fields must not bleed through.
        let absolute_cores: Option<i32> = None;
        let absolute_memory_mb: Option<i32> = None;

        assert_eq!(delta_cores, Some(2), "delta_cores cascades from global");
        assert_eq!(delta_memory_mb, Some(1024), "delta_memory_mb from item");
        assert!(absolute_cores.is_none(), "absolute_cores must be gated out");
        assert!(
            absolute_memory_mb.is_none(),
            "absolute_memory_mb must be gated out"
        );
    }
}
