use std::collections::HashSet;
use std::time::Duration;

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, IntoActiveModel,
    QueryFilter, TransactionTrait,
};
use time::{OffsetDateTime, UtcDateTime};
use uptrakit_internal_wire::{
    ControllerMessage, MqttTenantConfig, MqttTenantConfigUpdatedPayload, MqttTenantRevokedPayload,
    MqttTransport,
};
use uptrakit_shared_db::entity::{mqtt_client, mqtt_lease};
use uuid::Uuid;

use crate::service_connections::ServiceConnectionRegistry;

/// Error type for lease coordinator operations.
#[derive(Debug, thiserror::Error)]
pub enum LeaseCoordinatorError {
    #[error("database error: {0}")]
    Database(String),
    #[error("service not connected: {0}")]
    ServiceNotConnected(Uuid),
    #[error("mqtt client not found: {0}")]
    MqttClientNotFound(Uuid),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<LeaseCoordinatorError>>;

/// Centralized coordinator for MQTT client leases.
///
/// Manages the assignment of MQTT clients to MQTT service instances, updates the
/// `mqtt_leases` table, and pushes configuration changes to holding instances.
///
/// MQTT credential-bearing messages (`TenantAssignments`, `TenantConfigUpdated`,
/// `TenantRevoked`) are delivered locally only and never written to the outbox to
/// prevent plaintext credential persistence. The MQTT service reconciles its
/// state from the DB on reconnect.
#[derive(Clone)]
pub struct MqttLeaseCoordinator {
    db: DatabaseConnection,
    connections: ServiceConnectionRegistry,
}

impl MqttLeaseCoordinator {
    pub fn new(db: DatabaseConnection, connections: ServiceConnectionRegistry) -> Self {
        Self { db, connections }
    }

    /// Assign unclaimed MQTT clients to a service that has capacity.
    ///
    /// This is called when a service sends a Register message with its capacity.
    /// Returns the list of MQTT client configurations assigned to the service.
    pub async fn assign_available_tenants(
        &self,
        service_id: Uuid,
        instance_id: &str,
        requested_count: u32,
    ) -> Result<Vec<MqttTenantConfig>> {
        // Check available capacity for this service
        let available = self
            .connections
            .get_available_capacity(&service_id)
            .await
            .ok_or_else(|| report!(LeaseCoordinatorError::ServiceNotConnected(service_id)))?;

        if available == 0 {
            return Ok(vec![]);
        }

        // Calculate how many to assign (min of requested, available, and actual unclaimed)
        let max_to_assign = std::cmp::min(requested_count, available);

        // Start a transaction
        let txn = self
            .db
            .begin()
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to start transaction".into(),
            ))?;

