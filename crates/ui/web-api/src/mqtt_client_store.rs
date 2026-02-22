use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    Set,
};
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{mqtt_client, prelude::MqttClient};
use uptrakit_shared_db::{MqttClientConnectionStatus, MqttTransport};
use uptrakit_shared_macros::impl_report_conversion;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum MqttClientError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    #[error("encryption failed")]
    Encryption(#[from] uptrakit_shared_db::crypto::CryptoError),

    #[error("mqtt client not found")]
    NotFound,

    #[error("mqtt client limit reached: maximum {0} per tenant")]
    LimitReached(u16),
}

pub type Result<T> = std::result::Result<T, Report<MqttClientError>>;

impl_report_conversion!(sea_orm::DbErr => MqttClientError::Database);
impl_report_conversion!(uptrakit_shared_db::crypto::CryptoError => MqttClientError::Encryption);

/// Load all MQTT clients for a given tenant.
pub async fn load_mqtt_clients(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> Result<Vec<mqtt_client::Model>> {
    MqttClient::find()
        .filter(mqtt_client::Column::TenantId.eq(tenant_id))
        .all(db)
        .await
        .context_to()
}

/// Load a specific MQTT client by its ID, scoped to the given tenant.
pub async fn load_mqtt_client_by_id(
    db: &DatabaseConnection,
    id: Uuid,
    tenant_id: Uuid,
) -> Result<Option<mqtt_client::Model>> {
    MqttClient::find_by_id(id)
        .filter(mqtt_client::Column::TenantId.eq(tenant_id))
        .one(db)
        .await
        .context_to()
}

/// Count the number of MQTT clients for a given tenant.
pub async fn count_mqtt_clients(db: &DatabaseConnection, tenant_id: Uuid) -> Result<u64> {
    MqttClient::find()
        .filter(mqtt_client::Column::TenantId.eq(tenant_id))
        .count(db)
        .await
        .context_to()
}

/// Parameters for [`create_mqtt_client`].
pub struct CreateMqttClientParams<'a> {
    pub db: &'a DatabaseConnection,
    pub tenant_id: Uuid,
    pub max_clients: u16,
    pub enabled: bool,
    pub transport: MqttTransport,
    pub host: &'a str,
    pub port: u16,
    pub client_id: &'a str,
    pub username: Option<&'a str>,
    pub password: Option<&'a str>,
    pub ca_cert_pem: Option<&'a str>,
    pub topic_prefix: &'a str,
}

/// Create a new MQTT client for a tenant.
pub async fn create_mqtt_client(params: CreateMqttClientParams<'_>) -> Result<mqtt_client::Model> {
    let CreateMqttClientParams {
        db,
        tenant_id,
        max_clients,
        enabled,
        transport,
        host,
        port,
        client_id,
        username,
        password,
        ca_cert_pem,
        topic_prefix,
    } = params;

    // Check limit
    let count = count_mqtt_clients(db, tenant_id).await?;
    if count >= u64::from(max_clients) {
        bail!(MqttClientError::LimitReached(max_clients));
    }

    let now = OffsetDateTime::now_utc();
    let model = mqtt_client::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant_id),
        enabled: Set(enabled),
        transport: Set(transport),
        host: Set(host.to_string()),
        port: Set(i32::from(port)),
        client_id: Set(client_id.to_string()),
        username: Set(username.map(String::from)),
        password: Set(password
            .map(|p| uptrakit_shared_db::crypto::EncryptedString::new(p.to_string()))
            .transpose()
            .context_to()?),
        ca_cert_pem: Set(ca_cert_pem
            .map(|c| uptrakit_shared_db::crypto::EncryptedString::new(c.to_string()))
            .transpose()
            .context_to()?),
        topic_prefix: Set(topic_prefix.to_string()),
        connection_status: Set(MqttClientConnectionStatus::Offline),
        status_updated_at: Set(now),
        created_at: Set(now),
        updated_at: Set(now),
    };

    model.insert(db).await.context_to()
}

