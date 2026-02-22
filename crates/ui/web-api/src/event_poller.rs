use std::collections::HashMap;
use std::time::Duration;

use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect,
};
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use uptrakit_internal_wire::{
    ControllerMessage, MqttClientCreatedPayload, MqttTenantRevokedPayload,
};
use uptrakit_shared_db::entity::controller_event;
use uuid::Uuid;

use crate::mqtt_lease_coordinator::{LeaseCoordinatorError, MqttLeaseCoordinator};
use crate::service_connections::ServiceConnectionRegistry;

/// Maximum number of delivery retries before an event is skipped.
const MAX_DELIVERY_RETRIES: u8 = 3;
/// Retention window for controller events before cleanup.
pub const EVENT_CLEANUP_TTL_HOURS: i64 = 24;
/// Safety margin to avoid missing events between startup and first poll.
const STARTUP_CURSOR_SAFETY_MARGIN: i64 = 100;

/// Background task that polls the `controller_events` table for events written by
/// other controller instances and delivers them to locally connected services.
pub struct EventPoller {
    db: DatabaseConnection,
    registry: ServiceConnectionRegistry,
    controller_id: Uuid,
    /// Tracks delivery retry counts for events that failed delivery.
    retry_counts: HashMap<i64, u8>,
}

impl EventPoller {
    pub fn new(
        db: DatabaseConnection,
        registry: ServiceConnectionRegistry,
        controller_id: Uuid,
    ) -> Self {
        Self {
            db,
            registry,
            controller_id,
            retry_counts: HashMap::new(),
        }
    }

    /// Run the event poller until the cancellation token is triggered.
    pub async fn run(mut self, token: CancellationToken) {
        // Initialize cursor with a safety margin to avoid missing events on restart.
        let mut last_seen_id = self.fetch_max_id().await;
        tracing::info!(
            controller_id = %self.controller_id,
            last_seen_id,
            "event poller started"
        );

        let mut poll_interval = tokio::time::interval(Duration::from_secs(1));
        // Skip the first immediate tick
        poll_interval.tick().await;

        // NOTE: Event cleanup is handled by the centralised scheduler
        // (EventCleanupExecutor). This loop only polls for new events.
        loop {
            tokio::select! {
                _ = poll_interval.tick() => {
                    last_seen_id = self.poll_events(last_seen_id).await;
                }
                _ = token.cancelled() => {
                    tracing::debug!("event poller shutting down");
                    return;
                }
            }
        }
    }

    /// Fetch the current maximum event ID.
    async fn fetch_max_id(&self) -> i64 {
        match controller_event::Entity::find()
            .order_by(controller_event::Column::Id, Order::Desc)
            .select_only()
            .column(controller_event::Column::Id)
            .into_tuple::<i64>()
            .one(&self.db)
            .await
        {
            Ok(Some(max_id)) => max_id.saturating_sub(STARTUP_CURSOR_SAFETY_MARGIN),
            Ok(None) => 0,
            Err(e) => {
                tracing::warn!(error = %e, "failed to fetch max event ID, starting from 0");
                0
            }
        }
    }

    /// Poll for new events from other controllers. Returns the updated cursor.
    ///
    /// Only advances the cursor past events that were successfully delivered (or
    /// permanently skipped after [`MAX_DELIVERY_RETRIES`] failures). This
    /// prevents message loss under backpressure.
    async fn poll_events(&mut self, last_seen_id: i64) -> i64 {
        let events = match controller_event::Entity::find()
            .filter(controller_event::Column::SourceControllerId.ne(self.controller_id))
            .filter(controller_event::Column::Id.gt(last_seen_id))
            .order_by_asc(controller_event::Column::Id)
            .limit(100)
            .all(&self.db)
            .await
        {
            Ok(events) => events,
            Err(e) => {
                tracing::warn!(error = %e, "failed to poll controller events");
                return last_seen_id;
            }
        };

        let mut new_cursor = last_seen_id;

        for event in events {
            let event_id = event.id;

            if let Some(service_id) = event.target_service_id
                && let Some(connected_at) = self.registry.connected_at(&service_id).await
                && event.created_at < connected_at
            {
                tracing::debug!(
                    event_id,
                    %service_id,
                    "skipping stale event created before service connected"
                );
                new_cursor = event_id;
                continue;
            }

            let msg: ControllerMessage = match serde_json::from_value(event.message_json.clone()) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(
                        event_id,
                        error = %e,
                        "failed to deserialize controller event, skipping"
                    );
                    // Deserialization failures are permanent — advance past them.
                    new_cursor = event_id;
                    continue;
                }
            };