        // Find all MQTT clients that already have active leases (by mqtt_client_id)
        let existing_leases: HashSet<Uuid> = mqtt_lease::Entity::find()
            .all(&txn)
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to query leases".into(),
            ))?
            .into_iter()
            .map(|l| l.mqtt_client_id)
            .collect();

        // Get enabled MQTT clients without leases
        let available_clients: Vec<mqtt_client::Model> = mqtt_client::Entity::find()
            .filter(mqtt_client::Column::Enabled.eq(true))
            .all(&txn)
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to query mqtt_clients".into(),
            ))?
            .into_iter()
            .filter(|c| !existing_leases.contains(&c.id))
            .take(max_to_assign as usize)
            .collect();

        if available_clients.is_empty() {
            txn.commit().await.context(LeaseCoordinatorError::Database(
                "failed to commit transaction".into(),
            ))?;
            return Ok(vec![]);
        }

        let now = OffsetDateTime::now_utc();
        let mut assigned_configs = Vec::with_capacity(available_clients.len());

        for client in &available_clients {
            // Create lease record
            let lease = mqtt_lease::ActiveModel {
                id: ActiveValue::Set(Uuid::now_v7()),
                tenant_id: ActiveValue::Set(client.tenant_id),
                mqtt_client_id: ActiveValue::Set(client.id),
                instance_id: ActiveValue::Set(instance_id.to_string()),
                heartbeat_at: ActiveValue::Set(now),
                created_at: ActiveValue::Set(now),
            };
            lease
                .insert(&txn)
                .await
                .context(LeaseCoordinatorError::Database(
                    "failed to insert lease".into(),
                ))?;

            // Track in connection registry
            self.connections
                .assign_mqtt_client(&service_id, client.id)
                .await;

            // Build config for the service
            assigned_configs.push(model_to_config(client));
        }

        txn.commit().await.context(LeaseCoordinatorError::Database(
            "failed to commit transaction".into(),
        ))?;

        Ok(assigned_configs)
    }

    /// Release specific MQTT clients from a service.
    ///
    /// Called when a service sends ReleaseTenants message or disconnects.
    pub async fn release_mqtt_clients(
        &self,
        service_id: &Uuid,
        mqtt_client_ids: &[Uuid],
    ) -> Result<()> {
        // Delete leases from database
        mqtt_lease::Entity::delete_many()
            .filter(mqtt_lease::Column::MqttClientId.is_in(mqtt_client_ids.to_vec()))
            .exec(&self.db)
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to delete leases".into(),
            ))?;

        // Update connection registry
        for mqtt_client_id in mqtt_client_ids {
            self.connections
                .release_mqtt_client(service_id, mqtt_client_id)
                .await;
        }

        Ok(())
    }

    /// Release all MQTT clients held by a service (on disconnect).
    ///
    /// Returns the MQTT client IDs that were released.
    pub async fn release_all_for_service(&self, service_id: &Uuid) -> Result<HashSet<Uuid>> {
        // Get mqtt_client_ids from registry
        let mqtt_client_ids = self
            .connections
            .unregister(service_id)
            .await
            .unwrap_or_default();

        if !mqtt_client_ids.is_empty() {
            // Delete leases from database
            mqtt_lease::Entity::delete_many()
                .filter(
                    mqtt_lease::Column::MqttClientId
                        .is_in(mqtt_client_ids.iter().copied().collect::<Vec<_>>()),
                )
                .exec(&self.db)
                .await
                .context(LeaseCoordinatorError::Database(
                    "failed to delete leases".into(),
                ))?;
        }

        Ok(mqtt_client_ids)
    }

    /// Update heartbeat for all leases held by a service.
    ///
    /// Called when a service sends Ping. Updates the `heartbeat_at` timestamp
    /// for all leases matching the service's `instance_id`.
    pub async fn record_heartbeat(&self, service_id: &Uuid) -> Result<()> {
        // Update connection registry heartbeat
        self.connections.record_heartbeat(service_id).await;

        // Look up instance_id from the connection registry
        let instance_id = match self.connections.get_instance_id(service_id).await {
            Some(id) => id,
            None => return Ok(()),
        };

        // Update heartbeat timestamps for all leases belonging to this instance
        let now = OffsetDateTime::now_utc();
        mqtt_lease::Entity::update_many()
            .col_expr(
                mqtt_lease::Column::HeartbeatAt,
                sea_orm::sea_query::Expr::value(now),
            )
            .filter(mqtt_lease::Column::InstanceId.eq(instance_id))
            .exec(&self.db)
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to update heartbeats".into(),
            ))?;

        Ok(())
    }

    /// Push a config update to the service holding a specific MQTT client.
    ///
    /// Called when MQTT settings are updated via REST API.
    /// Uses the notification service for cross-controller delivery.
    pub async fn push_mqtt_client_config_update(&self, mqtt_client_id: Uuid) -> Result<bool> {
        // Load current config from database
        let client = mqtt_client::Entity::find_by_id(mqtt_client_id)
            .one(&self.db)
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to query mqtt_client".into(),
            ))?
            .ok_or_else(|| report!(LeaseCoordinatorError::MqttClientNotFound(mqtt_client_id)))?;

        let config = model_to_config(&client);
        let msg = ControllerMessage::TenantConfigUpdated(MqttTenantConfigUpdatedPayload {
            tenant: config,
        });

        // Find the service holding this MQTT client locally.
        // MQTT credential-bearing messages are never written to the outbox
        // (they contain plaintext passwords). The MQTT service reconciles
        // its state from the DB on reconnect to any controller.
        if let Some(service_id) = self
            .connections
            .get_instance_for_mqtt_client(&mqtt_client_id)
            .await
        {
            // Local holder found — deliver directly (no outbox write).
            Ok(self.connections.send(&service_id, msg).await)
        } else {
            tracing::debug!(
                %mqtt_client_id,
                "no local MQTT holder for config update; service will reconcile on reconnect"
            );
            Ok(false)
        }
    }

    /// Revoke an MQTT client (disabled or deleted) from the holding service.
    ///
    /// Called when MQTT settings are disabled/deleted via REST API.
    /// MQTT credential-bearing messages are never written to the outbox.
    /// The MQTT service reconciles state from the DB on reconnect.
    pub async fn revoke_mqtt_client(&self, mqtt_client_id: Uuid, reason: &str) -> Result<bool> {
        // Delete lease from database regardless
        mqtt_lease::Entity::delete_many()
            .filter(mqtt_lease::Column::MqttClientId.eq(mqtt_client_id))
            .exec(&self.db)
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to delete lease".into(),
            ))?;

        let msg = ControllerMessage::TenantRevoked(MqttTenantRevokedPayload {
            mqtt_client_id,
            reason: reason.to_string(),
        });

        // Find the service holding this MQTT client locally.
        if let Some(service_id) = self
            .connections
            .get_instance_for_mqtt_client(&mqtt_client_id)
            .await
        {
            // Update local registry
            self.connections
                .release_mqtt_client(&service_id, &mqtt_client_id)
                .await;

            // Deliver directly (no outbox write for credential-bearing messages).
            Ok(self.connections.send(&service_id, msg).await)
        } else {
            tracing::debug!(
                %mqtt_client_id,
                "no local MQTT holder for revocation; lease already deleted from DB"
            );
            Ok(false)
        }
    }

    /// Clean up stale leases (no heartbeat within timeout).
    ///
    /// Called periodically by a background task.
    pub async fn cleanup_stale_leases(&self, timeout: Duration) -> Result<usize> {
        let cutoff = OffsetDateTime::now_utc() - time::Duration::seconds(timeout.as_secs() as i64);

        let deleted = mqtt_lease::Entity::delete_many()
            .filter(mqtt_lease::Column::HeartbeatAt.lt(cutoff))
            .exec(&self.db)
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to delete stale leases".into(),
            ))?;

        Ok(deleted.rows_affected as usize)
    }

    /// Get MQTT client configs for a set of MQTT client IDs.
    ///
    /// Used during reconnection to rebuild state.
    pub async fn get_mqtt_client_configs(
        &self,
        mqtt_client_ids: &[Uuid],
    ) -> Result<Vec<MqttTenantConfig>> {
        if mqtt_client_ids.is_empty() {
            return Ok(vec![]);
        }

        let clients = mqtt_client::Entity::find()
            .filter(mqtt_client::Column::Id.is_in(mqtt_client_ids.to_vec()))
            .all(&self.db)
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to query mqtt_clients".into(),
            ))?;

        Ok(clients.iter().map(model_to_config).collect())
    }

    /// Reconcile service's claimed MQTT clients on reconnect.
    ///
    /// Called when a service reconnects and sends Register with its current
    /// active_mqtt_clients list. Verifies that leases exist for claimed clients,
    /// or re-creates them if they were cleaned up.
    pub async fn reconcile_mqtt_clients(
        &self,
        service_id: Uuid,
        instance_id: &str,
        claimed_mqtt_client_ids: &[Uuid],
    ) -> Result<Vec<MqttTenantConfig>> {
        if claimed_mqtt_client_ids.is_empty() {
            return Ok(vec![]);
        }

        let txn = self
            .db
            .begin()
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to start transaction".into(),
            ))?;

        let now = OffsetDateTime::now_utc();

        // Check existing leases for these MQTT clients
        let existing_leases: std::collections::HashMap<Uuid, mqtt_lease::Model> =
            mqtt_lease::Entity::find()
                .filter(mqtt_lease::Column::MqttClientId.is_in(claimed_mqtt_client_ids.to_vec()))
                .all(&txn)
                .await
                .context(LeaseCoordinatorError::Database(
                    "failed to query leases".into(),
                ))?
                .into_iter()
                .map(|l| (l.mqtt_client_id, l))
                .collect();

        // Get mqtt_client configs for all claimed IDs
        let clients: std::collections::HashMap<Uuid, mqtt_client::Model> =
            mqtt_client::Entity::find()
                .filter(mqtt_client::Column::Id.is_in(claimed_mqtt_client_ids.to_vec()))
                .filter(mqtt_client::Column::Enabled.eq(true))
                .all(&txn)
                .await
                .context(LeaseCoordinatorError::Database(
                    "failed to query mqtt_clients".into(),
                ))?
                .into_iter()
                .map(|c| (c.id, c))
                .collect();

        let mut reconciled_configs = Vec::new();

        for mqtt_client_id in claimed_mqtt_client_ids {
            // Skip if no enabled mqtt_client for this ID
            let client = match clients.get(mqtt_client_id) {
                Some(c) => c,
                None => continue,
            };

            // Check if lease exists
            if let Some(existing) = existing_leases.get(mqtt_client_id) {
                // Lease exists - verify it's ours (same instance) or take it over
                if existing.instance_id == instance_id {
                    // Same instance, just update heartbeat
                    let mut active: mqtt_lease::ActiveModel = existing.clone().into_active_model();
                    active.heartbeat_at = ActiveValue::Set(now);
                    active
                        .update(&txn)
                        .await
                        .context(LeaseCoordinatorError::Database(
                            "failed to update lease".into(),
                        ))?;
                } else {
                    // Different instance had the lease - take it over
                    let mut active: mqtt_lease::ActiveModel = existing.clone().into_active_model();
                    active.instance_id = ActiveValue::Set(instance_id.to_string());
                    active.heartbeat_at = ActiveValue::Set(now);
                    active
                        .update(&txn)
                        .await
                        .context(LeaseCoordinatorError::Database(
                            "failed to update lease".into(),
                        ))?;
                }
            } else {
                // No lease exists - create one
                let lease = mqtt_lease::ActiveModel {
                    id: ActiveValue::Set(Uuid::now_v7()),
                    tenant_id: ActiveValue::Set(client.tenant_id),
                    mqtt_client_id: ActiveValue::Set(*mqtt_client_id),
                    instance_id: ActiveValue::Set(instance_id.to_string()),
                    heartbeat_at: ActiveValue::Set(now),
                    created_at: ActiveValue::Set(now),
                };
                lease
                    .insert(&txn)
                    .await
                    .context(LeaseCoordinatorError::Database(
                        "failed to insert lease".into(),
                    ))?;
            }

            // Track in connection registry
            self.connections
                .assign_mqtt_client(&service_id, *mqtt_client_id)
                .await;

            // Add to result
            reconciled_configs.push(model_to_config(client));
        }

        txn.commit().await.context(LeaseCoordinatorError::Database(
            "failed to commit transaction".into(),
        ))?;

        Ok(reconciled_configs)
    }
}

/// Convert mqtt_client model to MqttTenantConfig wire type.
fn model_to_config(client: &mqtt_client::Model) -> MqttTenantConfig {
    MqttTenantConfig {
        mqtt_client_id: client.id,
        tenant_id: client.tenant_id,
        enabled: client.enabled,
        transport: wire_mqtt_transport(&client.transport),
        host: client.host.clone(),
        port: client.port as u16,
        client_id: client.client_id.clone(),
        username: client.username.clone(),
        password: client.password.clone(),
        topic_prefix: client.topic_prefix.clone(),
        updated_at: UtcDateTime::from_unix_timestamp(client.updated_at.unix_timestamp())
            .unwrap_or(UtcDateTime::UNIX_EPOCH),
    }
}

/// Convert a DB transport string to the wire protocol `MqttTransport`.
fn wire_mqtt_transport(transport: &str) -> MqttTransport {
    match transport {
        "tls" => MqttTransport::Tls,
        _ => MqttTransport::Tcp,
    }
}
