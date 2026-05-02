//! Unified embedded service infrastructure.
//!
//! The [`EmbeddedServiceHost`] manages the lifecycle of services that run
//! inside the controller process. It handles:
//!
//! - Auto-provisioning service records in the database
//! - Registering embedded services in the `ServiceConnectionRegistry`
//! - Coexistence with external services (yield/resume via capability matching)
//! - In-process transport via mpsc channels
//!
//! Individual service embeddings call [`EmbeddedServiceHost::add()`] with
//! decomposed parameters — no shared trait in any shared crate is needed.

pub(crate) mod bridge;
pub(crate) mod metadata_runtime;
pub(crate) mod provision;
#[allow(dead_code)] // Infrastructure types used by follow-up service embeddings.
pub(crate) mod types;

use std::collections::{BTreeSet, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use sea_orm::sea_query::OnConflict;
use sea_orm::{ActiveValue, EntityTrait};
use tokio_util::sync::CancellationToken;
use uptrakit_shared_db::entity::embedded_service_runtime_state;
use uptrakit_web_api::embedded_support::EmbeddedServiceNotifier;
use uptrakit_wire::Capability;
use uuid::Uuid;

use uptrakit_web_api::service_connections::ServiceConnectionRegistry;

use crate::tasks::BackgroundTasks;
use types::{CoexistencePolicy, EmbeddedTransport, ExternalServiceInfo};

/// Tokens passed to each embedded service's run closure to control its lifecycle.
pub(crate) struct EmbeddedShutdownTokens {
    /// Cancel to stop claiming new work. In-flight work completes naturally.
    #[cfg_attr(
        not(any(feature = "embedded-scheduler", feature = "embedded-ssh-agent")),
        allow(dead_code)
    )]
    pub drain: CancellationToken,
    /// Cancel to abort in-flight work immediately.
    pub abort: CancellationToken,
}

/// Result of registering an embedded service via [`EmbeddedServiceHost::add()`].
#[allow(dead_code)] // Fields used by follow-up service embeddings (agent).
pub(crate) struct AddResult {
    /// The provisioned service ID.
    pub service_id: Uuid,
    /// Receiver for `ServiceMessage` sent by the embedded service.
    /// System services (scheduler) can ignore this; tenant services (agent)
    /// must feed it into a message processor bridge.
    pub service_rx: tokio::sync::mpsc::Receiver<uptrakit_wire::ServiceMessage>,
}

// ---------------------------------------------------------------------------
// EmbeddedServiceHandle
// ---------------------------------------------------------------------------

/// Internal handle for a single embedded service tracked by the host.
struct EmbeddedServiceHandle {
    service_id: Uuid,
    label: &'static str,
    /// The service's own `service_app_name`, used by `YieldOnSameAppName` to
    /// compare against the connecting external service's name.
    app_name: String,
    yielded: Arc<AtomicBool>,
    /// Set of external service IDs that are currently causing this embedded
    /// service to yield. The `yielded` AtomicBool is set when the first ID is
    /// inserted and cleared when the last ID is removed, preventing false
    /// resumes when multiple yielder services are connected simultaneously.
    yielding_service_ids: Arc<parking_lot::Mutex<HashSet<Uuid>>>,
    yield_state_changed: Arc<tokio::sync::Notify>,
    capabilities: BTreeSet<Capability>,
    coexistence_policy: CoexistencePolicy,
}

// ---------------------------------------------------------------------------
// EmbeddedServiceHost
// ---------------------------------------------------------------------------

/// Orchestrator for embedded services running inside the controller process.
pub(crate) struct EmbeddedServiceHost {
    services: parking_lot::Mutex<Vec<EmbeddedServiceHandle>>,
    /// Cloned handle to the connection registry, used to look up
    /// `service_app_name` in `on_external_connected` without threading
    /// the value through the trait method signature.
    registry: std::sync::OnceLock<ServiceConnectionRegistry>,
}

impl EmbeddedServiceHost {
    pub(crate) fn new() -> Self {
        Self {
            services: parking_lot::Mutex::new(Vec::new()),
            registry: std::sync::OnceLock::new(),
        }
    }

