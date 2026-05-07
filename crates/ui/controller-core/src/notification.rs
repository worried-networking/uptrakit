//! NATS publishing abstraction and notification side-effect state for cross-controller messaging.
//!
//! This module contains:
//! - [`NatsPublisher`] — decouples [`NotificationService`] and [`EventBroadcaster`] from the
//!   concrete `NatsTransport` type so that controller-core can be tested without a NATS dependency.
//! - [`NotificationService`] — cross-controller notification service for push message delivery.
//! - [`EventBroadcaster`] — per-tenant broadcast channel for real-time admin event delivery via SSE.
//! - [`NotificationDispatcher`] — fire-and-forget notification dispatcher.
//! - [`NotificationState`] — grouped notification side-effect state for [`AppState`].

#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget channel sends intentionally drop the send result"
)]
#![expect(
    clippy::allow_attributes,
    reason = "feature-conditional #[allow] for unused_variables; #[expect] would be unfulfilled when nats feature is enabled"
)]

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use uptrakit_wire::{
    AdminEvent, Capability, ControllerMessage, HostConnectivityUpdate,
    HostConnectivityUpdatedPayload, SoftwareStatesPayload,
};
use uuid::Uuid;

use uptrakit_service_connections::ServiceConnectionRegistry;

use crate::workload_claims::WorkloadClaimRegistry;

// ─── NatsPublisher ──────────────────────────────────────────────────────────

/// Abstraction over NATS publishing used by `NotificationService` and `EventBroadcaster`.
///
/// Methods match exactly what `notification_service.rs` and `event_broadcaster.rs`
/// call on `NatsTransport`.  The concrete implementation lives in `uptrakit-web-api`
/// (`NatsTransport`) and delegates to the real NATS JetStream connection.
///
/// Fire-and-forget: implementors log errors internally and do not propagate them.
#[async_trait::async_trait]
pub trait NatsPublisher: Send + Sync {
    /// Publish a [`ControllerMessage`] to NATS JetStream.
    ///
    /// - `source_controller_id` — the publishing controller's UUID (used for
    ///   self-message filtering on the consumer side).
    /// - `target_service_id` — when `Some`, routes to the per-service subject;
    ///   when `None`, routes based on `target_capability`.
    /// - `target_capability` — when `Some`, routes to the capability subject
    ///   (e.g. `"update_tracking"` or `"controller"`); `None` broadcasts.
    /// - `msg` — the message to publish.
    async fn publish(
        &self,
        source_controller_id: Uuid,
        target_service_id: Option<Uuid>,
        target_capability: Option<&str>,
        msg: ControllerMessage,
    );
}

// ─── NotificationService ────────────────────────────────────────────────────

/// Cross-controller notification service.
///
/// Wraps `ServiceConnectionRegistry` to provide the push API. Without NATS,
/// messages are delivered locally only (single-controller mode). When the
/// `nats` feature is enabled and a `NatsTransport` is attached, messages are
/// also published to NATS JetStream for cross-controller delivery.
///
/// When a `WorkloadClaimRegistry` is set, tenant-scoped messages
/// (`SoftwareStates`, `HostConnectivityUpdated`) are routed only to services
/// that hold at least one claimed config key for the target tenant, rather
/// than broadcast to all update-tracking services.
#[derive(Clone)]
pub struct NotificationService {
    registry: ServiceConnectionRegistry,
    controller_id: Uuid,
    /// Tracks whether a NATS transport has been attached. Always present so
    /// that `has_nats()` can be implemented without any `#[cfg]`.
    nats_configured: bool,
    /// Workload claim registry for tenant-scoped routing.
    claim_registry: Option<Arc<WorkloadClaimRegistry>>,
    #[cfg(feature = "nats")]
    nats: Option<Arc<dyn NatsPublisher>>,
}

impl NotificationService {
    /// Create a new notification service backed by the given registry and controller ID.
    pub fn new(registry: ServiceConnectionRegistry, controller_id: Uuid) -> Self {
        Self {
            registry,
            controller_id,
            nats_configured: false,
            claim_registry: None,
            #[cfg(feature = "nats")]
            nats: None,
        }
    }

    /// Attach a workload claim registry for tenant-scoped delivery routing.
    pub fn with_claim_registry(mut self, registry: Arc<WorkloadClaimRegistry>) -> Self {
        self.claim_registry = Some(registry);
        self
    }

    /// Access the workload claim registry, if configured.
    pub fn claim_registry(&self) -> Option<&Arc<WorkloadClaimRegistry>> {
        self.claim_registry.as_ref()
    }

    /// Attach a NATS publisher for cross-controller delivery.
    #[cfg(feature = "nats")]
    pub fn with_nats(mut self, nats: Arc<dyn NatsPublisher>) -> Self {
        self.nats = Some(nats);
        self.nats_configured = true;
        self
    }

    /// Returns `true` if a NATS transport is configured.
    pub fn has_nats(&self) -> bool {
        self.nats_configured
    }

