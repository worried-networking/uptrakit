use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
};
use time::OffsetDateTime;
use uptrakit_plugin_infrastructure_registry::PluginOps;
use uptrakit_shared_macros::impl_report_conversion;
use uuid::Uuid;

use uptrakit_shared_db::entity::{notification_channel, notification_log, notification_rule};
use uptrakit_web_api_types::notifications::{
    NotificationChannelResponse, NotificationDeliveryStatus, NotificationEventType,
    NotificationLogResponse, NotificationRuleResponse,
};
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};

use crate::tenant_db::TenantDb;

// -- Channels -----------------------------------------------------------------

#[tracing::instrument(skip_all)]
pub async fn create_channel(
    tenant_db: &TenantDb,
    req: &uptrakit_web_api_types::notifications::CreateNotificationChannelRequest,
    plugin_ops: &dyn PluginOps,
) -> ChannelResult<NotificationChannelResponse> {
    use uptrakit_shared_types::PluginTypeId;
    let channel_type_id = PluginTypeId::new(&req.channel_type);

    // Validate config with channel implementation
    if plugin_ops.transport(&channel_type_id).is_none() {
        return Err(report!(ChannelQueryError::UnsupportedType(
            req.channel_type.clone()
        )));
    }

    plugin_ops
        .validate_config(&channel_type_id, &req.config)
        .map_err(|e| report!(ChannelQueryError::InvalidConfig(e.to_string())))?;

    let config_str = serde_json::to_string(&req.config)
        .map_err(|e| report!(ChannelQueryError::Db(sea_orm::DbErr::Custom(e.to_string()))))?;

    let now = OffsetDateTime::now_utc();
    let id = Uuid::now_v7();

    let encrypted_config =
        uptrakit_crypto::EncryptedString::new(config_str, "uptrakit:notification_channels:config")
            .map_err(|e| report!(ChannelQueryError::Db(sea_orm::DbErr::Custom(e.to_string()))))?;

    let model = notification_channel::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_db.tenant_id),
        name: Set(req.name.clone()),
        channel_type: Set(req.channel_type.clone()),
        config: Set(encrypted_config),
        enabled: Set(req.enabled),
        created_at: Set(now),
        updated_at: Set(now),
    };

    let result = model.insert(tenant_db.db()).await.context_to()?;

    // Return with masked config
    let masked_config = plugin_ops.mask_config_secrets(&channel_type_id, &req.config);
    Ok(channel_to_response(result, masked_config))
}

#[tracing::instrument(skip_all)]
pub async fn list_channels(
    tenant_db: &TenantDb,
    params: &PaginationParams,
    plugin_ops: &dyn PluginOps,
) -> ChannelResult<PaginatedResponse<NotificationChannelResponse>> {
    let resolved = params.resolve();
    let total = tenant_db
        .find::<notification_channel::Entity>()
        .count(tenant_db.db())
        .await
        .context_to()?;

    let channels = tenant_db
        .find::<notification_channel::Entity>()
        .order_by_desc(notification_channel::Column::CreatedAt)
        .offset(Some(resolved.offset()))
        .limit(Some(resolved.per_page))
        .all(tenant_db.db())
        .await
        .context_to()?;

    let items = channels
        .into_iter()
        .map(|ch| {
            let masked = mask_channel_config(&ch, plugin_ops);
            channel_to_response(ch, masked)
        })
        .collect();

    Ok(PaginatedResponse::new(items, total, resolved))
}

#[tracing::instrument(skip_all, fields(%id))]
pub async fn get_channel(
    tenant_db: &TenantDb,
    id: Uuid,
    plugin_ops: &dyn PluginOps,
) -> ChannelResult<Option<NotificationChannelResponse>> {
    let channel = tenant_db
        .find_by_id::<notification_channel::Entity, _>(id)
        .one(tenant_db.db())
        .await
        .context_to()?;

    Ok(channel.map(|ch| {
        let masked = mask_channel_config(&ch, plugin_ops);
        channel_to_response(ch, masked)
    }))
}