    /// Register and start an embedded service.
    ///
    /// 1. Auto-provision a service record in the DB
    /// 2. Register in `ServiceConnectionRegistry`
    /// 3. Create bidirectional channels
    /// 4. Spawn response forwarder
    /// 5. Spawn the service's run closure with `EmbeddedTransport`
    /// 6. Track handles in `BackgroundTasks`
    ///
    /// `tenant_id` is required when `!is_system_service` — it determines which
    /// tenant the embedded service record belongs to.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn add(
        &self,
        label: &'static str,
        app_name: &str,
        capabilities: BTreeSet<Capability>,
        is_system_service: bool,
        tenant_id: Option<Uuid>,
        embedded_owner_key: Uuid,
        coexistence_policy: CoexistencePolicy,
        run_fn: impl FnOnce(
            EmbeddedTransport,
            EmbeddedShutdownTokens,
        ) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + 'static,
        state: &Arc<uptrakit_web_api::AppState>,
        bg: &mut BackgroundTasks,
    ) -> rootcause::Result<AddResult> {
        // 1. Auto-provision.
        let hostname = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "embedded".to_string());

        let service_id = if is_system_service {
            provision::provision_embedded_system_service(
                state.db(),
                app_name,
                label,
                &capabilities,
                &hostname,
                embedded_owner_key,
            )
            .await?
        } else {
            #[expect(
                clippy::expect_used,
                reason = "guaranteed by branch: the surrounding `if !is_system` branch is taken precisely when `tenant_id` is `Some`"
            )]
            let tid = tenant_id.expect("tenant_id is required for non-system embedded services");
            provision::provision_embedded_tenant_service(
                state.db(),
                tid,
                app_name,
                label,
                &capabilities,
                &hostname,
                embedded_owner_key,
            )
            .await?
        };

        // 2. Register in ServiceConnectionRegistry.
        // Store the registry reference on first call so on_external_connected
        // can look up service_app_name without a signature change to the trait.
        self.registry
            .get_or_init(|| state.service_connections.clone());

        let (push_rx, _cancel_token) = state
            .service_connections
            .register(
                service_id,
                capabilities.clone(),
                None,
                None,
                Some(app_name.to_string()),
            )
            .await;

        // 3. Create bidirectional channels.
        //
        // Service → Controller (ServiceMessage):
        //   service_tx is given to the EmbeddedTransport
        //   service_rx is returned to the caller when a message bridge is needed
        //
        // Controller → Service (ControllerMessage):
        //   ctrl_tx is used by the response forwarder
        //   ctrl_rx is given to the EmbeddedTransport
        let (service_tx, service_rx) =
            tokio::sync::mpsc::channel::<uptrakit_wire::ServiceMessage>(32);
        let (ctrl_tx, ctrl_rx) = tokio::sync::mpsc::channel::<uptrakit_wire::ControllerMessage>(32);

        let yielded = Arc::new(AtomicBool::new(false));
        let yielding_service_ids = Arc::new(parking_lot::Mutex::new(HashSet::new()));
        let yield_state_changed = Arc::new(tokio::sync::Notify::new());

        // 4. Spawn response forwarder (push_rx → ctrl_tx).
        let forwarder_cancel = bg.child_token();
        let forwarder_handle = tokio::spawn(bridge::run_response_forwarder(
            push_rx,
            ctrl_tx,
            forwarder_cancel,
        ));
        // `track()` requires `&'static str`; we leak the formatted label since
        // there are at most a handful of embedded services per process.
        let forwarder_label: &'static str =
            Box::leak(format!("{label} (forwarder)").into_boxed_str());
        bg.track(forwarder_label, forwarder_handle);

        // 5. Create transport and spawn the service closure.
        //    - drain_token: independent, cancelled in Phase 2.5 before service drain.
        //    - abort_token: child of shutdown_token, cancelled in Phase 3 (hard stop).
        let transport = EmbeddedTransport::new(
            service_tx,
            ctrl_rx,
            Arc::clone(&yielded),
            Arc::clone(&yield_state_changed),
        );
        let drain_token = CancellationToken::new();
        let abort_token = bg.child_token();
        let tokens = EmbeddedShutdownTokens {
            drain: drain_token.clone(),
            abort: abort_token,
        };
        let service_handle = tokio::spawn(run_fn(transport, tokens));
        bg.mark_embedded(service_id, drain_token);
        bg.track(label, service_handle);

        let runtime_state_handle = tokio::spawn(run_yield_state_sync(
            state.db().clone(),
            service_id,
            Arc::clone(&yielding_service_ids),
            Arc::clone(&yield_state_changed),
            bg.child_token(),
        ));
        let runtime_state_label: &'static str =
            Box::leak(format!("{label} (yield-state)").into_boxed_str());
        bg.track(runtime_state_label, runtime_state_handle);

        // 6. Track the handle.
        {
            let mut services = self.services.lock();
            services.push(EmbeddedServiceHandle {
                service_id,
                label,
                app_name: app_name.to_string(),
                yielded,
                yielding_service_ids,
                yield_state_changed,
                capabilities,
                coexistence_policy,
            });
        }

        tracing::info!(
            %service_id,
            label,
            app_name,
            "embedded service registered"
        );

        Ok(AddResult {
            service_id,
            service_rx,
        })
    }

    /// Evaluate whether a specific embedded service should yield based on
    /// its coexistence policy and the external service info.
    fn should_yield(handle: &EmbeddedServiceHandle, info: &ExternalServiceInfo) -> bool {
        match &handle.coexistence_policy {
            CoexistencePolicy::YieldOnSameAppName => {
                info.service_app_name.as_deref() == Some(handle.app_name.as_str())
            }
            CoexistencePolicy::Custom(check) => check(info),
            CoexistencePolicy::NeverYield => false,
        }
    }

    fn apply_external_service_update(
        handle: &EmbeddedServiceHandle,
        info: &ExternalServiceInfo,
        registry: Option<&ServiceConnectionRegistry>,
    ) {
        let mut ids = handle.yielding_service_ids.lock();
        let was_yielded = handle.yielded.load(Ordering::Relaxed);

        if Self::should_yield(handle, info) {
            ids.insert(info.service_id);
        } else {
            ids.remove(&info.service_id);
        }

        let is_yielded = !ids.is_empty();
        if is_yielded != was_yielded {
            handle.yielded.store(is_yielded, Ordering::Release);
            if is_yielded {
                tracing::info!(
                    embedded_service_id = %handle.service_id,
                    embedded_label = handle.label,
                    external_service_id = %info.service_id,
                    "embedded service yielding to external"
                );
            } else {
                tracing::info!(
                    embedded_service_id = %handle.service_id,
                    embedded_label = handle.label,
                    external_service_id = %info.service_id,
                    "embedded service resuming (no more active yielders)"
                );
            }
        }

        // Keep the connection registry's availability view aligned with the
        // embedded runtime's local yield state so surface routing can fail fast.
        if let Some(registry) = registry {
            registry.set_yielded(&handle.service_id, is_yielded);
        }

        handle.yield_state_changed.notify_one();
    }
}

