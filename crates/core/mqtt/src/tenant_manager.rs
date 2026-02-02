use std::collections::HashMap;

use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use time::OffsetDateTime;
use uptrakit_web_api_types::mqtt_transport::MqttTransport;
use uuid::Uuid;

use uptrakit_shared_db::entity::mqtt_client;

use crate::db::DbError;
use crate::lease_manager::LeaseManager;
use crate::mqtt_client::{MqttConfig, MqttHandle};

/// Tracks the cached state for a tenant's MQTT client.
struct TenantState {
    handle: MqttHandle,
    updated_at: OffsetDateTime,
}

/// Manages per-tenant MQTT client lifecycles with hot-reload support.
pub struct TenantManager {
    tenants: HashMap<Uuid, TenantState>,
}

impl TenantManager {
    pub fn new() -> Self {
        Self {
            tenants: HashMap::new(),
        }
    }

    /// Run one poll cycle: claim tenants, detect config changes, start/stop clients.
    pub async fn poll(&mut self, lease_mgr: &LeaseManager) {
        // 1. Clean up stale leases
        if let Err(e) = lease_mgr.cleanup_stale_leases().await {
            tracing::error!(error = ?e, "failed to clean up stale leases");
        }

        // 2. Claim newly available tenants
        match lease_mgr.claim_tenants().await {
            Ok(new_tenants) => {
                for tenant_id in new_tenants {
                    self.start_tenant(lease_mgr.db(), tenant_id).await;
                }
            }
            Err(e) => {
                tracing::error!(error = ?e, "failed to claim tenants");
            }
        }

        // 3. For each held tenant, check for config changes or deletion
        let held = match lease_mgr.held_tenants().await {
            Ok(h) => h,
            Err(e) => {
                tracing::error!(error = ?e, "failed to get held tenants");
                return;
            }
        };

        for tenant_id in &held {
            let model = match load_mqtt_client(lease_mgr.db(), *tenant_id).await {
                Ok(Some(m)) => m,
                Ok(None) => {
                    // Config deleted — stop client, release lease
                    tracing::info!(%tenant_id, "MQTT config deleted, stopping client");
                    self.stop_tenant(*tenant_id).await;
                    if let Err(e) = lease_mgr.release_tenant(*tenant_id).await {
                        tracing::error!(%tenant_id, error = ?e, "failed to release tenant");
                    }
                    continue;
                }
                Err(e) => {
                    tracing::error!(%tenant_id, error = ?e, "failed to load MQTT config");
                    continue;
                }
            };

            if !model.enabled {
                tracing::info!(%tenant_id, "MQTT client disabled, stopping");
                self.stop_tenant(*tenant_id).await;
                if let Err(e) = lease_mgr.release_tenant(*tenant_id).await {
                    tracing::error!(%tenant_id, error = ?e, "failed to release tenant");
                }
                continue;
            }

            // Check for config change (updated_at differs from cached)
            if let Some(state) = self.tenants.get(tenant_id) {
                if model.updated_at != state.updated_at {
                    tracing::info!(%tenant_id, "MQTT config changed, reloading");
                    self.stop_tenant(*tenant_id).await;
                    self.start_tenant(lease_mgr.db(), *tenant_id).await;
                }
            }
        }
    }

    /// Start an MQTT client for a tenant.
    async fn start_tenant(&mut self, db: &DatabaseConnection, tenant_id: Uuid) {
        let model = match load_mqtt_client(db, tenant_id).await {
            Ok(Some(m)) if m.enabled => m,
            Ok(Some(_)) => {
                tracing::debug!(%tenant_id, "MQTT client disabled, skipping start");
                return;
            }
            Ok(None) => {
                tracing::debug!(%tenant_id, "no MQTT config found, skipping start");
                return;
            }
            Err(e) => {
                tracing::error!(%tenant_id, error = ?e, "failed to load MQTT config for start");
                return;
            }
        };

        let config = build_config_from_model(&model);
        let updated_at = model.updated_at;

        tracing::info!(%tenant_id, config = ?config, "starting MQTT client");
        match crate::mqtt_client::start(config).await {
            Ok(handle) => {
                self.tenants.insert(
                    tenant_id,
                    TenantState {
                        handle,
                        updated_at,
                    },
                );
            }
            Err(e) => {
                tracing::warn!(%tenant_id, error = ?e, "MQTT client startup failed");
            }
        }
    }

    /// Stop an MQTT client for a tenant (publish offline, disconnect).
    async fn stop_tenant(&mut self, tenant_id: Uuid) {
        if let Some(state) = self.tenants.remove(&tenant_id) {
            tracing::info!(%tenant_id, "shutting down MQTT client");
            state.handle.shutdown().await;
        }
    }

    /// Graceful shutdown: stop all MQTT clients.
    pub async fn shutdown_all(&mut self) {
        let tenant_ids: Vec<Uuid> = self.tenants.keys().copied().collect();
        for tenant_id in tenant_ids {
            self.stop_tenant(tenant_id).await;
        }
    }

}

fn build_config_from_model(model: &mqtt_client::Model) -> MqttConfig {
    let transport = MqttTransport::parse(&model.transport).unwrap_or_default();
    let port = u16::try_from(model.port).unwrap_or(transport.default_port());

    MqttConfig {
        transport,
        host: model.host.clone(),
        port,
        path: model.path.clone(),
        client_id: model.client_id.clone(),
        username: model.username.clone(),
        password: model.password.clone(),
        topic_prefix: model.topic_prefix.clone(),
    }
}

async fn load_mqtt_client(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> crate::db::Result<Option<mqtt_client::Model>> {
    mqtt_client::Entity::find()
        .filter(mqtt_client::Column::TenantId.eq(tenant_id))
        .one(db)
        .await
        .context_to::<DbError>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_config_from_model_correct() {
        let now = OffsetDateTime::now_utc();
        let model = mqtt_client::Model {
            id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7(),
            enabled: true,
            transport: "tls".to_string(),
            host: "broker.example.com".to_string(),
            port: 8883,
            path: Some("/mqtt".to_string()),
            client_id: "my-client".to_string(),
            username: Some("user".to_string()),
            password: Some("pass".to_string()),
            topic_prefix: "home/uptrakit".to_string(),
            created_at: now,
            updated_at: now,
        };

        let config = build_config_from_model(&model);

        assert_eq!(config.transport, MqttTransport::Tls);
        assert_eq!(config.host, "broker.example.com");
        assert_eq!(config.port, 8883);
        assert_eq!(config.path.as_deref(), Some("/mqtt"));
        assert_eq!(config.client_id, "my-client");
        assert_eq!(config.username.as_deref(), Some("user"));
        assert_eq!(config.password.as_deref(), Some("pass"));
        assert_eq!(config.topic_prefix, "home/uptrakit");
    }
}