#[tracing::instrument(skip_all, fields(%id))]
pub async fn update_channel(
    tenant_db: &TenantDb,
    id: Uuid,
    req: &uptrakit_web_api_types::notifications::UpdateNotificationChannelRequest,
    plugin_ops: &dyn PluginOps,
) -> ChannelResult<Option<NotificationChannelResponse>> {
    let existing = tenant_db
        .find_by_id::<notification_channel::Entity, _>(id)
        .one(tenant_db.db())
        .await
        .context_to()?;

    let Some(existing) = existing else {
        return Ok(None);
    };

    let mut active: notification_channel::ActiveModel = existing.clone().into();
    active.updated_at = Set(OffsetDateTime::now_utc());

    if let Some(name) = &req.name {
        active.name = Set(name.clone());
    }
    if let Some(config) = &req.config {
        // Validate with channel impl
        let channel_type_id = uptrakit_shared_types::PluginTypeId::new(&existing.channel_type);
        plugin_ops
            .validate_config(&channel_type_id, config)
            .map_err(|e| report!(ChannelQueryError::InvalidConfig(e.to_string())))?;
        let config_str = serde_json::to_string(config)
            .map_err(|e| report!(ChannelQueryError::Db(sea_orm::DbErr::Custom(e.to_string()))))?;
        let encrypted_config = uptrakit_crypto::EncryptedString::new(
            config_str,
            "uptrakit:notification_channels:config",
        )
        .map_err(|e| report!(ChannelQueryError::Db(sea_orm::DbErr::Custom(e.to_string()))))?;
        active.config = Set(encrypted_config);
    }
    if let Some(enabled) = req.enabled {
        active.enabled = Set(enabled);
    }

    let result = active.update(tenant_db.db()).await.context_to()?;
    let masked = mask_channel_config(&result, plugin_ops);
    Ok(Some(channel_to_response(result, masked)))
}

#[tracing::instrument(skip_all, fields(%id))]
pub async fn delete_channel(tenant_db: &TenantDb, id: Uuid) -> ChannelResult<bool> {
    let result = tenant_db
        .delete_many::<notification_channel::Entity>()
        .filter(notification_channel::Column::Id.eq(id))
        .exec(tenant_db.db())
        .await
        .context_to()?;

    Ok(result.rows_affected > 0)
}

// -- Rules --------------------------------------------------------------------

#[tracing::instrument(skip_all)]
pub async fn create_rule(
    tenant_db: &TenantDb,
    req: &uptrakit_web_api_types::notifications::CreateNotificationRuleRequest,
) -> RuleResult<NotificationRuleResponse> {
    // Verify channel belongs to tenant
    let channel = tenant_db
        .find_by_id::<notification_channel::Entity, _>(req.channel_id)
        .one(tenant_db.db())
        .await
        .context_to()?;

    if channel.is_none() {
        bail!(RuleQueryError::ChannelNotFound);
    }

    let id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();

    let model = notification_rule::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_db.tenant_id),
        channel_id: Set(req.channel_id),
        event_type: Set(req.event_type.as_str().to_string()),
        host_id: Set(req.host_id),
        software_item_id: Set(req.software_item_id),
        plugin_type: Set(req.plugin_type.clone()),
        enabled: Set(req.enabled),
        created_at: Set(now),
    };

    let result = model.insert(tenant_db.db()).await.context_to()?;
    Ok(rule_to_response(result))
}

#[tracing::instrument(skip_all)]
pub async fn list_rules(
    tenant_db: &TenantDb,
    params: &PaginationParams,
    channel_id_filter: Option<Uuid>,
    event_type_filter: Option<&str>,
) -> RuleResult<PaginatedResponse<NotificationRuleResponse>> {
    let resolved = params.resolve();
    let mut query = tenant_db.find::<notification_rule::Entity>();

    if let Some(channel_id) = channel_id_filter {
        query = query.filter(notification_rule::Column::ChannelId.eq(channel_id));
    }
    if let Some(event_type) = event_type_filter {
        query = query.filter(notification_rule::Column::EventType.eq(event_type));
    }

    let count_query = query.clone();
    let total = count_query.count(tenant_db.db()).await.context_to()?;

    let rules = query
        .order_by_desc(notification_rule::Column::CreatedAt)
        .offset(Some(resolved.offset()))
        .limit(Some(resolved.per_page))
        .all(tenant_db.db())
        .await
        .context_to()?;

    let items = rules.into_iter().map(rule_to_response).collect();
    Ok(PaginatedResponse::new(items, total, resolved))
}