impl EmbeddedServiceNotifier for EmbeddedServiceHost {
    fn on_external_connected(
        &self,
        service_id: Uuid,
        capabilities: &BTreeSet<Capability>,
        hostname: Option<&str>,
        is_system: bool,
    ) {
        let service_app_name = self
            .registry
            .get()
            .and_then(|r| r.get_app_name(&service_id));
        let info = ExternalServiceInfo {
            service_id,
            capabilities: capabilities.clone(),
            hostname: hostname.map(String::from),
            machine_id: None,
            service_app_name,
            is_system,
        };

        let services = self.services.lock();
        for handle in services.iter() {
            Self::apply_external_service_update(handle, &info, self.registry.get());
        }
    }

    fn on_external_disconnected(&self, service_id: &Uuid) {
        let services = self.services.lock();
        for handle in services.iter() {
            let mut ids = handle.yielding_service_ids.lock();
            if ids.remove(service_id) && ids.is_empty() {
                handle.yielded.store(false, Ordering::Release);
                if let Some(registry) = self.registry.get() {
                    registry.set_yielded(&handle.service_id, false);
                }
                tracing::info!(
                    embedded_service_id = %handle.service_id,
                    embedded_label = handle.label,
                    %service_id,
                    "embedded service resuming (no more active yielders)"
                );
            }
            if !ids.is_empty() || !handle.yielded.load(Ordering::Relaxed) {
                handle.yield_state_changed.notify_one();
            }
        }
    }