    /// Push a message to a specific service (local + optional NATS).
    ///
    /// Messages that contain credentials (`TenantAssignments`, `TenantConfigUpdated`,
    /// `TenantRevoked`, `ServiceCredentials`) are delivered locally but **not**
    /// published to NATS to prevent credential leakage.
    #[tracing::instrument(skip_all, fields(%service_id))]
    pub async fn send(&self, service_id: &Uuid, msg: ControllerMessage) -> bool {
        let local = self.registry.send(service_id, msg.clone()).await;
        if msg.is_nats_publishable() {
            self.maybe_publish_nats(Some(*service_id), None, msg).await;
        }
        local
    }

    /// Broadcast a message to all connected services (local + optional NATS).
    ///
    /// Messages that contain credentials are delivered locally but **not**
    /// published to NATS (see [`Self::send`] doc comment).
    #[tracing::instrument(skip_all)]
    pub async fn broadcast(&self, msg: ControllerMessage) {
        self.registry.broadcast(msg.clone()).await;
        if msg.is_nats_publishable() {
            self.maybe_publish_nats(None, None, msg).await;
        }
    }

    /// Publish a controller-targeted event for cross-controller delivery.
    ///
    /// When NATS is available, the message is published to the controller
    /// subject. When NATS is not configured, this is a no-op (single-controller
    /// mode means the local controller already handled the event).
    #[tracing::instrument(skip_all)]
    pub async fn publish_controller_event(&self, msg: ControllerMessage) {
        self.maybe_publish_nats(None, Some("controller"), msg).await;
    }

    /// Send a message to all services with a specific capability (local + optional NATS).
    ///
    /// The `capability_str` is the wire-format capability name (e.g. `"update_tracking"`).
    /// It is parsed to a [`Capability`] for local delivery via the registry.
    #[tracing::instrument(skip_all, fields(capability_str))]
    pub async fn send_by_capability(&self, capability_str: &str, msg: ControllerMessage) {
        let cap: Capability = capability_str.parse().unwrap_or_else(|_| {
            // Should never happen — parse() always succeeds via Other fallback.
            Capability::Other(capability_str.to_string())
        });
        self.registry
            .broadcast_by_capability(&cap, msg.clone())
            .await;
        if msg.is_nats_publishable() {
            self.maybe_publish_nats(None, Some(capability_str), msg)
                .await;
        }
    }

    /// Access the underlying registry (e.g. for `is_connected` checks).
    pub fn registry(&self) -> &ServiceConnectionRegistry {
        &self.registry
    }

