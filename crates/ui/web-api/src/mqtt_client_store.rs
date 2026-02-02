use rootcause::{Report, ReportConversion, markers, prelude::*};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{mqtt_client, prelude::MqttClient};
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum MqttClientError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    #[error("mqtt client not found")]
    NotFound,

    #[error("mqtt client already exists for this tenant")]
    AlreadyExists,
}

pub type Result<T> = std::result::Result<T, Report<MqttClientError>>;

impl<T> ReportConversion<sea_orm::DbErr, markers::Mutable, T> for MqttClientError
where
    MqttClientError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<sea_orm::DbErr, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(MqttClientError::Database)
    }
}

/// Load the MQTT client for a given tenant. Returns `None` if not configured.
pub async fn load_mqtt_client(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> Result<Option<mqtt_client::Model>> {
    MqttClient::find()
        .filter(mqtt_client::Column::TenantId.eq(tenant_id))
        .one(db)
        .await
        .context_to()
}

/// Create a new MQTT client for a tenant.
#[allow(clippy::too_many_arguments)]
pub async fn create_mqtt_client(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    enabled: bool,
    transport: &str,
    host: &str,
    port: u16,
    path: Option<&str>,
    client_id: &str,
    username: Option<&str>,
    password: Option<&str>,
    topic_prefix: &str,
) -> Result<mqtt_client::Model> {
    // Check for existing
    let existing = load_mqtt_client(db, tenant_id).await?;
    if existing.is_some() {
        return Err(report!(MqttClientError::AlreadyExists));
    }

    let now = OffsetDateTime::now_utc();
    let model = mqtt_client::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant_id),
        enabled: Set(enabled),
        transport: Set(transport.to_string()),
        host: Set(host.to_string()),
        port: Set(i32::from(port)),
        path: Set(path.map(String::from)),
        client_id: Set(client_id.to_string()),
        username: Set(username.map(String::from)),
        password: Set(password.map(String::from)),
        topic_prefix: Set(topic_prefix.to_string()),
        created_at: Set(now),
        updated_at: Set(now),
    };

    model.insert(db).await.context_to()
}

/// Update an existing MQTT client model. Pass the loaded model; only non-None
/// fields are updated.
#[allow(clippy::too_many_arguments)]
pub async fn update_mqtt_client(
    db: &DatabaseConnection,
    existing: mqtt_client::Model,
    enabled: Option<bool>,
    transport: Option<&str>,
    host: Option<&str>,
    port: Option<u16>,
    path: Option<Option<&str>>,
    client_id: Option<&str>,
    username: Option<Option<&str>>,
    password: Option<Option<&str>>,
    topic_prefix: Option<&str>,
) -> Result<mqtt_client::Model> {
    let mut model: mqtt_client::ActiveModel = existing.into();

    if let Some(v) = enabled {
        model.enabled = Set(v);
    }
    if let Some(v) = transport {
        model.transport = Set(v.to_string());
    }
    if let Some(v) = host {
        model.host = Set(v.to_string());
    }
    if let Some(v) = port {
        model.port = Set(i32::from(v));
    }
    if let Some(v) = path {
        model.path = Set(v.map(String::from));
    }
    if let Some(v) = client_id {
        model.client_id = Set(v.to_string());
    }
    if let Some(v) = username {
        model.username = Set(v.map(String::from));
    }
    if let Some(v) = password {
        model.password = Set(v.map(String::from));
    }
    if let Some(v) = topic_prefix {
        model.topic_prefix = Set(v.to_string());
    }
    model.updated_at = Set(OffsetDateTime::now_utc());

    model.update(db).await.context_to()
}

/// Delete the MQTT client for a given tenant.
pub async fn delete_mqtt_client(db: &DatabaseConnection, tenant_id: Uuid) -> Result<()> {
    let result = MqttClient::delete_many()
        .filter(mqtt_client::Column::TenantId.eq(tenant_id))
        .exec(db)
        .await
        .context_to()?;

    if result.rows_affected == 0 {
        return Err(report!(MqttClientError::NotFound));
    }

    Ok(())
}