    fn on_machine_id_reported(&self, service_id: &Uuid, machine_id: &str) {
        let service_app_name = self.registry.get().and_then(|r| r.get_app_name(service_id));
        let info = ExternalServiceInfo {
            service_id: *service_id,
            capabilities: BTreeSet::new(),
            hostname: None,
            machine_id: Some(machine_id.to_string()),
            service_app_name,
            is_system: false,
        };

        let services = self.services.lock();
        for handle in services.iter() {
            Self::apply_external_service_update(handle, &info, self.registry.get());
        }
    }

    fn is_capability_yielded(&self, capability: &Capability) -> bool {
        let services = self.services.lock();
        let registry = self.registry.get();
        services.iter().any(|handle| {
            let has_capability = registry
                .and_then(|registry| registry.get_capabilities(&handle.service_id))
                .map_or_else(
                    || handle.capabilities.contains(capability),
                    |capabilities| capabilities.contains(capability),
                );

            has_capability && handle.yielded.load(Ordering::Relaxed)
        })
    }
}

const YIELD_STATE_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(10);

async fn run_yield_state_sync(
    db: sea_orm::DatabaseConnection,
    service_id: Uuid,
    yielding_service_ids: Arc<parking_lot::Mutex<HashSet<Uuid>>>,
    yield_state_changed: Arc<tokio::sync::Notify>,
    cancel: CancellationToken,
) {
    let mut interval = tokio::time::interval(YIELD_STATE_HEARTBEAT);

    sync_yield_state_once(&db, service_id, &yielding_service_ids).await;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                if let Err(err) = clear_yield_state(&db, service_id).await {
                    tracing::warn!(%service_id, error = %err, "failed to clear embedded yield state during shutdown");
                }
                break;
            }
            _ = yield_state_changed.notified() => {}
            _ = interval.tick() => {}
        }

        sync_yield_state_once(&db, service_id, &yielding_service_ids).await;
    }
}

async fn sync_yield_state_once(
    db: &sea_orm::DatabaseConnection,
    service_id: Uuid,
    yielding_service_ids: &Arc<parking_lot::Mutex<HashSet<Uuid>>>,
) {
    let yielded_to = {
        let ids = yielding_service_ids.lock();
        if ids.is_empty() {
            None
        } else {
            let mut sorted: Vec<Uuid> = ids.iter().copied().collect();
            sorted.sort_unstable();
            Some(sorted)
        }
    };

    let result = match yielded_to {
        Some(ids) => persist_yield_state(db, service_id, &ids).await,
        None => clear_yield_state(db, service_id).await,
    };
    if let Err(err) = result {
        tracing::warn!(%service_id, error = %err, "failed to sync embedded yield state");
    }
}

async fn persist_yield_state(
    db: &sea_orm::DatabaseConnection,
    service_id: Uuid,
    yielded_to: &[Uuid],
) -> Result<(), sea_orm::DbErr> {
    let json = serde_json::to_string(&yielded_to.iter().map(Uuid::to_string).collect::<Vec<_>>())
        .map_err(|err| sea_orm::DbErr::Custom(err.to_string()))?;
    let now = time::OffsetDateTime::now_utc();

    embedded_service_runtime_state::Entity::insert(embedded_service_runtime_state::ActiveModel {
        service_id: ActiveValue::Set(service_id),
        yielded_to_json: ActiveValue::Set(Some(json)),
        updated_at: ActiveValue::Set(now),
    })
    .on_conflict(
        OnConflict::column(embedded_service_runtime_state::Column::ServiceId)
            .update_column(embedded_service_runtime_state::Column::YieldedToJson)
            .update_column(embedded_service_runtime_state::Column::UpdatedAt)
            .to_owned(),
    )
    .exec(db)
    .await?;

    Ok(())
}

