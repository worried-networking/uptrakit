use std::time::Duration;

use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect,
};
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use uptrakit_internal_wire::{ControllerMessage, MqttTenantRevokedPayload};
use uptrakit_shared_db::entity::controller_event;
use uuid::Uuid;

use crate::service_connections::ServiceConnectionRegistry;

/// Background task that polls the `controller_events` table for events written by
/// other controller instances and delivers them to locally connected services.
pub struct EventPoller {
    db: DatabaseConnection,
    registry: ServiceConnectionRegistry,
    controller_id: Uuid,
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
        }
    }

    /// Run the event poller until the cancellation token is triggered.
    pub async fn run(self, token: CancellationToken) {
        // Initialize cursor to current max ID to avoid replaying old events.
        let mut last_seen_id = self.fetch_max_id().await;
        tracing::info!(
            controller_id = %self.controller_id,
            last_seen_id,
            "event poller started"
        );

        let mut poll_interval = tokio::time::interval(Duration::from_secs(1));
        let mut cleanup_interval = tokio::time::interval(Duration::from_secs(300));
        // Skip the first immediate ticks
        poll_interval.tick().await;
        cleanup_interval.tick().await;

        loop {
            tokio::select! {
                _ = poll_interval.tick() => {
                    last_seen_id = self.poll_events(last_seen_id).await;
                }
                _ = cleanup_interval.tick() => {
                    self.cleanup_old_events().await;
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
            Ok(Some(max_id)) => max_id,
            Ok(None) => 0,
            Err(e) => {
                tracing::warn!(error = %e, "failed to fetch max event ID, starting from 0");
                0
            }
        }
    }

    /// Poll for new events from other controllers. Returns the updated cursor.
    async fn poll_events(&self, last_seen_id: i64) -> i64 {
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
            new_cursor = event.id;

            let msg: ControllerMessage = match serde_json::from_str(&event.message_json) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(
                        event_id = event.id,
                        error = %e,
                        "failed to deserialize controller event, skipping"
                    );
                    continue;
                }
            };

            self.deliver_event(
                event.target_service_id,
                event.target_service_type.as_deref(),
                msg,
            )
            .await;
        }

        new_cursor
    }

    /// Deliver a single event to the appropriate local service(s).
    async fn deliver_event(
        &self,
        target_service_id: Option<Uuid>,
        target_service_type: Option<&str>,
        msg: ControllerMessage,
    ) {
        match (target_service_id, target_service_type) {
            // Targeted to a specific service
            (Some(id), _) => {
                self.registry.send(&id, msg).await;
            }
            // Targeted to MQTT services
            (None, Some("mqtt")) => {
                self.deliver_mqtt_event(msg).await;
            }
            // Targeted to agent services
            (None, Some("agent")) => {
                self.registry
                    .broadcast_by_type(uptrakit_shared_db::entity::service::ServiceType::Agent, msg)
                    .await;
            }
            // Broadcast to all services
            (None, _) => {
                self.registry.broadcast(msg).await;
            }
        }
    }

    /// Deliver an MQTT-targeted event with special routing for tenant messages.
    async fn deliver_mqtt_event(&self, msg: ControllerMessage) {
        match &msg {
            ControllerMessage::TenantConfigUpdated(payload) => {
                // Route to the specific instance holding this MQTT client
                let mqtt_client_id = payload.tenant.mqtt_client_id;
                if let Some(service_id) = self
                    .registry
                    .get_instance_for_mqtt_client(&mqtt_client_id)
                    .await
                {
                    self.registry.send(&service_id, msg).await;
                }
                // If no local holder, the event is not for this controller
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
                    self.registry.send(&service_id, msg).await;
                }
                // If no local holder, the event is not for this controller
            }
            _ => {
                // Other MQTT messages: broadcast to all local MQTT services
                self.registry
                    .broadcast_by_type(uptrakit_shared_db::entity::service::ServiceType::Mqtt, msg)
                    .await;
            }
        }
    }

    /// Delete events older than 1 hour.
    async fn cleanup_old_events(&self) {
        let cutoff = OffsetDateTime::now_utc() - time::Duration::hours(1);
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
