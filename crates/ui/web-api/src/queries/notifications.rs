use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
};
use time::OffsetDateTime;
use uuid::Uuid;

use uptrakit_shared_db::entity::{notification_channel, notification_log, notification_rule};
use uptrakit_web_api_types::notifications::{
    NotificationChannelResponse, NotificationChannelType, NotificationDeliveryStatus,
    NotificationEventType, NotificationLogResponse, NotificationRuleResponse,
};
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};

use crate::tenant_db::TenantDb;

// -- Channels -----------------------------------------------------------------

pub async fn create_channel(
    tenant_db: &TenantDb,
    req: &uptrakit_web_api_types::notifications::CreateNotificationChannelRequest,
    channel_registry: &uptrakit_notification_channels::ChannelRegistry,
) -> Result<NotificationChannelResponse, ChannelQueryError> {
    // Validate config with channel implementation
    let channel_type_str = req.channel_type.as_str();
    let channel_impl = channel_registry
        .get(channel_type_str)
        .ok_or_else(|| ChannelQueryError::UnsupportedType(channel_type_str.to_string()))?;

    channel_impl
        .validate_config(&req.config)
        .map_err(|e| ChannelQueryError::InvalidConfig(e.to_string()))?;

    let config_str = serde_json::to_string(&req.config)
        .map_err(|e| ChannelQueryError::Db(sea_orm::DbErr::Custom(e.to_string())))?;

    let now = OffsetDateTime::now_utc();
    let id = Uuid::now_v7();

    let encrypted_config = uptrakit_crypto::EncryptedString::new(config_str)
        .map_err(|e| ChannelQueryError::Db(sea_orm::DbErr::Custom(e.to_string())))?;

    let model = notification_channel::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_db.tenant_id),
        name: Set(req.name.clone()),
        channel_type: Set(channel_type_str.to_string()),
        config: Set(encrypted_config),
        enabled: Set(req.enabled),
        created_at: Set(now),
        updated_at: Set(now),
    };

    let result = model
        .insert(tenant_db.db())
        .await
        .map_err(ChannelQueryError::Db)?;

    // Return with masked config
    let masked_config = channel_impl.mask_config_secrets(&req.config);
    Ok(channel_to_response(result, masked_config))
}

pub async fn list_channels(
    tenant_db: &TenantDb,
    params: &PaginationParams,
    channel_registry: &uptrakit_notification_channels::ChannelRegistry,
) -> Result<PaginatedResponse<NotificationChannelResponse>, sea_orm::DbErr> {
    let resolved = params.resolve();
    let total = tenant_db
        .find::<notification_channel::Entity>()
        .count(tenant_db.db())
        .await?;

    let channels = tenant_db
        .find::<notification_channel::Entity>()
        .order_by_desc(notification_channel::Column::CreatedAt)
        .offset(Some(resolved.offset()))
        .limit(Some(resolved.per_page))
        .all(tenant_db.db())
        .await?;

    let items = channels
        .into_iter()
        .map(|ch| {
            let masked = mask_channel_config(&ch, channel_registry);
            channel_to_response(ch, masked)
        })
        .collect();

    Ok(PaginatedResponse::new(items, total, resolved))
}

pub async fn get_channel(
    tenant_db: &TenantDb,
    id: Uuid,
    channel_registry: &uptrakit_notification_channels::ChannelRegistry,
) -> Result<Option<NotificationChannelResponse>, sea_orm::DbErr> {
    let channel = tenant_db
        .find_by_id::<notification_channel::Entity, _>(id)
        .one(tenant_db.db())
        .await?;

    Ok(channel.map(|ch| {
        let masked = mask_channel_config(&ch, channel_registry);
        channel_to_response(ch, masked)
    }))
}

pub async fn update_channel(
    tenant_db: &TenantDb,
    id: Uuid,
    req: &uptrakit_web_api_types::notifications::UpdateNotificationChannelRequest,
    channel_registry: &uptrakit_notification_channels::ChannelRegistry,
) -> Result<Option<NotificationChannelResponse>, ChannelQueryError> {
    let existing = tenant_db
        .find_by_id::<notification_channel::Entity, _>(id)
        .one(tenant_db.db())
        .await
        .map_err(ChannelQueryError::Db)?;

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
        if let Some(channel_impl) = channel_registry.get(&existing.channel_type) {
            channel_impl
                .validate_config(config)
                .map_err(|e| ChannelQueryError::InvalidConfig(e.to_string()))?;
        }
        let config_str = serde_json::to_string(config)
            .map_err(|e| ChannelQueryError::Db(sea_orm::DbErr::Custom(e.to_string())))?;
        let encrypted_config = uptrakit_crypto::EncryptedString::new(config_str)
            .map_err(|e| ChannelQueryError::Db(sea_orm::DbErr::Custom(e.to_string())))?;
        active.config = Set(encrypted_config);
    }
    if let Some(enabled) = req.enabled {
        active.enabled = Set(enabled);
    }

    let result = active
        .update(tenant_db.db())
        .await
        .map_err(ChannelQueryError::Db)?;
    let masked = mask_channel_config(&result, channel_registry);
    Ok(Some(channel_to_response(result, masked)))
}

pub async fn delete_channel(tenant_db: &TenantDb, id: Uuid) -> Result<bool, sea_orm::DbErr> {
    let result = tenant_db
        .delete_many::<notification_channel::Entity>()
        .filter(notification_channel::Column::Id.eq(id))
        .exec(tenant_db.db())
        .await?;

    Ok(result.rows_affected > 0)
}

// -- Rules --------------------------------------------------------------------

