use std::collections::HashSet;
use std::time::Duration;

use rootcause::prelude::*;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, IntoActiveModel,
    QueryFilter, QueryOrder, QuerySelect, TransactionTrait,
};
use time::{OffsetDateTime, UtcDateTime};
use uptrakit_internal_wire::{
    ControllerMessage, MqttTenantAssignmentsPayload, MqttTenantConfig,
    MqttTenantConfigUpdatedPayload, MqttTenantRevokedPayload, SecretString,
};
use uptrakit_shared_db::entity::{mqtt_client, mqtt_lease};
use uuid::Uuid;

use crate::mqtt_client_store;
use crate::service_connections::{MqttServiceLoad, ServiceConnectionRegistry};
use uptrakit_web_api_types::settings_mqtt::MqttClientConnectionStatus;

/// Maximum allowed age of an MQTT lease heartbeat before considering it stale.
pub const MQTT_LEASE_STALE_AFTER: Duration = Duration::from_secs(60);

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

pub type Result<T> = std::result::Result<T, Report<LeaseCoordinatorError>>;

/// Outcome when attempting to lease an MQTT client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseOutcome {
    Leased { service_id: Uuid },
    NoLocalService,
    AlreadyLeased,
}

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

        // Calculate how many to assign (min of requested and available capacity)
        let max_to_assign = std::cmp::min(requested_count, available) as usize;

        // Start a transaction
        let txn = self
            .db
            .begin()
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to start transaction".into(),
            ))?;

        if max_to_assign == 0 {
            txn.commit().await.context(LeaseCoordinatorError::Database(
                "failed to commit transaction".into(),
            ))?;
            return Ok(vec![]);
        }

        let now = OffsetDateTime::now_utc();
        let mut assigned_clients = Vec::new();
        let mut offset = 0u64;

        while assigned_clients.len() < max_to_assign {
            let batch = mqtt_client::Entity::find()
                .filter(mqtt_client::Column::Enabled.eq(true))
                .order_by_asc(mqtt_client::Column::CreatedAt)
                .offset(offset)
                .limit(100)
                .all(&txn)
                .await
                .context(LeaseCoordinatorError::Database(
                    "failed to query mqtt_clients".into(),
                ))?;

            if batch.is_empty() {
                break;
            }

            let batch_len = batch.len();
            for client in batch {
                if assigned_clients.len() >= max_to_assign {
                    break;
                }

                let lease = mqtt_lease::ActiveModel {
                    id: ActiveValue::Set(Uuid::now_v7()),
                    tenant_id: ActiveValue::Set(client.tenant_id),
                    mqtt_client_id: ActiveValue::Set(client.id),
                    instance_id: ActiveValue::Set(instance_id.to_string()),
                    heartbeat_at: ActiveValue::Set(now),
                    created_at: ActiveValue::Set(now),
                };

                match mqtt_lease::Entity::insert(lease)
                    .on_conflict(
                        OnConflict::column(mqtt_lease::Column::MqttClientId)
                            .do_nothing()
                            .to_owned(),
                    )
                    .exec(&txn)
                    .await
                {
                    Ok(_) => {
                        assigned_clients.push(client);
                    }
                    Err(sea_orm::DbErr::RecordNotInserted) => {
                        continue;
                    }
                    Err(e) => {
                        bail!(LeaseCoordinatorError::Database(format!(
                            "failed to insert lease: {e}"
                        )));
                    }
                }
            }

            offset = offset.saturating_add(batch_len as u64);
        }

        txn.commit().await.context(LeaseCoordinatorError::Database(
            "failed to commit transaction".into(),
        ))?;

        let mut assigned_configs = Vec::with_capacity(assigned_clients.len());
        for client in &assigned_clients {
            if !self
                .connections
                .assign_mqtt_client(&service_id, client.id)
                .await
            {
                if let Err(e) = self.rollback_lease(client.id, instance_id).await {
                    tracing::error!(error = %e, "failed to rollback lease after disconnect");
                }
                continue;
            }
            assigned_configs.push(model_to_config(client));
        }

        Ok(assigned_configs)
    }

    /// Attempt to lease a newly created MQTT client to the least busy local service.
    pub async fn lease_new_client_to_least_busy(
        &self,
        client: &mqtt_client::Model,
    ) -> Result<LeaseOutcome> {
        if !client.enabled {
            return Ok(LeaseOutcome::NoLocalService);
        }

        let candidates = self.connections.list_mqtt_service_loads().await;
        if candidates.is_empty() {
            return Ok(LeaseOutcome::NoLocalService);
        }

        for candidate in candidates {
            match self.try_lease_to_service(client, &candidate).await {
                Ok(LeaseOutcome::Leased { service_id }) => {
                    return Ok(LeaseOutcome::Leased { service_id });
                }
                Ok(LeaseOutcome::AlreadyLeased) => {
                    return Ok(LeaseOutcome::AlreadyLeased);
                }
                Ok(LeaseOutcome::NoLocalService) => {
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        Ok(LeaseOutcome::NoLocalService)
    }

    /// Attempt to lease a client by ID (used by outbox events).
    pub async fn lease_client_by_id(&self, mqtt_client_id: Uuid) -> Result<LeaseOutcome> {
        let client = mqtt_client::Entity::find_by_id(mqtt_client_id)
            .one(&self.db)
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to query mqtt_client".into(),
            ))?
            .ok_or_else(|| report!(LeaseCoordinatorError::MqttClientNotFound(mqtt_client_id)))?;

        self.lease_new_client_to_least_busy(&client).await
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

        if let Err(e) = mqtt_client_store::update_mqtt_clients_status(
            &self.db,
            mqtt_client_ids,
            MqttClientConnectionStatus::Offline,
        )
        .await
        {
            tracing::warn!(error = %e, "failed to mark MQTT clients offline after release");
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

            let client_ids: Vec<Uuid> = mqtt_client_ids.iter().copied().collect();
            if let Err(e) = mqtt_client_store::update_mqtt_clients_status(
                &self.db,
                &client_ids,
                MqttClientConnectionStatus::Offline,
            )
            .await
            {
                tracing::warn!(error = %e, "failed to mark MQTT clients offline after disconnect");
            }
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

        if let Err(e) = mqtt_client_store::update_mqtt_client_status(
            &self.db,
            mqtt_client_id,
            MqttClientConnectionStatus::Offline,
        )
        .await
        {
            tracing::warn!(error = %e, "failed to mark MQTT client offline after revoke");
        }

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

impl MqttLeaseCoordinator {
    async fn try_lease_to_service(
        &self,
        client: &mqtt_client::Model,
        candidate: &MqttServiceLoad,
    ) -> Result<LeaseOutcome> {
        let now = OffsetDateTime::now_utc();
        let lease = mqtt_lease::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7()),
            tenant_id: ActiveValue::Set(client.tenant_id),
            mqtt_client_id: ActiveValue::Set(client.id),
            instance_id: ActiveValue::Set(candidate.instance_id.clone()),
            heartbeat_at: ActiveValue::Set(now),
            created_at: ActiveValue::Set(now),
        };

        match mqtt_lease::Entity::insert(lease)
            .on_conflict(
                OnConflict::column(mqtt_lease::Column::MqttClientId)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(&self.db)
            .await
        {
            Ok(_) => {}
            Err(sea_orm::DbErr::RecordNotInserted) => {
                return Ok(LeaseOutcome::AlreadyLeased);
            }
            Err(e) => {
                bail!(LeaseCoordinatorError::Database(format!(
                    "failed to insert lease: {e}"
                )));
            }
        }

        let lease = mqtt_lease::Entity::find()
            .filter(mqtt_lease::Column::MqttClientId.eq(client.id))
            .one(&self.db)
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to load lease after insert".into(),
            ))?;
        let Some(lease) = lease else {
            bail!(LeaseCoordinatorError::Database(
                "lease missing after insert".into(),
            ));
        };

        if lease.instance_id != candidate.instance_id {
            return Ok(LeaseOutcome::AlreadyLeased);
        }

        if !self
            .connections
            .assign_mqtt_client(&candidate.service_id, client.id)
            .await
        {
            self.rollback_lease(client.id, &candidate.instance_id)
                .await?;
            return Ok(LeaseOutcome::NoLocalService);
        }

        let msg = ControllerMessage::TenantAssignments(MqttTenantAssignmentsPayload {
            tenants: vec![model_to_config(client)],
        });

        if !self.connections.send(&candidate.service_id, msg).await {
            self.connections
                .release_mqtt_client(&candidate.service_id, &client.id)
                .await;
            self.rollback_lease(client.id, &candidate.instance_id)
                .await?;
            return Ok(LeaseOutcome::NoLocalService);
        }

        Ok(LeaseOutcome::Leased {
            service_id: candidate.service_id,
        })
    }

    async fn rollback_lease(&self, mqtt_client_id: Uuid, instance_id: &str) -> Result<()> {
        mqtt_lease::Entity::delete_many()
            .filter(mqtt_lease::Column::MqttClientId.eq(mqtt_client_id))
            .filter(mqtt_lease::Column::InstanceId.eq(instance_id))
            .exec(&self.db)
            .await
            .context(LeaseCoordinatorError::Database(
                "failed to delete lease".into(),
            ))?;
        Ok(())
    }
}

