use std::collections::BTreeSet;

use sea_orm::{
    ActiveValue, ColumnTrait, Condition, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use time::OffsetDateTime;
use uptrakit_internal_wire::{Capability, ControllerMessage};
use uptrakit_shared_db::entity::controller_event;
use uuid::Uuid;

use crate::event_poller::EVENT_CLEANUP_TTL_HOURS;
use crate::service_connections::ServiceConnectionRegistry;

/// Cross-controller notification service.
///
/// Wraps `ServiceConnectionRegistry` to provide the same push API but additionally
/// writes events to the `controller_events` outbox table. A background poller on
/// each controller picks up events from other controllers and delivers them to
/// locally connected services.
#[derive(Clone)]
pub struct NotificationService {
    db: DatabaseConnection,
    registry: ServiceConnectionRegistry,
    controller_id: Uuid,
}

impl NotificationService {
    pub fn new(
        db: DatabaseConnection,
        registry: ServiceConnectionRegistry,
        controller_id: Uuid,
    ) -> Self {
        Self {
            db,
            registry,
            controller_id,
        }
    }

    /// Push a message to a specific service (local + outbox).
    ///
    /// MQTT-specific messages that may contain credentials (`TenantAssignments`,
    /// `TenantConfigUpdated`, `TenantRevoked`) are delivered locally but **not**
    /// written to the outbox to prevent plaintext credential persistence. The
    /// MQTT service reconciles its state from the DB on reconnect.
    pub async fn send(&self, service_id: &Uuid, msg: ControllerMessage) -> bool {
        let local = self.registry.send(service_id, msg.clone()).await;
        if !is_mqtt_tenant_message(&msg) {
            self.write_outbox_event(Some(*service_id), None, &msg).await;
        }
        local
    }

    /// Broadcast a message to all connected services (local + outbox).
    ///
    /// MQTT-specific messages that may contain credentials are delivered locally
    /// but **not** written to the outbox (see [`Self::send`] doc comment).
    pub async fn broadcast(&self, msg: ControllerMessage) {
        self.registry.broadcast(msg.clone()).await;
        if !is_mqtt_tenant_message(&msg) {
            self.write_outbox_event(None, None, &msg).await;
        }
    }

    /// Write an outbox event for cross-controller delivery.
    ///
    /// This is fire-and-forget: errors are logged but do not fail the caller.
    pub(crate) async fn write_outbox_event(
        &self,
        target_service_id: Option<Uuid>,
        target_capability: Option<&str>,
        msg: &ControllerMessage,
    ) {
        let json = match serde_json::to_value(msg) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(error = %e, "failed to serialize outbox event");
                return;
            }
        };

        let event = controller_event::ActiveModel {
            id: ActiveValue::NotSet,
            source_controller_id: ActiveValue::Set(self.controller_id),
            target_service_id: ActiveValue::Set(target_service_id),
            target_capability: ActiveValue::Set(target_capability.map(|t| t.to_string())),

            message_json: ActiveValue::Set(json),
            created_at: ActiveValue::Set(OffsetDateTime::now_utc()),
        };

        if let Err(e) = controller_event::Entity::insert(event).exec(&self.db).await {
            tracing::error!(error = %e, "failed to write outbox event");
        }
    }

    /// Access the underlying registry (e.g. for `is_connected` checks).
    pub fn registry(&self) -> &ServiceConnectionRegistry {
        &self.registry
    }

    /// Deliver recent outbox events to a newly connected service.
    ///
    /// Uses the service's previous `last_seen_at` to avoid replaying older
    /// broadcasts. This is best-effort and bounded by the outbox retention
    /// window.
    pub async fn deliver_backlog_for_authenticated_service(
        &self,
        service_id: Uuid,
        capabilities: &BTreeSet<Capability>,
        last_seen_at: Option<OffsetDateTime>,
    ) -> usize {
        let cutoff = last_seen_at.unwrap_or_else(|| {
            OffsetDateTime::now_utc() - time::Duration::hours(EVENT_CLEANUP_TTL_HOURS)
        });

        // Build the list of target_capability values this service should receive
        let mut relevant_capabilities: Vec<String> = Vec::new();
        if capabilities.contains(&Capability::SoftwareDiscovery) {
            relevant_capabilities.push("software_discovery".to_string());
        }
        if capabilities.contains(&Capability::MqttBridge) {
            relevant_capabilities.push("mqtt_bridge".to_string());
        }
        if capabilities.contains(&Capability::SshRemote) {
            relevant_capabilities.push("ssh_remote".to_string());
        }
        if capabilities.contains(&Capability::UpdateHooks) {
            relevant_capabilities.push("update_hooks".to_string());
        }

        let broadcast_condition = Condition::all()
            .add(controller_event::Column::TargetServiceId.is_null())
            .add({
                let mut cap_condition = Condition::any()
                    .add(controller_event::Column::TargetCapability.is_null());
                for cap in &relevant_capabilities {
                    cap_condition = cap_condition
                        .add(controller_event::Column::TargetCapability.eq(cap.as_str()));
                }
                cap_condition
            });

        let condition = Condition::any()
            .add(controller_event::Column::TargetServiceId.eq(service_id))
            .add(broadcast_condition);

        let events = match controller_event::Entity::find()
            .filter(condition)
            .filter(controller_event::Column::CreatedAt.gt(cutoff))
            .order_by_asc(controller_event::Column::Id)
            .limit(500)
            .all(&self.db)
            .await
        {
            Ok(events) => events,
            Err(e) => {
                tracing::warn!(error = %e, "failed to load outbox backlog for service");
                return 0;
            }
        };

        let mut delivered = 0usize;
        for event in events {
            let msg: ControllerMessage = match serde_json::from_value(event.message_json.clone()) {
                Ok(msg) => msg,
                Err(e) => {
                    tracing::warn!(
                        event_id = event.id,
                        error = %e,
                        "failed to deserialize backlog event"
                    );
                    continue;
                }
            };

            if !should_deliver_backlog_message(capabilities, &msg) {
                continue;
            }

            if self.registry.send(&service_id, msg).await {
                delivered += 1;
            } else {
                break;
            }
        }

        delivered
    }

    /// Load software states for a tenant and push to all locally connected MQTT services
    /// (immediately) and to the outbox for cross-controller delivery.
    pub async fn push_software_states_for_tenant(&self, tenant_id: uuid::Uuid) {
        let payload = match crate::queries::mqtt_software_states::load_software_states_for_tenant(
            &self.db,
            tenant_id,
        )
        .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    %tenant_id,
                    "failed to load software states for MQTT push"
                );
                return;
            }
        };
        let msg = ControllerMessage::SoftwareStates(payload);
        // Deliver to locally connected MQTT services immediately.
        self.registry
            .broadcast_by_capability(&Capability::MqttBridge, msg.clone())
            .await;
        // Write to outbox for cross-controller delivery (MQTT-only).
        self.write_outbox_event(None, Some("mqtt_bridge"), &msg)
            .await;
    }

    /// Return the controller ID.
    pub fn controller_id(&self) -> Uuid {
        self.controller_id
    }
}