#[tracing::instrument(skip_all, fields(%id))]
pub async fn get_rule(
    tenant_db: &TenantDb,
    id: Uuid,
) -> RuleResult<Option<NotificationRuleResponse>> {
    let rule = tenant_db
        .find_by_id::<notification_rule::Entity, _>(id)
        .one(tenant_db.db())
        .await
        .context_to()?;

    Ok(rule.map(rule_to_response))
}

#[tracing::instrument(skip_all, fields(%id))]
pub async fn update_rule(
    tenant_db: &TenantDb,
    id: Uuid,
    req: &uptrakit_web_api_types::notifications::UpdateNotificationRuleRequest,
) -> RuleResult<Option<NotificationRuleResponse>> {
    let existing = tenant_db
        .find_by_id::<notification_rule::Entity, _>(id)
        .one(tenant_db.db())
        .await
        .context_to()?;

    let Some(existing) = existing else {
        return Ok(None);
    };

    let mut active: notification_rule::ActiveModel = existing.into();

    if let Some(event_type) = &req.event_type {
        active.event_type = Set(event_type.as_str().to_string());
    }
    if let Some(enabled) = req.enabled {
        active.enabled = Set(enabled);
    }

    // Scope filters use the Option<serde_json::Value> nullable-update pattern:
    //   absent (None)          → keep current value
    //   Some(Value::Null)      → clear to NULL
    //   Some(Value::String(s)) → set to the parsed value
    if let Some(val) = &req.host_id {
        match val {
            serde_json::Value::Null => active.host_id = Set(None),
            serde_json::Value::String(s) => {
                let id = Uuid::parse_str(s)
                    .map_err(|_| report!(RuleQueryError::InvalidField("host_id".to_string())))?;
                active.host_id = Set(Some(id));
            }
            _ => bail!(RuleQueryError::InvalidField("host_id".to_string())),
        }
    }
    if let Some(val) = &req.software_item_id {
        match val {
            serde_json::Value::Null => active.software_item_id = Set(None),
            serde_json::Value::String(s) => {
                let id = Uuid::parse_str(s).map_err(|_| {
                    report!(RuleQueryError::InvalidField("software_item_id".to_string()))
                })?;
                active.software_item_id = Set(Some(id));
            }
            _ => bail!(RuleQueryError::InvalidField("software_item_id".to_string())),
        }
    }
    if let Some(val) = &req.plugin_type {
        match val {
            serde_json::Value::Null => active.plugin_type = Set(None),
            serde_json::Value::String(s) => active.plugin_type = Set(Some(s.clone())),
            _ => bail!(RuleQueryError::InvalidField("plugin_type".to_string())),
        }
    }

    let result = active.update(tenant_db.db()).await.context_to()?;
    Ok(Some(rule_to_response(result)))
}

#[tracing::instrument(skip_all, fields(%id))]
pub async fn delete_rule(tenant_db: &TenantDb, id: Uuid) -> RuleResult<bool> {
    let result = tenant_db
        .delete_many::<notification_rule::Entity>()
        .filter(notification_rule::Column::Id.eq(id))
        .exec(tenant_db.db())
        .await
        .context_to()?;

    Ok(result.rows_affected > 0)
}

// -- Log ----------------------------------------------------------------------

#[tracing::instrument(skip_all)]
pub async fn list_log(
    tenant_db: &TenantDb,
    params: &PaginationParams,
) -> ChannelResult<PaginatedResponse<NotificationLogResponse>> {
    let resolved = params.resolve();

    let total = tenant_db
        .find::<notification_log::Entity>()
        .count(tenant_db.db())
        .await
        .context_to()?;

    let logs = tenant_db
        .find::<notification_log::Entity>()
        .order_by_desc(notification_log::Column::CreatedAt)
        .offset(Some(resolved.offset()))
        .limit(Some(resolved.per_page))
        .all(tenant_db.db())
        .await
        .context_to()?;

    let items = logs.into_iter().map(log_to_response).collect();
    Ok(PaginatedResponse::new(items, total, resolved))
}