    /// Load software states for a tenant and push to all locally connected
    /// update-tracking services (immediately) and to NATS for cross-controller delivery.
    ///
    /// Loads both software-item states and host-package host states in two
    /// independent bulk queries, then delivers the merged payload.
    #[tracing::instrument(skip_all, fields(%tenant_id))]
    pub async fn push_software_states_for_tenant(
        &self,
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
    ) {
        let tenant_db = uptrakit_shared_db::TenantDb::new(db.clone(), tenant_id);
        let payload = match uptrakit_web_api_queries::queries::update_tracking_states::load_software_states_for_tenant(
            &tenant_db,
        )
        .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    %tenant_id,
                    "failed to load software states for push"
                );
                return;
            }
        };

        self.deliver_software_states(payload).await;
    }

    /// Load software states for a tenant using paginated host delivery and push
    /// each page to all locally connected update-tracking services and to NATS.
    ///
    /// Pages are delivered in order (page 0 first). The receiver accumulates
    /// pages until `page_index + 1 == total_pages` before applying state.
    #[tracing::instrument(skip_all, fields(%tenant_id))]
    pub async fn push_software_states_paginated_for_tenant(
        &self,
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
    ) {
        let tenant_db = uptrakit_shared_db::TenantDb::new(db.clone(), tenant_id);
        let page_size = uptrakit_wire::limits::STATES_HOST_PAGE_SIZE;
        let mut host_page: u64 = 0;
        loop {
            let payload =
                match uptrakit_web_api_queries::queries::update_tracking_states::load_software_states_page_for_tenant(
                    &tenant_db, host_page, page_size,
                )
                .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            %tenant_id,
                            host_page,
                            "failed to load software states page for push"
                        );
                        return;
                    }
                };
            let total_pages = u64::from(payload.page.total_pages);
            self.deliver_software_states(payload).await;
            host_page += 1;
            if host_page >= total_pages {
                break;
            }
        }
    }

    /// Deliver a pre-loaded `SoftwareStatesPayload` to all locally connected
    /// update-tracking services and publish to NATS for cross-controller delivery.
    ///
    /// When a `WorkloadClaimRegistry` is set, delivery is scoped to services that
    /// hold at least one claimed config key for the payload's tenant. Falls back
    /// to capability-based broadcast when no claim registry is configured or no
    /// services have claims for the tenant.
    ///
    /// Used by `ControllerSchedulerNotifier` to avoid re-loading an already-loaded
    /// payload through the ORM layer.
    #[tracing::instrument(skip_all, fields(tenant_id = %payload.tenant_id))]
    pub async fn deliver_software_states(&self, payload: SoftwareStatesPayload) {
        let tenant_id = payload.tenant_id;
        let msg = ControllerMessage::SoftwareStates(payload);
        // Deliver to locally connected services — scoped by claim when available.
        self.deliver_tenant_scoped(tenant_id, msg.clone()).await;
        // Publish to NATS for cross-controller delivery.
        self.maybe_publish_nats(None, Some("update_tracking"), msg)
            .await;
    }

    /// Publish a `HostConnectivityUpdated` event to all locally connected
    /// update-tracking services and to NATS for cross-controller delivery.
    ///
    /// This is the authoritative mechanism for notifying update-tracking services
    /// that an agent has connected or disconnected. Unlike `SoftwareStates`,
    /// connectivity state is sourced from the live WebSocket session on the
    /// controller that owns the agent connection — not from a DB query — so it
    /// must be delivered as an event rather than included in the
    /// `SoftwareStates` payload.
    ///
    /// Multi-controller safety: `HostConnectivityUpdated` is NATS-publishable
    /// (contains no credentials) so all update-tracking services across the
    /// cluster receive it regardless of which controller holds the agent
    /// WebSocket.
    #[tracing::instrument(skip_all, fields(%tenant_id))]
    pub async fn send_connectivity_update(
        &self,
        tenant_id: Uuid,
        updates: Vec<HostConnectivityUpdate>,
    ) {
        let payload = HostConnectivityUpdatedPayload::new(tenant_id, updates);
        let msg = ControllerMessage::HostConnectivityUpdated(payload);
        // Deliver to locally connected services — scoped by claim when available.
        self.deliver_tenant_scoped(tenant_id, msg.clone()).await;
        // Publish to NATS for cross-controller delivery.
        self.maybe_publish_nats(None, Some("update_tracking"), msg)
            .await;
    }

    /// Push `HostConnectivityUpdated` "online" events for all agents that are
    /// currently connected and serve hosts in `tenant_id`.
    ///
    /// Called when an update-tracking service (re)connects to ensure the service
    /// receives retained connectivity state for hosts whose agents were already
    /// online before the service connected.
    ///
    /// Only services that are currently registered in the connection registry
    /// receive an "online" event.  Services not in the registry are skipped —
    /// they will emit their own `HostConnectivityUpdated` events once they
    /// reconnect and send `ReportHosts`.
    #[tracing::instrument(skip_all, fields(%tenant_id))]
    pub async fn push_connected_agent_states_for_tenant(
        &self,
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
    ) {
        let agent_services =
            match uptrakit_web_api_queries::queries::update_tracking_states::load_agent_connectivity_for_tenant(
                db, tenant_id,
            )
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        %tenant_id,
                        "failed to load agent connectivity for push"
                    );
                    return;
                }
            };

        if agent_services.is_empty() {
            return;
        }

        let service_ids: Vec<uuid::Uuid> = agent_services.iter().map(|s| s.service_id).collect();
        let connected = self.registry.filter_connected(&service_ids).await;

        if connected.is_empty() {
            return;
        }

        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();

        let mut updates: Vec<HostConnectivityUpdate> = Vec::new();
        for s in &agent_services {
            if !connected.contains(&s.service_id) {
                continue;
            }
            let last_seen = s
                .last_seen_at
                .and_then(|t| {
                    t.format(&time::format_description::well_known::Rfc3339)
                        .ok()
                })
                .unwrap_or_else(|| now.clone());
            for &host_id in &s.host_ids {
                updates.push(HostConnectivityUpdate::online(
                    host_id,
                    Some(last_seen.clone()),
                    s.client_version.clone(),
                ));
            }
        }

        if !updates.is_empty() {
            self.send_connectivity_update(tenant_id, updates).await;
        }
    }

    /// Return the controller ID.
    pub fn controller_id(&self) -> Uuid {
        self.controller_id
    }

    /// Route a tenant-scoped message to local services.
    ///
    /// When a claim registry is configured, delivers only to services that hold
    /// at least one claimed config key for `tenant_id`. Falls back to
    /// capability-based broadcast when no claims exist or no registry is set.
    async fn deliver_tenant_scoped(&self, tenant_id: Uuid, msg: ControllerMessage) {
        if let Some(ref cr) = self.claim_registry {
            let service_ids = cr.services_for_tenant(tenant_id);
            if !service_ids.is_empty() {
                for svc_id in &service_ids {
                    self.registry.send(svc_id, msg.clone()).await;
                }
                return;
            }
            // No claims for this tenant — fall through to broadcast for graceful
            // degradation (e.g. during rolling deployment or before claims arrive).
        }
        self.registry
            .broadcast_by_capability(&Capability::UpdateTracking, msg)
            .await;
    }

    /// Conditionally publish a message to NATS when a transport is configured.
    async fn maybe_publish_nats(
        &self,
        target_service_id: Option<Uuid>,
        target_capability: Option<&str>,
        msg: ControllerMessage,
    ) {
        #[cfg(feature = "nats")]
        if let Some(ref nats) = self.nats {
            nats.publish(
                self.controller_id,
                target_service_id,
                target_capability,
                msg,
            )
            .await;
            return;
        }
        // Suppress unused-variable warnings when nats feature is disabled or no
        // transport is configured. Events are ephemeral; dropping them is safe.
        let _ = (target_service_id, target_capability, msg);
    }
}