/// Convert mqtt_client model to MqttTenantConfig wire type.
fn model_to_config(client: &mqtt_client::Model) -> MqttTenantConfig {
    MqttTenantConfig {
        mqtt_client_id: client.id,
        tenant_id: client.tenant_id,
        enabled: client.enabled,
        transport: client.transport,
        host: client.host.clone(),
        port: client.port as u16,
        client_id: client.client_id.clone(),
        username: client
            .username
            .as_ref()
            .map(|u| SecretString::new(u.clone())),
        password: client
            .password
            .as_ref()
            .map(|p| SecretString::new(p.expose_secret().to_string())),
        ca_pem: client
            .ca_cert_pem
            .as_ref()
            .map(|c| SecretString::new(c.expose_secret().to_string())),
        topic_prefix: client.topic_prefix.clone(),
        ha_discovery: client.ha_discovery,
        ha_discovery_prefix: client.ha_discovery_prefix.clone(),
        updated_at: UtcDateTime::from_unix_timestamp(client.updated_at.unix_timestamp())
            .unwrap_or(UtcDateTime::UNIX_EPOCH),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::sea_query::Index;
    use sea_orm::{
        ActiveModelTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Schema,
    };
    use std::collections::BTreeSet;
    use uptrakit_internal_wire::Capability;
    use uptrakit_shared_db::MqttTransport;
    use uptrakit_shared_db::entity::{mqtt_client, mqtt_lease, tenant};

    fn mqtt_caps() -> BTreeSet<Capability> {
        BTreeSet::from([Capability::GracefulShutdown, Capability::MqttBridge])
    }

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await.expect("test db");

        let schema = Schema::new(db.get_database_backend());
        let stmt = schema.create_table_from_entity(tenant::Entity);
        db.execute(&stmt).await.expect("create tenants table");
        let stmt = schema.create_table_from_entity(mqtt_client::Entity);
        db.execute(&stmt).await.expect("create mqtt_clients table");
        let stmt = schema.create_table_from_entity(mqtt_lease::Entity);
        db.execute(&stmt).await.expect("create mqtt_leases table");
        let stmt = Index::create()
            .name("uq_mqtt_leases_mqtt_client_id")
            .table(mqtt_lease::Entity)
            .col(mqtt_lease::Column::MqttClientId)
            .unique()
            .to_owned();
        db.execute(&stmt).await.expect("create mqtt_leases index");

        db
    }

    async fn seed_tenant(db: &DatabaseConnection) -> tenant::Model {
        let now = OffsetDateTime::now_utc();
        let model = tenant::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7()),
            name: ActiveValue::Set("Default".to_string()),
            slug: ActiveValue::Set("default".to_string()),
            is_default: ActiveValue::Set(true),
            created_at: ActiveValue::Set(now),
            updated_at: ActiveValue::Set(now),
            deactivated_at: ActiveValue::Set(None),
        };
        model.insert(db).await.expect("insert tenant")
    }

    async fn seed_client(db: &DatabaseConnection, tenant_id: Uuid) -> mqtt_client::Model {
        let now = OffsetDateTime::now_utc();
        let model = mqtt_client::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7()),
            tenant_id: ActiveValue::Set(tenant_id),
            enabled: ActiveValue::Set(true),
            transport: ActiveValue::Set(MqttTransport::Tcp),
            host: ActiveValue::Set("broker.local".to_string()),
            port: ActiveValue::Set(1883),
            client_id: ActiveValue::Set("uptrakit-controller".to_string()),
            username: ActiveValue::Set(None),
            password: ActiveValue::Set(None),
            ca_cert_pem: ActiveValue::Set(None),
            topic_prefix: ActiveValue::Set("uptrakit".to_string()),
            connection_status: ActiveValue::Set(MqttClientConnectionStatus::Offline),
            status_updated_at: ActiveValue::Set(now),
            ha_discovery: ActiveValue::Set(false),
            ha_discovery_prefix: ActiveValue::Set("homeassistant".to_string()),
            created_at: ActiveValue::Set(now),
            updated_at: ActiveValue::Set(now),
        };
        model.insert(db).await.expect("insert mqtt client")
    }

    #[tokio::test]
    async fn lease_new_client_to_local_service() {
        let db = test_db().await;
        let tenant = seed_tenant(&db).await;
        let client = seed_client(&db, tenant.id).await;

        let registry = ServiceConnectionRegistry::new();
        let service_id = Uuid::now_v7();
        let _rx = registry
            .register(
                service_id,
                mqtt_caps(),
                Some("instance-1".to_string()),
                Some(10),
            )
            .await;

        let coordinator = MqttLeaseCoordinator::new(db.clone(), registry.clone());
        let result = coordinator
            .lease_new_client_to_least_busy(&client)
            .await
            .expect("lease");

        assert!(matches!(result, LeaseOutcome::Leased { .. }));

        let lease = mqtt_lease::Entity::find()
            .one(&db)
            .await
            .expect("find lease")
            .expect("lease row");
        assert_eq!(lease.mqtt_client_id, client.id);
        assert_eq!(registry.assigned_mqtt_client_count(&service_id).await, 1);
    }

    #[tokio::test]
    async fn lease_new_client_without_local_service() {
        let db = test_db().await;
        let tenant = seed_tenant(&db).await;
        let client = seed_client(&db, tenant.id).await;

        let registry = ServiceConnectionRegistry::new();
        let coordinator = MqttLeaseCoordinator::new(db.clone(), registry);
        let result = coordinator
            .lease_new_client_to_least_busy(&client)
            .await
            .expect("lease");

        assert!(matches!(result, LeaseOutcome::NoLocalService));

        let lease = mqtt_lease::Entity::find()
            .one(&db)
            .await
            .expect("find lease");
        assert!(lease.is_none());
    }

    #[tokio::test]
    async fn lease_new_client_already_leased() {
        let db = test_db().await;
        let tenant = seed_tenant(&db).await;
        let client = seed_client(&db, tenant.id).await;

        let registry = ServiceConnectionRegistry::new();
        let service_id = Uuid::now_v7();
        let _ = registry
            .register(
                service_id,
                mqtt_caps(),
                Some("instance-1".to_string()),
                Some(10),
            )
            .await;

        let now = OffsetDateTime::now_utc();
        let existing = mqtt_lease::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7()),
            tenant_id: ActiveValue::Set(client.tenant_id),
            mqtt_client_id: ActiveValue::Set(client.id),
            instance_id: ActiveValue::Set("other".to_string()),
            heartbeat_at: ActiveValue::Set(now),
            created_at: ActiveValue::Set(now),
        };
        existing.insert(&db).await.expect("insert lease");

        let coordinator = MqttLeaseCoordinator::new(db, registry);
        let result = coordinator
            .lease_new_client_to_least_busy(&client)
            .await
            .expect("lease");

        assert!(matches!(result, LeaseOutcome::AlreadyLeased));
    }

    #[tokio::test]
    async fn assign_available_tenants_skips_already_leased_clients()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let db = test_db().await;
        let tenant = seed_tenant(&db).await;
        let client_a = seed_client(&db, tenant.id).await;
        let client_b = seed_client(&db, tenant.id).await;

        let now = OffsetDateTime::now_utc();
        let existing = mqtt_lease::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7()),
            tenant_id: ActiveValue::Set(client_a.tenant_id),
            mqtt_client_id: ActiveValue::Set(client_a.id),
            instance_id: ActiveValue::Set("other".to_string()),
            heartbeat_at: ActiveValue::Set(now),
            created_at: ActiveValue::Set(now),
        };
        existing.insert(&db).await?;

        let registry = ServiceConnectionRegistry::new();
        let service_id = Uuid::now_v7();
        let _ = registry
            .register(
                service_id,
                mqtt_caps(),
                Some("instance-1".to_string()),
                Some(10),
            )
            .await;

        let coordinator = MqttLeaseCoordinator::new(db.clone(), registry.clone());
        let configs = match coordinator
            .assign_available_tenants(service_id, "instance-1", 2)
            .await
        {
            Ok(configs) => configs,
            Err(e) => return Err(format!("{e}").into()),
        };

        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].mqtt_client_id, client_b.id);

        let leases = mqtt_lease::Entity::find().all(&db).await?;
        assert_eq!(leases.len(), 2);
        Ok(())
    }
}
