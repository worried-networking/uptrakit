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
    #[error("tenant not found: {0}")]
    TenantNotFound(Uuid),
}

/// Centralized coordinator for MQTT tenant leases.
///
/// Manages the assignment of tenants to MQTT service instances, updates the
/// `mqtt_leases` table, and pushes configuration changes to holding instances.
#[derive(Clone)]
pub struct MqttLeaseCoordinator {
    db: DatabaseConnection,
    connections: ServiceConnectionRegistry,
}

impl MqttLeaseCoordinator {
    pub fn new(db: DatabaseConnection, connections: ServiceConnectionRegistry) -> Self {
        Self { db, connections }
    }

    /// Assign unclaimed tenants to a service that has capacity.
    ///
    /// This is called when a service sends a Register message with its capacity.
    /// Returns the list of tenant configurations assigned to the service.
    pub async fn assign_available_tenants(
        &self,
        service_id: Uuid,
        instance_id: &str,
        requested_count: u32,
    ) -> Result<Vec<MqttTenantConfig>, Report<LeaseCoordinatorError>> {
        // Check available capacity for this service
        let available = self
            .connections
            .get_available_capacity(&service_id)
            .await
            .ok_or_else(|| report!(LeaseCoordinatorError::ServiceNotConnected(service_id)))?;

        if available == 0 {
            return Ok(vec![]);
        }

        // Calculate how many tenants to assign (min of requested, available, and actual unclaimed)
        let max_to_assign = std::cmp::min(requested_count, available);

        // Start a transaction
        let txn = self
            .db
            .begin()
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to start transaction".into(),
            ))?;

        // Find all enabled mqtt_clients that don't have active leases
        let existing_leases: HashSet<Uuid> = mqtt_lease::Entity::find()
            .all(&txn)
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to query leases".into(),
            ))?
            .into_iter()
            .map(|l| l.tenant_id)
            .collect();

        // Get enabled tenants without leases
        let available_clients: Vec<mqtt_client::Model> = mqtt_client::Entity::find()
            .filter(mqtt_client::Column::Enabled.eq(true))
            .all(&txn)
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to query mqtt_clients".into(),
            ))?
            .into_iter()
            .filter(|c| !existing_leases.contains(&c.tenant_id))
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
                .assign_tenant(&service_id, client.tenant_id)
                .await;

            // Build config for the service
            assigned_configs.push(model_to_config(client));
        }

        txn.commit().await.context(LeaseCoordinatorError::Database(
            "failed to commit transaction".into(),
        ))?;

        Ok(assigned_configs)
    }

    /// Release specific tenants from a service.
    ///
    /// Called when a service sends ReleaseTenants message or disconnects.
    pub async fn release_tenants(
        &self,
        service_id: &Uuid,
        tenant_ids: &[Uuid],
    ) -> Result<(), Report<LeaseCoordinatorError>> {
        // Delete leases from database
        mqtt_lease::Entity::delete_many()
            .filter(mqtt_lease::Column::TenantId.is_in(tenant_ids.to_vec()))
            .exec(&self.db)
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to delete leases".into(),
            ))?;

        // Update connection registry
        for tenant_id in tenant_ids {
            self.connections.release_tenant(service_id, tenant_id).await;
        }

        Ok(())
    }

    /// Release all tenants held by a service (on disconnect).
    ///
    /// Returns the tenant IDs that were released.
    pub async fn release_all_for_service(
        &self,
        service_id: &Uuid,
    ) -> Result<HashSet<Uuid>, Report<LeaseCoordinatorError>> {
        // Get tenants from registry
        let tenants = self
            .connections
            .unregister(service_id)
            .await
            .unwrap_or_default();

        if !tenants.is_empty() {
            // Delete leases from database
            mqtt_lease::Entity::delete_many()
                .filter(
                    mqtt_lease::Column::TenantId.is_in(tenants.iter().copied().collect::<Vec<_>>()),
                )
                .exec(&self.db)
                .await
                .context(LeaseCoordinatorError::Database(
                    "failed to delete leases".into(),
                ))?;
        }

        Ok(tenants)
    }

    /// Update heartbeat for tenants held by a service.
    ///
    /// Called when a service sends Heartbeat message.
    pub async fn record_heartbeat(
        &self,
        service_id: &Uuid,
        active_tenant_ids: &[Uuid],
    ) -> Result<(), Report<LeaseCoordinatorError>> {
        // Update connection registry heartbeat
        self.connections.record_heartbeat(service_id).await;

        if active_tenant_ids.is_empty() {
            return Ok(());
        }

        // Update heartbeat timestamps in database
        let now = OffsetDateTime::now_utc();
        mqtt_lease::Entity::update_many()
            .col_expr(
                mqtt_lease::Column::HeartbeatAt,
                sea_orm::sea_query::Expr::value(now),
            )
            .filter(mqtt_lease::Column::TenantId.is_in(active_tenant_ids.to_vec()))
            .exec(&self.db)
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to update heartbeats".into(),
            ))?;

        Ok(())
    }

    /// Push a config update to the service holding a specific tenant.
    ///
    /// Called when MQTT settings are updated via REST API.
    pub async fn push_tenant_config_update(
        &self,
        tenant_id: Uuid,
    ) -> Result<bool, Report<LeaseCoordinatorError>> {
        // Find the service holding this tenant
        let service_id = match self.connections.get_instance_for_tenant(&tenant_id).await {
            Some(id) => id,
            None => return Ok(false), // No service holds this tenant
        };

        // Load current config from database
        let client = mqtt_client::Entity::find()
            .filter(mqtt_client::Column::TenantId.eq(tenant_id))
            .one(&self.db)
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to query mqtt_client".into(),
            ))?
            .ok_or_else(|| report!(LeaseCoordinatorError::TenantNotFound(tenant_id)))?;

        let config = model_to_config(&client);
        let msg = ControllerMessage::TenantConfigUpdated(MqttTenantConfigUpdatedPayload {
            tenant: config,
        });

        Ok(self.connections.send(&service_id, msg).await)
    }

    /// Revoke a tenant (disabled or deleted) from the holding service.
    ///
    /// Called when MQTT settings are disabled/deleted via REST API.
    pub async fn revoke_tenant(
        &self,
        tenant_id: Uuid,
        reason: &str,
    ) -> Result<bool, Report<LeaseCoordinatorError>> {
        // Find the service holding this tenant
        let service_id = match self.connections.get_instance_for_tenant(&tenant_id).await {
            Some(id) => id,
            None => {
                // No service holds this tenant, just delete any orphaned lease
                mqtt_lease::Entity::delete_many()
                    .filter(mqtt_lease::Column::TenantId.eq(tenant_id))
                    .exec(&self.db)
                    .await
                    .context(LeaseCoordinatorError::Database(
                        "failed to delete lease".into(),
                    ))?;
                return Ok(false);
            }
        };

        // Delete lease from database
        mqtt_lease::Entity::delete_many()
            .filter(mqtt_lease::Column::TenantId.eq(tenant_id))
            .exec(&self.db)
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to delete lease".into(),
            ))?;

        // Update registry
        self.connections
            .release_tenant(&service_id, &tenant_id)
            .await;

        // Push revocation message
        let msg = ControllerMessage::TenantRevoked(MqttTenantRevokedPayload {
            tenant_id: tenant_id.to_string(),
            reason: reason.to_string(),
        });

        Ok(self.connections.send(&service_id, msg).await)
    }

    /// Clean up stale leases (no heartbeat within timeout).
    ///
    /// Called periodically by a background task.
    pub async fn cleanup_stale_leases(
        &self,
        timeout: Duration,
    ) -> Result<usize, Report<LeaseCoordinatorError>> {
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

    /// Get tenant configs for a set of tenant IDs.
    ///
    /// Used during reconnection to rebuild state.
    pub async fn get_tenant_configs(
        &self,
        tenant_ids: &[Uuid],
    ) -> Result<Vec<MqttTenantConfig>, Report<LeaseCoordinatorError>> {
        if tenant_ids.is_empty() {
            return Ok(vec![]);
        }

        let clients = mqtt_client::Entity::find()
            .filter(mqtt_client::Column::TenantId.is_in(tenant_ids.to_vec()))
            .all(&self.db)
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to query mqtt_clients".into(),
            ))?;

        Ok(clients.iter().map(model_to_config).collect())
    }

    /// Reconcile service's claimed tenants on reconnect.
    ///
    /// Called when a service reconnects and sends Register with its current
    /// active_tenants list. Verifies that leases exist for claimed tenants,
    /// or re-creates them if they were cleaned up.
    pub async fn reconcile_tenants(
        &self,
        service_id: Uuid,
        instance_id: &str,
        claimed_tenant_ids: &[Uuid],
    ) -> Result<Vec<MqttTenantConfig>, Report<LeaseCoordinatorError>> {
        if claimed_tenant_ids.is_empty() {
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

        // Check existing leases for these tenants
        let existing_leases: std::collections::HashMap<Uuid, mqtt_lease::Model> =
            mqtt_lease::Entity::find()
                .filter(mqtt_lease::Column::TenantId.is_in(claimed_tenant_ids.to_vec()))
                .all(&txn)
                .await
                .context(LeaseCoordinatorError::Database(
                    "failed to query leases".into(),
                ))?
                .into_iter()
                .map(|l| (l.tenant_id, l))
                .collect();

        // Get mqtt_client configs for all claimed tenants
        let clients: std::collections::HashMap<Uuid, mqtt_client::Model> =
            mqtt_client::Entity::find()
                .filter(mqtt_client::Column::TenantId.is_in(claimed_tenant_ids.to_vec()))
                .filter(mqtt_client::Column::Enabled.eq(true))
                .all(&txn)
                .await
                .context(LeaseCoordinatorError::Database(
                    "failed to query mqtt_clients".into(),
                ))?
                .into_iter()
                .map(|c| (c.tenant_id, c))
                .collect();

        let mut reconciled_configs = Vec::new();

        for tenant_id in claimed_tenant_ids {
            // Skip if no enabled mqtt_client for this tenant
            let client = match clients.get(tenant_id) {
                Some(c) => c,
                None => continue,
            };

            // Check if lease exists
            if let Some(existing) = existing_leases.get(tenant_id) {
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
                    // (This can happen if the previous instance crashed without cleanup)
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
                    tenant_id: ActiveValue::Set(*tenant_id),
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
                .assign_tenant(&service_id, *tenant_id)
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
        tenant_id: client.tenant_id.to_string(),
        enabled: client.enabled,
        transport: client.transport.clone(),
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