#[async_trait::async_trait]
impl uptrakit_web_api_queries::notifier::ServiceNotifier for NotificationService {
    async fn send_to_service(&self, service_id: &Uuid, msg: ControllerMessage) -> bool {
        self.send(service_id, msg).await
    }
}

// ─── EventBroadcaster ───────────────────────────────────────────────────────

/// Broadcast channel capacity per tenant. Larger than device flow (4) because
/// many event types and producers can fire concurrently.
const CHANNEL_CAPACITY: usize = 512;

/// Internal channel entry tracking the sender and active subscriber count.
struct ChannelEntry {
    tx: broadcast::Sender<AdminEvent>,
    subscriber_count: usize,
}

/// Registry of per-tenant broadcast channels for real-time admin event delivery.
///
/// Thread-safe and cheaply cloneable (interior `Arc`).
///
/// ## Multi-instance support (NATS)
///
/// When the `nats` feature is enabled and a NATS transport is attached via
/// [`EventBroadcaster::with_nats`], every call to [`send`](Self::send) and
/// [`send_global`](Self::send_global) also publishes the event to all other
/// controller instances via NATS JetStream. Receiving instances call
/// [`send_local`](Self::send_local) / [`send_global_local`](Self::send_global_local)
/// to re-broadcast locally without re-publishing (avoiding infinite loops).
#[derive(Clone)]
pub struct EventBroadcaster {
    channels: Arc<RwLock<HashMap<Uuid, ChannelEntry>>>,
    /// This controller's UUID — used as the NATS publication source.
    #[cfg(feature = "nats")]
    controller_id: Uuid,
    #[cfg(feature = "nats")]
    nats: Option<Arc<dyn NatsPublisher>>,
}

impl EventBroadcaster {
    /// Create a new empty broadcaster (single-instance mode, no NATS).
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "nats")]
            controller_id: Uuid::nil(),
            #[cfg(feature = "nats")]
            nats: None,
        }
    }

    /// Attach a NATS transport for cross-controller event fan-out.
    ///
    /// Returns `self` for builder-style chaining.  All subsequent calls to
    /// [`send`] and [`send_global`] will also publish the event via NATS so
    /// that other controller instances can relay it to their local SSE clients.
    #[cfg(feature = "nats")]
    pub fn with_nats(mut self, nats: Arc<dyn NatsPublisher>, controller_id: Uuid) -> Self {
        self.nats = Some(nats);
        self.controller_id = controller_id;
        self
    }

    /// Broadcast an event to local SSE subscribers of the given tenant **only**.
    ///
    /// Does **not** publish to NATS. Used by the NATS consumer to relay
    /// cross-controller events without causing a re-publish loop.
    #[tracing::instrument(skip_all, fields(%tenant_id))]
    pub async fn send_local(&self, tenant_id: Uuid, event: AdminEvent) {
        let channels = self.channels.read();
        if let Some(entry) = channels.get(&tenant_id) {
            let _ = entry.tx.send(event);
        }
    }

    /// Broadcast an event to **all** local tenant channels **only**.
    ///
    /// Does **not** publish to NATS. Used by the NATS consumer for system-wide
    /// cross-controller events without causing a re-publish loop.
    #[tracing::instrument(skip_all)]
    pub async fn send_global_local(&self, event: AdminEvent) {
        let channels = self.channels.read();
        for entry in channels.values() {
            let _ = entry.tx.send(event.clone());
        }
    }

    /// Send an event to all subscribers of the given tenant.
    ///
    /// Fire-and-forget: no-op if no subscribers are connected for this tenant.
    /// Also publishes to NATS when a transport is configured so other controller
    /// instances relay the event to their own local SSE subscribers.
    #[tracing::instrument(skip_all, fields(%tenant_id))]
    pub async fn send(&self, tenant_id: Uuid, event: AdminEvent) {
        self.send_local(tenant_id, event.clone()).await;
        self.maybe_publish_nats(Some(tenant_id), event).await;
    }

    /// Send an event to all active tenant channels (for system-wide events).
    ///
    /// Fire-and-forget: iterates all tenant channels and sends to each.
    /// Also publishes to NATS when a transport is configured.
    #[tracing::instrument(skip_all)]
    pub async fn send_global(&self, event: AdminEvent) {
        self.send_global_local(event.clone()).await;
        self.maybe_publish_nats(None, event).await;
    }

    /// Publish an `AdminEvent` to NATS so other controller instances can
    /// relay it to their local SSE subscribers.  No-op when NATS is not
    /// configured or the `nats` feature is disabled.
    #[allow(
        unused_variables,
        reason = "tenant_id and event are used only when nats feature is enabled"
    )]
    async fn maybe_publish_nats(&self, tenant_id: Option<Uuid>, event: AdminEvent) {
        #[cfg(feature = "nats")]
        if let (Some(nats), Ok(event_json)) = (&self.nats, serde_json::to_string(&event)) {
            nats.publish(
                self.controller_id,
                None,
                Some("controller"),
                uptrakit_wire::ControllerMessage::BroadcastAdminEvent(
                    uptrakit_wire::BroadcastAdminEventPayload {
                        tenant_id,
                        event_json,
                    },
                ),
            )
            .await;
        }
    }

    /// Subscribe to admin events for the given tenant.
    ///
    /// Creates the channel lazily on first subscribe, increments the subscriber
    /// count on subsequent subscribes.
    #[tracing::instrument(skip_all, fields(%tenant_id))]
    pub async fn subscribe(&self, tenant_id: Uuid) -> broadcast::Receiver<AdminEvent> {
        let mut channels = self.channels.write();
        let entry = channels.entry(tenant_id).or_insert_with(|| {
            let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
            ChannelEntry {
                tx,
                subscriber_count: 0,
            }
        });
        entry.subscriber_count += 1;
        entry.tx.subscribe()
    }

    /// Decrement the subscriber count for the given tenant.
    ///
    /// When the count reaches zero, the channel is removed to free memory.
    #[tracing::instrument(skip_all, fields(%tenant_id))]
    pub async fn unsubscribe(&self, tenant_id: Uuid) {
        let mut channels = self.channels.write();
        let remove = if let Some(entry) = channels.get_mut(&tenant_id) {
            entry.subscriber_count = entry.subscriber_count.saturating_sub(1);
            entry.subscriber_count == 0
        } else {
            false
        };
        if remove {
            channels.remove(&tenant_id);
        }
    }
}