            let delivered = self
                .deliver_event(
                    event.target_service_id,
                    event.target_service_type.as_deref(),
                    msg,
                )
                .await;

            if delivered {
                new_cursor = event_id;
                self.retry_counts.remove(&event_id);
            } else {
                let retries = self.retry_counts.entry(event_id).or_insert(0);
                *retries += 1;
                if *retries >= MAX_DELIVERY_RETRIES {
                    tracing::warn!(
                        event_id,
                        retries = *retries,
                        "event exceeded max delivery retries, skipping"
                    );
                    new_cursor = event_id;
                    self.retry_counts.remove(&event_id);
                } else {
                    // Stop processing this batch — retry from this event next poll.
                    break;
                }
            }
        }

        // Clean up retry entries for events we've moved past.
        self.retry_counts.retain(|&id, _| id > new_cursor);

        new_cursor
    }

    /// Deliver a single event to the appropriate local service(s).
    ///
    /// Returns `true` if the message was delivered (or the target is not on
    /// this controller), `false` if delivery failed (channel full / send error).
    async fn deliver_event(
        &self,
        target_service_id: Option<Uuid>,
        target_service_type: Option<&str>,
        msg: ControllerMessage,
    ) -> bool {
        // Controller-targeted events are handled locally (not forwarded to services)
        if target_service_id.is_none() && target_service_type == Some("controller") {
            return self.deliver_controller_event(msg).await;
        }

        let parsed_service_type =
            target_service_type.and_then(|service_type| match service_type
                .parse::<uptrakit_internal_wire::ServiceType>(
            ) {
                Ok(parsed) => Some(parsed),
                Err(_) => {
                    tracing::warn!(
                        value = %service_type,
                        "unknown target_service_type in outbox event"
                    );
                    None
                }
            });

        match (target_service_id, parsed_service_type) {
            // Targeted to a specific service
            (Some(id), _) => {
                if self.registry.is_connected(&id).await {
                    self.registry.send(&id, msg).await
                } else {
                    // Service not on this controller — not our responsibility.
                    true
                }
            }
            // Targeted to MQTT services
            (None, Some(uptrakit_internal_wire::ServiceType::Mqtt)) => {
                self.deliver_mqtt_event(msg).await
            }
            // Targeted to agent services
            (None, Some(uptrakit_internal_wire::ServiceType::Agent)) => {
                self.registry
                    .broadcast_by_type(uptrakit_shared_db::entity::service::ServiceType::Agent, msg)
                    .await;
                true
            }
            // Targeted to SSH agent services
            (None, Some(uptrakit_internal_wire::ServiceType::SshAgent)) => {
                self.registry
                    .broadcast_by_type(
                        uptrakit_shared_db::entity::service::ServiceType::SshAgent,
                        msg,
                    )
                    .await;
                true
            }
            // Broadcast to all services (no type filter or unknown type)
            (None, None) => {
                self.registry.broadcast(msg).await;
                true
            }
        }
    }

    /// Deliver an MQTT-targeted event with special routing for tenant messages.
    ///
    /// Returns `true` if delivery succeeded or the target is not on this
    /// controller.
    async fn deliver_mqtt_event(&self, msg: ControllerMessage) -> bool {
        match &msg {
            ControllerMessage::TenantConfigUpdated(payload) => {
                // Route to the specific instance holding this MQTT client
                let mqtt_client_id = payload.tenant.mqtt_client_id;
                if let Some(service_id) = self
                    .registry
                    .get_instance_for_mqtt_client(&mqtt_client_id)
                    .await
                {
                    self.registry.send(&service_id, msg).await
                } else {
                    // Not on this controller.
                    true
                }
            }
            ControllerMessage::TenantRevoked(MqttTenantRevokedPayload {
                mqtt_client_id, ..
            }) => {
                // Route to the specific instance holding this MQTT client
                let mqtt_client_id = *mqtt_client_id;
                if let Some(service_id) = self
                    .registry
                    .get_instance_for_mqtt_client(&mqtt_client_id)
                    .await
                {
                    self.registry
                        .release_mqtt_client(&service_id, &mqtt_client_id)
                        .await;
                    self.registry.send(&service_id, msg).await
                } else {
                    // Not on this controller.
                    true
                }
            }
            _ => {
                // Other MQTT messages: broadcast to all local MQTT services
                self.registry
                    .broadcast_by_type(uptrakit_shared_db::entity::service::ServiceType::Mqtt, msg)
                    .await;
                true
            }
        }
    }

    async fn deliver_controller_event(&self, msg: ControllerMessage) -> bool {
        match msg {
            ControllerMessage::MqttClientCreated(MqttClientCreatedPayload { mqtt_client_id }) => {
                let coordinator = MqttLeaseCoordinator::new(self.db.clone(), self.registry.clone());
                match coordinator.lease_client_by_id(mqtt_client_id).await {
                    Ok(_) => true,
                    Err(e) => {
                        if matches!(
                            e.current_context(),
                            LeaseCoordinatorError::MqttClientNotFound(_)
                        ) {
                            return true;
                        }
                        tracing::warn!(
                            error = %e,
                            %mqtt_client_id,
                            "failed to lease MQTT client from outbox event"
                        );
                        false
                    }
                }
            }
            _ => true,
        }
    }

    /// Delete events older than the cleanup TTL.
    ///
    /// This is public so the centralised scheduler (`EventCleanupExecutor`)
    /// can run the same logic without duplicating it. The `EventPoller::run()`
    /// loop itself no longer calls this directly.
    pub async fn cleanup_old_events(&self) {
        let cutoff = OffsetDateTime::now_utc() - time::Duration::hours(EVENT_CLEANUP_TTL_HOURS);
        match controller_event::Entity::delete_many()
            .filter(controller_event::Column::CreatedAt.lt(cutoff))
            .exec(&self.db)
            .await
        {
            Ok(result) => {
                if result.rows_affected > 0 {
                    tracing::debug!(
                        deleted = result.rows_affected,
                        "cleaned up old controller events"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to clean up old controller events");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{
        ActiveModelTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Schema,
        Set,
    };
    use uptrakit_shared_db::entity::controller_event;

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        Database::connect(opt).await.expect("test db")
    }

    async fn setup_test_db() -> DatabaseConnection {
        let db = test_db().await;
        let schema = Schema::new(db.get_database_backend());
        let stmt = schema.create_table_from_entity(controller_event::Entity);
        db.execute(&stmt)
            .await
            .expect("create controller_events table");
        db
    }

    #[tokio::test]
    async fn fetch_max_id_uses_safety_margin() {
        let db = setup_test_db().await;
        let now = OffsetDateTime::now_utc();
        for _ in 0..150 {
            let event = controller_event::ActiveModel {
                source_controller_id: Set(Uuid::now_v7()),
                target_service_id: Set(None),
                target_service_type: Set(Some("agent".to_string())),
                message_json: Set(serde_json::json!({"type": "ping"})),
                created_at: Set(now),
                ..Default::default()
            };
            let _ = event.insert(&db).await;
        }

        let poller = EventPoller::new(db, ServiceConnectionRegistry::new(), Uuid::now_v7());
        let cursor = poller.fetch_max_id().await;
        assert_eq!(cursor, 50);
    }

    #[tokio::test]
    async fn skips_events_older_than_connection() {
        let db = setup_test_db().await;
        let registry = ServiceConnectionRegistry::new();
        let service_id = Uuid::now_v7();
        let (mut rx, _token) = registry.register_agent(service_id).await;

        let old_time = OffsetDateTime::now_utc() - time::Duration::seconds(30);
        let event = controller_event::ActiveModel {
            source_controller_id: Set(Uuid::now_v7()),
            target_service_id: Set(Some(service_id)),
            target_service_type: Set(Some("agent".to_string())),
            message_json: Set(serde_json::json!({"type": "ping"})),
            created_at: Set(old_time),
            ..Default::default()
        };
        let event = event.insert(&db).await.expect("insert event");

        let mut poller = EventPoller::new(db, registry, Uuid::now_v7());
        let cursor = poller.poll_events(0).await;
        assert_eq!(cursor, event.id);
        assert!(rx.try_recv().is_err());
    }
}