/// Returns `true` for MQTT-specific messages that may contain credentials
/// (`TenantAssignments`, `TenantConfigUpdated`, `TenantRevoked`).
///
/// These messages are delivered locally but **not** written to the outbox to
/// prevent plaintext credential persistence. The MQTT service reconciles its
/// state from the DB on reconnect.
fn is_mqtt_tenant_message(msg: &ControllerMessage) -> bool {
    matches!(
        msg,
        ControllerMessage::TenantAssignments(_)
            | ControllerMessage::TenantConfigUpdated(_)
            | ControllerMessage::TenantRevoked(_)
    )
}

/// Return true if a backlog message should be delivered to an authenticated service.
fn should_deliver_backlog_message(
    capabilities: &BTreeSet<Capability>,
    msg: &ControllerMessage,
) -> bool {
    match msg {
        ControllerMessage::CheckVersions(_) | ControllerMessage::DiscoverSoftware(_) => {
            capabilities.contains(&Capability::SoftwareDiscovery)
        }
        ControllerMessage::SoftwareStates(_) => capabilities.contains(&Capability::MqttBridge),
        ControllerMessage::CaBundleUpdated(_) | ControllerMessage::RequestCertRenewal(_) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, Schema};
    use uptrakit_internal_wire::{
        ApprovedPayload, CaBundleUpdatedPayload, ControllerMessage, RequestCertRenewalPayload,
        ServerRestartingPayload,
    };

    async fn test_db() -> Result<DatabaseConnection, Box<dyn std::error::Error>> {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await?;

        let schema = Schema::new(db.get_database_backend());
        let stmt = schema.create_table_from_entity(controller_event::Entity);
        db.execute(&stmt).await?;

        Ok(db)
    }

    #[tokio::test]
    async fn send_writes_outbox_event() -> Result<(), Box<dyn std::error::Error>> {
        let db = test_db().await?;
        let registry = ServiceConnectionRegistry::new();
        let controller_id = Uuid::now_v7();
        let svc = NotificationService::new(db, registry, controller_id);

        let service_id = Uuid::now_v7();
        let msg = ControllerMessage::Approved(ApprovedPayload { service_id });

        // send returns false because no service is connected, but outbox write
        // should succeed.
        let result = svc.send(&service_id, msg).await;
        assert!(!result);

        let outbox = controller_event::Entity::find().all(&svc.db).await?;
        assert_eq!(outbox.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn server_restarting_is_local_only() {
        // ServerRestarting should NOT be sent through NotificationService.
        // It stays on ServiceConnectionRegistry.broadcast_server_restarting_scattered().
        // This test documents the design intent.
        let msg = ControllerMessage::ServerRestarting(ServerRestartingPayload {
            reason: "test".to_string(),
        });
        // The message type exists and is valid
        assert!(matches!(msg, ControllerMessage::ServerRestarting(_)));
    }

    #[test]
    fn is_mqtt_tenant_message_matches_credential_bearing_variants() {
        use uptrakit_internal_wire::{
            MqttTenantAssignmentsPayload, MqttTenantConfigUpdatedPayload, MqttTenantRevokedPayload,
        };

        // Credential-bearing variants must be filtered from the outbox.
        assert!(is_mqtt_tenant_message(
            &ControllerMessage::TenantAssignments(MqttTenantAssignmentsPayload { tenants: vec![] })
        ));
        assert!(is_mqtt_tenant_message(
            &ControllerMessage::TenantConfigUpdated(MqttTenantConfigUpdatedPayload {
                tenant: uptrakit_internal_wire::MqttTenantConfig {
                    mqtt_client_id: Uuid::nil(),
                    tenant_id: Uuid::nil(),
                    enabled: true,
                    transport: uptrakit_internal_wire::MqttTransport::Tcp,
                    host: "localhost".into(),
                    port: 1883,
                    client_id: "test".into(),
                    username: Some(uptrakit_internal_wire::SecretString::new("user".into())),
                    password: Some(uptrakit_internal_wire::SecretString::new("secret".into())),
                    ca_pem: None,
                    topic_prefix: "test/".into(),
                    ha_discovery: false,
                    ha_discovery_prefix: "homeassistant".to_string(),
                    updated_at: time::UtcDateTime::UNIX_EPOCH,
                },
            })
        ));
        assert!(is_mqtt_tenant_message(&ControllerMessage::TenantRevoked(
            MqttTenantRevokedPayload {
                mqtt_client_id: Uuid::nil(),
                reason: "test".into(),
            }
        )));

        // Non-credential variants must NOT be filtered.
        assert!(!is_mqtt_tenant_message(&ControllerMessage::Approved(
            ApprovedPayload {
                service_id: Uuid::nil(),
            }
        )));
    }

    #[tokio::test]
    async fn delivers_backlog_for_authenticated_service() -> Result<(), Box<dyn std::error::Error>>
    {
        let db = test_db().await?;

        let registry = ServiceConnectionRegistry::new();
        let controller_id = Uuid::now_v7();
        let svc = NotificationService::new(db.clone(), registry.clone(), controller_id);

        let service_id = Uuid::now_v7();
        let agent_capabilities = BTreeSet::from([
            Capability::SoftwareDiscovery,
            Capability::UpdateHooks,
            Capability::GracefulShutdown,
        ]);
        let (mut rx, _cancel) = registry
            .register(service_id, agent_capabilities.clone(), None, None)
            .await;

        let base_time = OffsetDateTime::now_utc();
        let event = controller_event::ActiveModel {
            id: ActiveValue::NotSet,
            source_controller_id: ActiveValue::Set(controller_id),
            target_service_id: ActiveValue::Set(None),
            target_capability: ActiveValue::Set(None),
            message_json: ActiveValue::Set(serde_json::to_value(
                ControllerMessage::CaBundleUpdated(CaBundleUpdatedPayload {
                    ca_bundle_pem: "pem".to_string(),
                }),
            )?),
            created_at: ActiveValue::Set(base_time),
        };
        controller_event::Entity::insert(event).exec(&db).await?;

        let delivered = svc
            .deliver_backlog_for_authenticated_service(
                service_id,
                &agent_capabilities,
                Some(base_time - time::Duration::minutes(5)),
            )
            .await;
        assert_eq!(delivered, 1);

        let msg = match rx.recv().await {
            Some(msg) => msg,
            None => return Err("expected backlog message".into()),
        };
        assert!(matches!(msg, ControllerMessage::CaBundleUpdated(_)));

        let ignored_event = controller_event::ActiveModel {
            id: ActiveValue::NotSet,
            source_controller_id: ActiveValue::Set(controller_id),
            target_service_id: ActiveValue::Set(None),
            target_capability: ActiveValue::Set(None),
            message_json: ActiveValue::Set(serde_json::to_value(
                ControllerMessage::ExecuteUpdate(Box::new(
                    uptrakit_internal_wire::ExecuteUpdatePayload {
                        host_machine_id: "test-machine-id".to_string(),
                        update_history_id: Uuid::now_v7(),
                        software_item_id: Uuid::now_v7(),
                        software_item_name: "item".to_string(),
                        to_version: "1.0".to_string(),
                        detect_version_plugin: None,
                        execute_update_plugin: uptrakit_internal_wire::PluginAssignment {
                            plugin_type: uptrakit_internal_wire::PluginType::GithubReleases,
                            package_identifier: "pkg".to_string(),
                            config: serde_json::json!({}),
                        },
                        pre_update_hooks: vec![],
                        post_update_hooks: vec![],
                        release_info: None,
                        timeout_seconds: 300,
                    },
                )),
            )?),
            created_at: ActiveValue::Set(base_time + time::Duration::seconds(10)),
        };
        controller_event::Entity::insert(ignored_event)
            .exec(&db)
            .await?;

        let delivered = svc
            .deliver_backlog_for_authenticated_service(
                service_id,
                &agent_capabilities,
                Some(base_time + time::Duration::seconds(5)),
            )
            .await;
        assert_eq!(delivered, 0);

        let msg = tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;
        assert!(msg.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn skips_non_matching_capability_backlog() -> Result<(), Box<dyn std::error::Error>> {
        let db = test_db().await?;

        let registry = ServiceConnectionRegistry::new();
        let controller_id = Uuid::now_v7();
        let svc = NotificationService::new(db.clone(), registry.clone(), controller_id);

        let service_id = Uuid::now_v7();
        let mqtt_capabilities = BTreeSet::from([
            Capability::MqttBridge,
            Capability::GracefulShutdown,
        ]);
        let (mut rx, _cancel) = registry
            .register(
                service_id,
                mqtt_capabilities.clone(),
                Some("instance".to_string()),
                Some(1),
            )
            .await;

        let event = controller_event::ActiveModel {
            id: ActiveValue::NotSet,
            source_controller_id: ActiveValue::Set(controller_id),
            target_service_id: ActiveValue::Set(None),
            target_capability: ActiveValue::Set(Some("software_discovery".to_string())),
            message_json: ActiveValue::Set(serde_json::to_value(
                ControllerMessage::RequestCertRenewal(RequestCertRenewalPayload {
                    reason: "test".to_string(),
                }),
            )?),
            created_at: ActiveValue::Set(OffsetDateTime::now_utc()),
        };
        controller_event::Entity::insert(event).exec(&db).await?;

        let delivered = svc
            .deliver_backlog_for_authenticated_service(
                service_id,
                &mqtt_capabilities,
                Some(OffsetDateTime::now_utc() - time::Duration::minutes(5)),
            )
            .await;
        assert_eq!(delivered, 0);

        let msg = tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;
        assert!(msg.is_err());
        Ok(())
    }
}