async fn clear_yield_state(
    db: &sea_orm::DatabaseConnection,
    service_id: Uuid,
) -> Result<(), sea_orm::DbErr> {
    embedded_service_runtime_state::Entity::delete_by_id(service_id)
        .exec(db)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::unused_result_ok,
        reason = "test code: `OnceCell::set(...).ok()` is the canonical idiom for ignoring the case where the cell is already initialized"
    )]

    use super::*;
    use crate::service_host::yielding::matches_yield_policy;
    use uptrakit_service_platform::YieldPolicy;
    use uptrakit_web_api::service_connections::ServiceConnectionRegistry;

    fn make_scheduler_handle() -> EmbeddedServiceHandle {
        EmbeddedServiceHandle {
            service_id: Uuid::nil(),
            label: "scheduler",
            app_name: "uptrakit-scheduler".to_string(),
            yielded: Arc::new(AtomicBool::new(false)),
            yielding_service_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            yield_state_changed: Arc::new(tokio::sync::Notify::new()),
            capabilities: [Capability::Scheduler].into(),
            coexistence_policy: CoexistencePolicy::YieldOnSameAppName,
        }
    }

    fn make_same_host_agent_handle(local_machine_id: &str) -> EmbeddedServiceHandle {
        let local_machine_id = local_machine_id.to_string();
        EmbeddedServiceHandle {
            service_id: Uuid::nil(),
            label: "agent",
            app_name: "uptrakit-agent".to_string(),
            yielded: Arc::new(AtomicBool::new(false)),
            yielding_service_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            yield_state_changed: Arc::new(tokio::sync::Notify::new()),
            capabilities: [Capability::UpdateTracking].into(),
            coexistence_policy: CoexistencePolicy::Custom(Box::new(move |info| {
                matches_yield_policy(
                    YieldPolicy::SameServiceSameHost,
                    "uptrakit-agent",
                    Some(local_machine_id.as_str()),
                    info,
                )
            })),
        }
    }

    fn ext_info(
        service_id: Uuid,
        caps: BTreeSet<Capability>,
        service_app_name: Option<&str>,
    ) -> ExternalServiceInfo {
        ExternalServiceInfo {
            service_id,
            capabilities: caps,
            hostname: None,
            machine_id: None,
            service_app_name: service_app_name.map(String::from),
            is_system: true,
        }
    }

    fn ext_info_with_machine_id(
        service_id: Uuid,
        caps: BTreeSet<Capability>,
        service_app_name: Option<&str>,
        machine_id: Option<&str>,
    ) -> ExternalServiceInfo {
        ExternalServiceInfo {
            service_id,
            capabilities: caps,
            hostname: None,
            machine_id: machine_id.map(String::from),
            service_app_name: service_app_name.map(String::from),
            is_system: true,
        }
    }

    // -----------------------------------------------------------------------
    // Policy unit tests — call should_yield directly, no registry needed.
    // -----------------------------------------------------------------------

    #[test]
    fn yield_on_same_app_name_matches() {
        let handle = make_scheduler_handle();
        let info = ext_info(Uuid::nil(), BTreeSet::new(), Some("uptrakit-scheduler"));
        assert!(EmbeddedServiceHost::should_yield(&handle, &info));
    }

    #[test]
    fn yield_on_same_app_name_no_match_different_app() {
        let handle = make_scheduler_handle();
        // An agent connecting — different app name must NOT trigger a yield.
        let info = ext_info(
            Uuid::nil(),
            [Capability::GracefulShutdown].into(),
            Some("uptrakit-agent"),
        );
        assert!(!EmbeddedServiceHost::should_yield(&handle, &info));
    }

    #[test]
    fn yield_on_same_app_name_no_match_none() {
        let handle = make_scheduler_handle();
        // External service with no app_name must NOT trigger a yield even when
        // it carries the Scheduler capability.
        let info = ext_info(Uuid::nil(), [Capability::Scheduler].into(), None);
        assert!(!EmbeddedServiceHost::should_yield(&handle, &info));
    }

    #[test]
    fn same_service_anywhere_matches_by_app_name_only() {
        let info = ext_info(
            Uuid::nil(),
            [Capability::GracefulShutdown].into(),
            Some("uptrakit-scheduler"),
        );

        assert!(matches_yield_policy(
            YieldPolicy::SameServiceAnywhere,
            "uptrakit-scheduler",
            None,
            &info,
        ));
    }

    #[test]
    fn same_service_same_host_requires_machine_id_match() {
        let matching = ext_info_with_machine_id(
            Uuid::nil(),
            BTreeSet::new(),
            Some("uptrakit-agent"),
            Some("machine-a"),
        );
        let different_machine = ext_info_with_machine_id(
            Uuid::nil(),
            BTreeSet::new(),
            Some("uptrakit-agent"),
            Some("machine-b"),
        );

        assert!(matches_yield_policy(
            YieldPolicy::SameServiceSameHost,
            "uptrakit-agent",
            Some("machine-a"),
            &matching,
        ));
        assert!(!matches_yield_policy(
            YieldPolicy::SameServiceSameHost,
            "uptrakit-agent",
            Some("machine-a"),
            &different_machine,
        ));
    }

    #[test]
    fn custom_policy_closure() {
        let handle = EmbeddedServiceHandle {
            service_id: Uuid::nil(),
            label: "test",
            app_name: "test-app".to_string(),
            yielded: Arc::new(AtomicBool::new(false)),
            yielding_service_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            yield_state_changed: Arc::new(tokio::sync::Notify::new()),
            capabilities: [Capability::Scheduler].into(),
            coexistence_policy: CoexistencePolicy::Custom(Box::new(|_| true)),
        };
        let info = ext_info(Uuid::nil(), BTreeSet::new(), None);
        assert!(EmbeddedServiceHost::should_yield(&handle, &info));
    }

    #[test]
    fn never_yield_policy() {
        let handle = EmbeddedServiceHandle {
            service_id: Uuid::nil(),
            label: "test",
            app_name: "test-app".to_string(),
            yielded: Arc::new(AtomicBool::new(false)),
            yielding_service_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            yield_state_changed: Arc::new(tokio::sync::Notify::new()),
            capabilities: [Capability::Scheduler].into(),
            coexistence_policy: CoexistencePolicy::NeverYield,
        };
        // Even with a matching app name, NeverYield must return false.
        let info = ext_info(
            Uuid::nil(),
            [Capability::Scheduler].into(),
            Some("test-app"),
        );
        assert!(!EmbeddedServiceHost::should_yield(&handle, &info));
    }

    // -----------------------------------------------------------------------
    // Integration tests — call on_external_connected, registry required.
    //
    // The registry is pre-seeded so that on_external_connected can look up
    // service_app_name for the connecting external service.
    // -----------------------------------------------------------------------

    async fn make_host_with_scheduler_registry(
        ids_and_names: &[(Uuid, &str)],
    ) -> EmbeddedServiceHost {
        let registry = ServiceConnectionRegistry::new();
        for (id, name) in ids_and_names {
            registry
                .register(*id, BTreeSet::new(), None, None, Some(name.to_string()))
                .await;
        }
        let host = EmbeddedServiceHost::new();
        host.registry.set(registry).ok();
        host
    }

    #[tokio::test]
    async fn on_external_connected_sets_yielded_flag() {
        let ext_id = Uuid::now_v7();
        let host = make_host_with_scheduler_registry(&[(ext_id, "uptrakit-scheduler")]).await;
        host.services.lock().push(make_scheduler_handle());

        let ext_caps: BTreeSet<Capability> = [Capability::Scheduler].into();
        host.on_external_connected(ext_id, &ext_caps, None, true);

        assert!(host.is_capability_yielded(&Capability::Scheduler));
    }

    #[tokio::test]
    async fn non_scheduler_service_does_not_yield_embedded_scheduler() {
        let ext_id = Uuid::now_v7();
        // Agent connects with a different app name — must NOT trigger yield.
        let host = make_host_with_scheduler_registry(&[(ext_id, "uptrakit-agent")]).await;
        host.services.lock().push(make_scheduler_handle());

        let ext_caps: BTreeSet<Capability> = [Capability::GracefulShutdown].into();
        host.on_external_connected(ext_id, &ext_caps, None, false);

        assert!(!host.is_capability_yielded(&Capability::Scheduler));
    }

    #[test]
    fn on_external_disconnected_clears_yielded_flag() {
        let host = EmbeddedServiceHost::new();
        let id = Uuid::now_v7();
        {
            let handle = make_scheduler_handle();
            handle.yielding_service_ids.lock().insert(id);
            handle.yielded.store(true, Ordering::Release);
            host.services.lock().push(handle);
        }

        host.on_external_disconnected(&id);

        assert!(!host.is_capability_yielded(&Capability::Scheduler));
    }

    #[tokio::test]
    async fn multiple_yielders_one_disconnect_yield_remains() {
        let id_a = Uuid::now_v7();
        let id_b = Uuid::now_v7();
        let host = make_host_with_scheduler_registry(&[
            (id_a, "uptrakit-scheduler"),
            (id_b, "uptrakit-scheduler"),
        ])
        .await;
        host.services.lock().push(make_scheduler_handle());

        let caps: BTreeSet<Capability> = [Capability::Scheduler].into();
        host.on_external_connected(id_a, &caps, None, true);
        host.on_external_connected(id_b, &caps, None, true);
        assert!(host.is_capability_yielded(&Capability::Scheduler));

        // Only A disconnects — B is still connected, yield must remain.
        host.on_external_disconnected(&id_a);
        assert!(host.is_capability_yielded(&Capability::Scheduler));

        // Now B disconnects — yield must clear.
        host.on_external_disconnected(&id_b);
        assert!(!host.is_capability_yielded(&Capability::Scheduler));
    }

    #[tokio::test]
    async fn same_service_id_reconnect_does_not_double_count() {
        let id = Uuid::now_v7();
        let host = make_host_with_scheduler_registry(&[(id, "uptrakit-scheduler")]).await;
        host.services.lock().push(make_scheduler_handle());

        let caps: BTreeSet<Capability> = [Capability::Scheduler].into();

        // Register twice with the same ID (e.g. reconnect without disconnect).
        host.on_external_connected(id, &caps, None, true);
        host.on_external_connected(id, &caps, None, true);
        assert!(host.is_capability_yielded(&Capability::Scheduler));

        // Single disconnect must clear the flag since the set has exactly one entry.
        host.on_external_disconnected(&id);
        assert!(!host.is_capability_yielded(&Capability::Scheduler));
    }

    #[tokio::test]
    async fn machine_id_report_triggers_same_host_yield_after_connect() {
        let external_id = Uuid::now_v7();
        let host = make_host_with_scheduler_registry(&[(external_id, "uptrakit-agent")]).await;
        host.services
            .lock()
            .push(make_same_host_agent_handle("machine-a"));

        host.on_external_connected(external_id, &BTreeSet::new(), None, false);
        assert!(!host.is_capability_yielded(&Capability::UpdateTracking));

        host.on_machine_id_reported(&external_id, "machine-a");
        assert!(host.is_capability_yielded(&Capability::UpdateTracking));
    }

    #[tokio::test]
    async fn is_capability_yielded_uses_current_registry_capabilities() {
        let embedded_service_id = Uuid::now_v7();
        let registry = ServiceConnectionRegistry::new();
        let _ = registry
            .register(
                embedded_service_id,
                [Capability::Scheduler].into(),
                None,
                None,
                Some("uptrakit-scheduler".to_string()),
            )
            .await;

        let host = EmbeddedServiceHost::new();
        host.registry.set(registry.clone()).ok();

        let handle = EmbeddedServiceHandle {
            service_id: embedded_service_id,
            label: "test",
            app_name: "uptrakit-scheduler".to_string(),
            yielded: Arc::new(AtomicBool::new(true)),
            yielding_service_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            yield_state_changed: Arc::new(tokio::sync::Notify::new()),
            capabilities: BTreeSet::new(),
            coexistence_policy: CoexistencePolicy::YieldOnSameAppName,
        };
        host.services.lock().push(handle);

        assert!(host.is_capability_yielded(&Capability::Scheduler));

        registry
            .update_capabilities(embedded_service_id, BTreeSet::new())
            .await;

        assert!(
            !host.is_capability_yielded(&Capability::Scheduler),
            "yield bookkeeping should stop advertising dropped capabilities"
        );
    }

    #[test]
    fn unknown_service_disconnect_does_not_clear_yield() {
        let host = EmbeddedServiceHost::new();
        let known_id = Uuid::now_v7();
        {
            let handle = make_scheduler_handle();
            handle.yielding_service_ids.lock().insert(known_id);
            handle.yielded.store(true, Ordering::Release);
            host.services.lock().push(handle);
        }

        // Disconnect of a service that never set the yield must not clear it.
        host.on_external_disconnected(&Uuid::now_v7());
        assert!(host.is_capability_yielded(&Capability::Scheduler));
    }
}