/// Parameters for [`update_mqtt_client`].
pub struct UpdateMqttClientParams<'a> {
    pub db: &'a DatabaseConnection,
    pub existing: mqtt_client::Model,
    pub enabled: Option<bool>,
    pub transport: Option<MqttTransport>,
    pub host: Option<&'a str>,
    pub port: Option<u16>,
    pub client_id: Option<&'a str>,
    pub username: Option<Option<&'a str>>,
    pub password: Option<Option<&'a str>>,
    pub ca_cert_pem: Option<Option<&'a str>>,
    pub topic_prefix: Option<&'a str>,
}

/// Update an existing MQTT client model. Pass the loaded model; only non-None
/// fields are updated.
pub async fn update_mqtt_client(params: UpdateMqttClientParams<'_>) -> Result<mqtt_client::Model> {
    let UpdateMqttClientParams {
        db,
        existing,
        enabled,
        transport,
        host,
        port,
        client_id,
        username,
        password,
        ca_cert_pem,
        topic_prefix,
    } = params;
    let mut model: mqtt_client::ActiveModel = existing.into();

    if let Some(v) = enabled {
        model.enabled = Set(v);
    }
    if let Some(v) = transport {
        model.transport = Set(v);
    }
    if let Some(v) = host {
        model.host = Set(v.to_string());
    }
    if let Some(v) = port {
        model.port = Set(i32::from(v));
    }
    if let Some(v) = client_id {
        model.client_id = Set(v.to_string());
    }
    if let Some(v) = username {
        model.username = Set(v.map(String::from));
    }
    if let Some(v) = password {
        model.password = Set(v
            .map(|p| uptrakit_shared_db::crypto::EncryptedString::new(p.to_string()))
            .transpose()
            .context_to()?);
    }
    if let Some(v) = ca_cert_pem {
        model.ca_cert_pem = Set(v
            .map(|c| uptrakit_shared_db::crypto::EncryptedString::new(c.to_string()))
            .transpose()
            .context_to()?);
    }
    if let Some(v) = topic_prefix {
        model.topic_prefix = Set(v.to_string());
    }
    model.updated_at = Set(OffsetDateTime::now_utc());

    model.update(db).await.context_to()
}

/// Delete a specific MQTT client by ID.
pub async fn delete_mqtt_client(db: &DatabaseConnection, id: Uuid) -> Result<()> {
    let result = MqttClient::delete_by_id(id).exec(db).await.context_to()?;

    if result.rows_affected == 0 {
        bail!(MqttClientError::NotFound);
    }

    Ok(())
}

/// Update the connection status for a specific MQTT client.
pub async fn update_mqtt_client_status(
    db: &DatabaseConnection,
    id: Uuid,
    status: MqttClientConnectionStatus,
) -> Result<()> {
    let Some(existing) = MqttClient::find_by_id(id).one(db).await.context_to()? else {
        bail!(MqttClientError::NotFound);
    };

    let mut model: mqtt_client::ActiveModel = existing.into();
    model.connection_status = Set(status);
    model.status_updated_at = Set(OffsetDateTime::now_utc());
    model.update(db).await.context_to()?;

    Ok(())
}

/// Update the connection status for multiple MQTT clients.
pub async fn update_mqtt_clients_status(
    db: &DatabaseConnection,
    ids: &[Uuid],
    status: MqttClientConnectionStatus,
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }

    let now = OffsetDateTime::now_utc();
    let mut updated = 0usize;

    for id in ids {
        let Some(existing) = MqttClient::find_by_id(*id).one(db).await.context_to()? else {
            continue;
        };
        let mut model: mqtt_client::ActiveModel = existing.into();
        model.connection_status = Set(status);
        model.status_updated_at = Set(now);
        model.update(db).await.context_to()?;
        updated += 1;
    }

    if updated == 0 {
        bail!(MqttClientError::NotFound);
    }

    Ok(())
}