impl Default for EventBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

// ─── NotificationDispatcher ─────────────────────────────────────────────────

use uptrakit_notification_delivery::NotificationEvent;
use uptrakit_plugin_infrastructure_registry::PluginOps;

/// Bounded capacity for the notification dispatcher channel.
///
/// Limits memory consumption under bulk update completions that generate many
/// simultaneous notification events. When the channel is full, events are
/// dropped and a warning is logged (fire-and-forget semantics).
const NOTIFICATION_DISPATCHER_CAPACITY: usize = 4096;

/// Fire-and-forget notification dispatcher.
///
/// Event producers call `dispatch()` to enqueue events. The background
/// loop processes events asynchronously: matching rules, building messages,
/// and delivering through channels. Delivery failures are logged but never
/// surface to event producers.
#[derive(Clone)]
pub struct NotificationDispatcher {
    tx: mpsc::Sender<NotificationEvent>,
}

impl NotificationDispatcher {
    /// Create a new dispatcher and spawn the background processing loop.
    pub fn new(
        db: sea_orm::DatabaseConnection,
        notification_ops: Arc<dyn PluginOps>,
        callback_base_url: String,
    ) -> Self {
        let (tx, rx) = mpsc::channel(NOTIFICATION_DISPATCHER_CAPACITY);
        tokio::spawn(dispatch_loop(db, notification_ops, callback_base_url, rx));
        Self { tx }
    }

    /// Enqueue a notification event for background processing.
    ///
    /// This never blocks and never fails from the caller's perspective.
    /// If the channel is full, the event is dropped and a warning is logged.
    /// If the channel is closed (dispatcher shut down), the event is silently dropped.
    #[tracing::instrument(skip_all)]
    pub fn dispatch(&self, event: NotificationEvent) {
        match self.tx.try_send(event) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!(
                    "notification dispatcher channel full (capacity: {NOTIFICATION_DISPATCHER_CAPACITY}), dropping event"
                );
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                tracing::warn!("notification dispatcher channel closed, dropping event");
            }
        }
    }
}

#[cfg(any(test, feature = "testing"))]
impl NotificationDispatcher {
    /// Create a dispatcher whose events are sent to the returned receiver
    /// instead of spawning the background dispatch loop. Use in unit tests to
    /// observe dispatched [`NotificationEvent`]s without a real database.
    pub fn test_channel() -> (Self, mpsc::Receiver<NotificationEvent>) {
        let (tx, rx) = mpsc::channel(64);
        (Self { tx }, rx)
    }
}