/// Look up a notification log entry by its action token.
///
/// # Cross-tenant design
///
/// This function intentionally takes a raw `DatabaseConnection` rather than
/// `TenantDb`. Telegram webhook callbacks arrive without any tenant
/// authentication context; the only identifier available is the `action_token`
/// embedded in the `callback_query.data` field. Because `action_token` is a
/// randomly generated UUID assigned to a single notification event (enforced
/// by the unique index `idx_notification_log_action_token`), knowledge of it
/// is sufficient proof of legitimacy. The tenant can be derived from the
/// returned `notification_log::Model::tenant_id` field if needed for
/// subsequent operations.
#[tracing::instrument(skip_all)]
pub async fn find_log_by_action_token(
    db: &sea_orm::DatabaseConnection,
    action_token: Uuid,
) -> ChannelResult<Option<notification_log::Model>> {
    notification_log::Entity::find()
        .filter(notification_log::Column::ActionToken.eq(action_token))
        .one(db)
        .await
        .context_to()
}

// -- Error types --------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ChannelQueryError {
    #[error("database error: {0}")]
    Db(sea_orm::DbErr),
    #[error("unsupported channel type: {0}")]
    UnsupportedType(String),
    #[error("invalid channel config: {0}")]
    InvalidConfig(String),
}

pub type ChannelResult<T> = std::result::Result<T, rootcause::Report<ChannelQueryError>>;
impl_report_conversion!(sea_orm::DbErr => ChannelQueryError::Db);

#[derive(Debug, thiserror::Error)]
pub enum RuleQueryError {
    #[error("database error: {0}")]
    Db(sea_orm::DbErr),
    #[error("channel not found")]
    ChannelNotFound,
    #[error("invalid value for field '{0}'")]
    InvalidField(String),
}

pub type RuleResult<T> = std::result::Result<T, rootcause::Report<RuleQueryError>>;
impl_report_conversion!(sea_orm::DbErr => RuleQueryError::Db);

// -- Helpers ------------------------------------------------------------------

fn channel_to_response(
    model: notification_channel::Model,
    masked_config: serde_json::Value,
) -> NotificationChannelResponse {
    NotificationChannelResponse {
        id: model.id,
        name: model.name,
        channel_type: model.channel_type,
        config: masked_config,
        enabled: model.enabled,
        created_at: model.created_at,
        updated_at: model.updated_at,
    }
}

fn mask_channel_config(
    channel: &notification_channel::Model,
    plugin_ops: &dyn PluginOps,
) -> serde_json::Value {
    let config: serde_json::Value =
        serde_json::from_str(channel.config.expose_secret()).unwrap_or_default();

    let channel_type_id = uptrakit_shared_types::PluginTypeId::new(&channel.channel_type);
    if plugin_ops.transport(&channel_type_id).is_some() {
        plugin_ops.mask_config_secrets(&channel_type_id, &config)
    } else {
        // Unknown channel type -- mask all values
        serde_json::json!({})
    }
}

fn rule_to_response(model: notification_rule::Model) -> NotificationRuleResponse {
    let event_type = model
        .event_type
        .parse::<NotificationEventType>()
        .unwrap_or_else(|_| {
            tracing::warn!(
                event_type = %model.event_type,
                "unknown event type; defaulting to UpdateAvailable"
            );
            NotificationEventType::UpdateAvailable
        });

    NotificationRuleResponse {
        id: model.id,
        channel_id: model.channel_id,
        event_type,
        host_id: model.host_id,
        software_item_id: model.software_item_id,
        plugin_type: model.plugin_type,
        enabled: model.enabled,
        created_at: model.created_at,
    }
}

fn log_to_response(model: notification_log::Model) -> NotificationLogResponse {
    let event_type = model
        .event_type
        .parse::<NotificationEventType>()
        .unwrap_or_else(|_| {
            tracing::warn!(
                event_type = %model.event_type,
                "unknown event type in log; defaulting to UpdateAvailable"
            );
            NotificationEventType::UpdateAvailable
        });

    let status = model
        .status
        .parse::<NotificationDeliveryStatus>()
        .unwrap_or_else(|_| {
            tracing::warn!(
                status = %model.status,
                "unknown delivery status; defaulting to Pending"
            );
            NotificationDeliveryStatus::Pending
        });

    NotificationLogResponse {
        id: model.id,
        channel_id: model.channel_id,
        rule_id: model.rule_id,
        event_type,
        event_payload: model.event_payload,
        status,
        error_message: model.error_message,
        action_token: model.action_token,
        action_taken: model.action_taken,
        created_at: model.created_at,
        delivered_at: model.delivered_at,
    }
}