pub async fn create_rule(
    tenant_db: &TenantDb,
    req: &uptrakit_web_api_types::notifications::CreateNotificationRuleRequest,
) -> Result<NotificationRuleResponse, RuleQueryError> {
    // Verify channel belongs to tenant
    let channel = tenant_db
        .find_by_id::<notification_channel::Entity, _>(req.channel_id)
        .one(tenant_db.db())
        .await
        .map_err(RuleQueryError::Db)?;

    if channel.is_none() {
        return Err(RuleQueryError::ChannelNotFound);
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

    let result = model
        .insert(tenant_db.db())
        .await
        .map_err(RuleQueryError::Db)?;
    Ok(rule_to_response(result))
}

pub async fn list_rules(
    tenant_db: &TenantDb,
    params: &PaginationParams,
    channel_id_filter: Option<Uuid>,
    event_type_filter: Option<&str>,
) -> Result<PaginatedResponse<NotificationRuleResponse>, sea_orm::DbErr> {
    let resolved = params.resolve();
    let mut query = tenant_db.find::<notification_rule::Entity>();

    if let Some(channel_id) = channel_id_filter {
        query = query.filter(notification_rule::Column::ChannelId.eq(channel_id));
    }
    if let Some(event_type) = event_type_filter {
        query = query.filter(notification_rule::Column::EventType.eq(event_type));
    }

    let count_query = query.clone();
    let total = count_query.count(tenant_db.db()).await?;

    let rules = query
        .order_by_desc(notification_rule::Column::CreatedAt)
        .offset(Some(resolved.offset()))
        .limit(Some(resolved.per_page))
        .all(tenant_db.db())
        .await?;

    let items = rules.into_iter().map(rule_to_response).collect();
    Ok(PaginatedResponse::new(items, total, resolved))
}

pub async fn get_rule(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<Option<NotificationRuleResponse>, sea_orm::DbErr> {
    let rule = tenant_db
        .find_by_id::<notification_rule::Entity, _>(id)
        .one(tenant_db.db())
        .await?;

    Ok(rule.map(rule_to_response))
}

pub async fn update_rule(
    tenant_db: &TenantDb,
    id: Uuid,
    req: &uptrakit_web_api_types::notifications::UpdateNotificationRuleRequest,
) -> Result<Option<NotificationRuleResponse>, sea_orm::DbErr> {
    let existing = tenant_db
        .find_by_id::<notification_rule::Entity, _>(id)
        .one(tenant_db.db())
        .await?;

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
    // host_id, software_item_id, plugin_type can be set to None to clear scope
    if req.host_id.is_some() {
        active.host_id = Set(req.host_id);
    }
    if req.software_item_id.is_some() {
        active.software_item_id = Set(req.software_item_id);
    }
    if req.plugin_type.is_some() {
        active.plugin_type = Set(req.plugin_type.clone());
    }

    let result = active.update(tenant_db.db()).await?;
    Ok(Some(rule_to_response(result)))
}

pub async fn delete_rule(tenant_db: &TenantDb, id: Uuid) -> Result<bool, sea_orm::DbErr> {
    let result = tenant_db
        .delete_many::<notification_rule::Entity>()
        .filter(notification_rule::Column::Id.eq(id))
        .exec(tenant_db.db())
        .await?;

    Ok(result.rows_affected > 0)
}

// -- Log ----------------------------------------------------------------------

pub async fn list_log(
    tenant_db: &TenantDb,
    params: &PaginationParams,
) -> Result<PaginatedResponse<NotificationLogResponse>, sea_orm::DbErr> {
    let resolved = params.resolve();

    let total = tenant_db
        .find::<notification_log::Entity>()
        .count(tenant_db.db())
        .await?;

    let logs = tenant_db
        .find::<notification_log::Entity>()
        .order_by_desc(notification_log::Column::CreatedAt)
        .offset(Some(resolved.offset()))
        .limit(Some(resolved.per_page))
        .all(tenant_db.db())
        .await?;

    let items = logs.into_iter().map(log_to_response).collect();
    Ok(PaginatedResponse::new(items, total, resolved))
}

pub async fn find_log_by_action_token(
    db: &sea_orm::DatabaseConnection,
    action_token: Uuid,
) -> Result<Option<notification_log::Model>, sea_orm::DbErr> {
    notification_log::Entity::find()
        .filter(notification_log::Column::ActionToken.eq(action_token))
        .one(db)
        .await
}

// -- Error types --------------------------------------------------------------

#[derive(Debug)]
pub enum ChannelQueryError {
    Db(sea_orm::DbErr),
    UnsupportedType(String),
    InvalidConfig(String),
}

#[derive(Debug)]
pub enum RuleQueryError {
    Db(sea_orm::DbErr),
    ChannelNotFound,
}

// -- Helpers ------------------------------------------------------------------

fn channel_to_response(
    model: notification_channel::Model,
    masked_config: serde_json::Value,
) -> NotificationChannelResponse {
    let channel_type = model
        .channel_type
        .parse::<NotificationChannelType>()
        .unwrap_or_else(|_| {
            tracing::warn!(
                channel_type = %model.channel_type,
                "unknown channel type; defaulting to Webhook"
            );
            NotificationChannelType::Webhook
        });

    NotificationChannelResponse {
        id: model.id,
        name: model.name,
        channel_type,
        config: masked_config,
        enabled: model.enabled,
        created_at: model.created_at,
        updated_at: model.updated_at,
    }
}

fn mask_channel_config(
    channel: &notification_channel::Model,
    registry: &uptrakit_notification_channels::ChannelRegistry,
) -> serde_json::Value {
    let config: serde_json::Value =
        serde_json::from_str(channel.config.expose_secret()).unwrap_or_default();

    if let Some(channel_impl) = registry.get(&channel.channel_type) {
        channel_impl.mask_config_secrets(&config)
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