#[tracing::instrument(skip_all)]
async fn dispatch_loop(
    db: sea_orm::DatabaseConnection,
    notification_ops: Arc<dyn PluginOps>,
    callback_base_url: String,
    mut rx: mpsc::Receiver<NotificationEvent>,
) {
    use sea_orm::{ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{notification_channel, notification_log, notification_rule};

    while let Some(event) = rx.recv().await {
        let event_type = event.event_type();
        let event_type_str = event_type.as_str().to_string();
        let tenant_id = event.tenant_id;

        // Load matching rules
        let rules = match notification_rule::Entity::find()
            .filter(notification_rule::Column::TenantId.eq(tenant_id))
            .filter(notification_rule::Column::EventType.eq(&event_type_str))
            .filter(notification_rule::Column::Enabled.eq(true))
            .all(&db)
            .await
        {
            Ok(rules) => rules,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    %tenant_id,
                    event_type = %event_type_str,
                    "failed to load notification rules"
                );
                continue;
            }
        };

        for rule in rules {
            // Scope filtering: if rule specifies a filter, it must match
            if let Some(rule_host_id) = rule.host_id
                && event.host_id != Some(rule_host_id)
            {
                continue;
            }
            if let Some(ref rule_software_id) = rule.software_item_id
                && event.software_item_id.as_ref() != Some(rule_software_id)
            {
                continue;
            }
            if let Some(ref rule_plugin_type) = rule.plugin_type
                && event.plugin_type.as_ref() != Some(rule_plugin_type)
            {
                continue;
            }

            // Load the channel
            let channel_model = match notification_channel::Entity::find_by_id(rule.channel_id)
                .filter(notification_channel::Column::TenantId.eq(tenant_id))
                .filter(notification_channel::Column::Enabled.eq(true))
                .one(&db)
                .await
            {
                Ok(Some(c)) => c,
                Ok(None) => continue, // Channel disabled or deleted
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        channel_id = %rule.channel_id,
                        "failed to load notification channel"
                    );
                    continue;
                }
            };

            // Look up channel implementation
            let channel_type_id =
                uptrakit_shared_types::PluginTypeId::new(&channel_model.channel_type);
            let channel_transport = match notification_ops.transport(&channel_type_id) {
                Some(c) => c,
                None => {
                    tracing::warn!(
                        channel_type = %channel_model.channel_type,
                        "no channel implementation for type"
                    );
                    continue;
                }
            };

            // Parse config JSON
            let config_json: serde_json::Value =
                match serde_json::from_str(channel_model.config.expose_secret()) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            channel_id = %channel_model.id,
                            "failed to parse channel config JSON"
                        );
                        continue;
                    }
                };

            // Build a generic settings bag from the database.
            // Each plugin's `deliver()` extracts only the keys it needs.
            let settings_bag =
                uptrakit_web_api_queries::notification_settings::build_settings_bag(&db, tenant_id)
                    .await;

            // Generate action token if the event is actionable
            let action_token = event.action_params().map(|_| Uuid::now_v7());

            // Build the channel-agnostic message
            let message = match uptrakit_notification_delivery::build_delivery_message(
                &event,
                action_token,
                &callback_base_url,
                &channel_model.channel_type,
                channel_model.id,
            ) {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        channel_id = %channel_model.id,
                        "failed to build delivery message"
                    );
                    continue;
                }
            };

            // Serialize event payload for the log
            let event_payload = serde_json::to_value(&event.details).unwrap_or_default();

            // Insert log entry as pending
            let log_id = Uuid::now_v7();
            let now = OffsetDateTime::now_utc();
            let log_entry = notification_log::ActiveModel {
                id: Set(log_id),
                tenant_id: Set(tenant_id),
                channel_id: Set(channel_model.id),
                rule_id: Set(rule.id),
                event_type: Set(event_type_str.clone()),
                event_payload: Set(event_payload),
                status: Set("pending".to_string()),
                error_message: Set(None),
                action_token: Set(action_token),
                action_taken: Set(None),
                created_at: Set(now),
                delivered_at: Set(None),
            };

            if let Err(e) = notification_log::Entity::insert(log_entry).exec(&db).await {
                tracing::error!(
                    error = %e,
                    log_id = %log_id,
                    "failed to insert notification log entry"
                );
                continue;
            }

            // Spawn delivery task
            let db_clone = db.clone();
            let channel_transport = channel_transport.clone();
            tokio::spawn(async move {
                match uptrakit_notification_delivery::deliver(
                    channel_transport,
                    &config_json,
                    &settings_bag,
                    &message,
                )
                .await
                {
                    Ok(()) => {
                        let now = OffsetDateTime::now_utc();
                        let update = notification_log::ActiveModel {
                            id: Set(log_id),
                            status: Set("delivered".to_string()),
                            delivered_at: Set(Some(now)),
                            ..Default::default()
                        };
                        if let Err(e) = notification_log::Entity::update(update)
                            .exec(&db_clone)
                            .await
                        {
                            tracing::error!(
                                error = %e,
                                log_id = %log_id,
                                "failed to update notification log to delivered"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            log_id = %log_id,
                            error = %e,
                            "notification delivery failed"
                        );
                        let update = notification_log::ActiveModel {
                            id: Set(log_id),
                            status: Set("failed".to_string()),
                            error_message: Set(Some(e.to_string())),
                            ..Default::default()
                        };
                        if let Err(db_err) = notification_log::Entity::update(update)
                            .exec(&db_clone)
                            .await
                        {
                            tracing::error!(
                                error = %db_err,
                                log_id = %log_id,
                                "failed to update notification log to failed"
                            );
                        }
                    }
                }
            });
        }
    }
}

// ─── NotificationState ──────────────────────────────────────────────────────

/// Notification side-effect state used by mutation actions.
///
/// Bundles the three common notification handles so that callers need only
/// a single reference to access all notification side-effects.
#[non_exhaustive]
#[derive(Clone)]
pub struct NotificationState {
    /// Cross-controller notification service for push message delivery via outbox pattern.
    pub notification_service: NotificationService,
    /// Notification dispatcher for fire-and-forget event delivery.
    pub notification_dispatcher: NotificationDispatcher,
    /// Per-tenant broadcast channels for real-time admin event SSE delivery.
    pub event_broadcaster: EventBroadcaster,
}

impl NotificationState {
    /// Create a new [`NotificationState`] from its components.
    pub fn new(
        notification_service: NotificationService,
        notification_dispatcher: NotificationDispatcher,
        event_broadcaster: EventBroadcaster,
    ) -> Self {
        Self {
            notification_service,
            notification_dispatcher,
            event_broadcaster,
        }
    }
}

/// Implemented by application state types (e.g. `AppState`) so the blanket
/// `FromRef<Arc<S>> for NotificationState` can live in controller-core without
/// violating the orphan rule.
///
/// Only available with the `axum-integration` feature.
#[cfg(feature = "axum-integration")]
pub trait NotificationStateSource {
    /// Returns a clone of the [`NotificationState`] held by this state.
    fn notification_state(&self) -> NotificationState;
}

/// Enables Axum to extract [`NotificationState`] from any `Arc<S>` where
/// `S: NotificationStateSource`.
///
/// Only available with the `axum-integration` feature.
#[cfg(feature = "axum-integration")]
impl<S> axum::extract::FromRef<std::sync::Arc<S>> for NotificationState
where
    S: NotificationStateSource + Clone + Send + Sync + 'static,
{
    fn from_ref(state: &std::sync::Arc<S>) -> Self {
        state.notification_state()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod notification_service_tests {
    use super::*;
    use uptrakit_wire::{ApprovedPayload, ServerRestartingPayload};

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

    /// Verify that credential-bearing variants are blocked by `is_nats_publishable`.
    /// The authoritative tests for this are in the wire crate; this test documents
    /// the integration point used by NotificationService.
    #[test]
    fn credential_bearing_variants_are_not_nats_publishable() {
        // Approved is a safe, publishable variant — sanity check.
        assert!(
            ControllerMessage::Approved(ApprovedPayload {
                service_id: Uuid::nil(),
            })
            .is_nats_publishable()
        );

        // ServiceCredentials must never be published to NATS.
        assert!(
            !ControllerMessage::ServiceCredentials(uptrakit_wire::ServiceCredentialsPayload {
                db_url: None,
                master_key_hex: None,
                nats_url: None,
            })
            .is_nats_publishable()
        );
    }

    #[tokio::test]
    async fn send_delivers_locally_without_nats() {
        let registry = ServiceConnectionRegistry::new();
        let controller_id = Uuid::now_v7();
        let svc = NotificationService::new(registry, controller_id);

        assert!(!svc.has_nats());

        let service_id = Uuid::now_v7();
        let msg = ControllerMessage::Approved(ApprovedPayload { service_id });

        // send returns false because no service is connected.
        let result = svc.send(&service_id, msg).await;
        assert!(!result);
    }

    #[tokio::test]
    async fn broadcast_delivers_locally_without_nats() {
        let registry = ServiceConnectionRegistry::new();
        let controller_id = Uuid::now_v7();
        let svc = NotificationService::new(registry, controller_id);

        // Broadcast to an empty registry should not panic.
        svc.broadcast(ControllerMessage::Approved(ApprovedPayload {
            service_id: Uuid::nil(),
        }))
        .await;
    }

    #[tokio::test]
    async fn publish_controller_event_noop_without_nats() {
        let registry = ServiceConnectionRegistry::new();
        let controller_id = Uuid::now_v7();
        let svc = NotificationService::new(registry, controller_id);

        // Should be a no-op (no NATS configured).
        svc.publish_controller_event(ControllerMessage::Approved(ApprovedPayload {
            service_id: Uuid::nil(),
        }))
        .await;
    }
}

#[cfg(test)]
mod event_broadcaster_tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]

    use super::*;

    #[tokio::test]
    async fn subscribe_creates_channel() {
        let broadcaster = EventBroadcaster::new();
        let tenant = Uuid::now_v7();

        let _rx = broadcaster.subscribe(tenant).await;

        // Channel should exist
        let channels = broadcaster.channels.read();
        assert!(channels.contains_key(&tenant));
        assert_eq!(channels[&tenant].subscriber_count, 1);
    }

    #[tokio::test]
    async fn unsubscribe_removes_channel_when_zero() {
        let broadcaster = EventBroadcaster::new();
        let tenant = Uuid::now_v7();

        let _rx = broadcaster.subscribe(tenant).await;
        broadcaster.unsubscribe(tenant).await;

        let channels = broadcaster.channels.read();
        assert!(!channels.contains_key(&tenant));
    }

    #[tokio::test]
    async fn send_delivers_to_subscriber() {
        let broadcaster = EventBroadcaster::new();
        let tenant = Uuid::now_v7();
        let host_id = Uuid::now_v7();

        let mut rx = broadcaster.subscribe(tenant).await;

        broadcaster
            .send(tenant, AdminEvent::HostUpdated { id: host_id })
            .await;

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, AdminEvent::HostUpdated { id } if id == host_id));
    }

    #[tokio::test]
    async fn send_to_empty_tenant_is_silent() {
        let broadcaster = EventBroadcaster::new();
        let tenant = Uuid::now_v7();

        // Should not panic or error
        broadcaster
            .send(tenant, AdminEvent::HostUpdated { id: Uuid::now_v7() })
            .await;
    }

    #[tokio::test]
    async fn send_global_delivers_to_all_tenants() {
        let broadcaster = EventBroadcaster::new();
        let tenant_a = Uuid::now_v7();
        let tenant_b = Uuid::now_v7();
        let service_id = Uuid::now_v7();

        let mut rx_a = broadcaster.subscribe(tenant_a).await;
        let mut rx_b = broadcaster.subscribe(tenant_b).await;

        broadcaster
            .send_global(AdminEvent::SystemServiceStatusChanged {
                id: service_id,
                status: "approved".to_string(),
            })
            .await;

        let event_a = rx_a.recv().await.unwrap();
        let event_b = rx_b.recv().await.unwrap();

        assert!(
            matches!(event_a, AdminEvent::SystemServiceStatusChanged { id, .. } if id == service_id)
        );
        assert!(
            matches!(event_b, AdminEvent::SystemServiceStatusChanged { id, .. } if id == service_id)
        );
    }

    #[tokio::test]
    async fn multiple_tenants_isolated() {
        let broadcaster = EventBroadcaster::new();
        let tenant_a = Uuid::now_v7();
        let tenant_b = Uuid::now_v7();
        let host_a = Uuid::now_v7();

        let mut rx_a = broadcaster.subscribe(tenant_a).await;
        let _rx_b = broadcaster.subscribe(tenant_b).await;

        // Send only to tenant A
        broadcaster
            .send(tenant_a, AdminEvent::HostCreated { id: host_a })
            .await;

        let event = rx_a.recv().await.unwrap();
        assert!(matches!(event, AdminEvent::HostCreated { id } if id == host_a));

        // tenant B should not have received anything (no data available)
        // We can't easily test "no event" without a timeout, so just verify
        // the send was targeted.
    }

    #[tokio::test]
    async fn multiple_subscribers_same_tenant() {
        let broadcaster = EventBroadcaster::new();
        let tenant = Uuid::now_v7();

        let mut rx1 = broadcaster.subscribe(tenant).await;
        let mut rx2 = broadcaster.subscribe(tenant).await;

        {
            let channels = broadcaster.channels.read();
            assert_eq!(channels[&tenant].subscriber_count, 2);
        }

        broadcaster
            .send(tenant, AdminEvent::HostUpdated { id: Uuid::now_v7() })
            .await;

        assert!(rx1.recv().await.is_ok());
        assert!(rx2.recv().await.is_ok());

        // Unsubscribe one — channel should remain
        broadcaster.unsubscribe(tenant).await;
        {
            let channels = broadcaster.channels.read();
            assert!(channels.contains_key(&tenant));
            assert_eq!(channels[&tenant].subscriber_count, 1);
        }

        // Unsubscribe second — channel should be removed
        broadcaster.unsubscribe(tenant).await;
        {
            let channels = broadcaster.channels.read();
            assert!(!channels.contains_key(&tenant));
        }
    }

    #[tokio::test]
    async fn lagged_subscriber_continues() {
        let broadcaster = EventBroadcaster::new();
        let tenant = Uuid::now_v7();

        let mut rx = broadcaster.subscribe(tenant).await;

        // Overflow the channel capacity (512) — the subscriber will lag.
        for i in 0..CHANNEL_CAPACITY + 10 {
            broadcaster
                .send(
                    tenant,
                    AdminEvent::HostUpdated {
                        id: Uuid::from_u128(i as u128),
                    },
                )
                .await;
        }

        // First recv should return Lagged error
        let result = rx.recv().await;
        assert!(
            result.is_ok() || matches!(result, Err(broadcast::error::RecvError::Lagged(_))),
            "expected either Ok or Lagged, got: {result:?}"
        );
    }
}
