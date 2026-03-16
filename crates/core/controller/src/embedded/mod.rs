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
pub(crate) mod provision;
#[allow(dead_code)] // Infrastructure types used by follow-up service embeddings.
pub(crate) mod types;

use std::collections::{BTreeSet, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio_util::sync::CancellationToken;
use uptrakit_internal_wire::Capability;
use uptrakit_web_api::embedded_support::EmbeddedServiceNotifier;
use uuid::Uuid;

use crate::tasks::BackgroundTasks;
use types::{EmbeddedTransport, ExternalServiceInfo};

/// Custom yield predicate for embedded service coexistence decisions.
type YieldCheckFn = Box<dyn Fn(&ExternalServiceInfo) -> bool + Send + Sync>;

// ---------------------------------------------------------------------------
// EmbeddedServiceHandle
// ---------------------------------------------------------------------------

/// Internal handle for a single embedded service tracked by the host.
struct EmbeddedServiceHandle {
    _service_id: Uuid,
    label: &'static str,
    yielded: Arc<AtomicBool>,
    /// Set of external service IDs that are currently causing this embedded
    /// service to yield. The `yielded` AtomicBool is set when the first ID is
    /// inserted and cleared when the last ID is removed, preventing false
    /// resumes when multiple yielder services are connected simultaneously.
    yielding_service_ids: Arc<parking_lot::Mutex<HashSet<Uuid>>>,
    capabilities: BTreeSet<Capability>,
    yield_check: Option<YieldCheckFn>,
}

// ---------------------------------------------------------------------------
// EmbeddedServiceHost
// ---------------------------------------------------------------------------

/// Orchestrator for embedded services running inside the controller process.
pub(crate) struct EmbeddedServiceHost {
    services: parking_lot::Mutex<Vec<EmbeddedServiceHandle>>,
}

impl EmbeddedServiceHost {
    pub(crate) fn new() -> Self {
        Self {
            services: parking_lot::Mutex::new(Vec::new()),
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
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn add(
        &self,
        label: &'static str,
        app_name: &str,
        capabilities: BTreeSet<Capability>,
        is_system_service: bool,
        yield_check: Option<YieldCheckFn>,
        run_fn: impl FnOnce(
            EmbeddedTransport,
            CancellationToken,
        ) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + 'static,
        state: &Arc<uptrakit_web_api::AppState>,
        bg: &mut BackgroundTasks,
    ) -> rootcause::Result<Uuid> {
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
            )
            .await?
        } else {
            // Non-system embedded services are not yet supported — this path
            // is reserved for future use.
            unimplemented!("non-system embedded services are not yet implemented");
        };

        // 2. Register in ServiceConnectionRegistry.
        let (push_rx, _cancel_token) = state
            .service_connections
            .register(service_id, capabilities.clone(), None, None, None)
            .await;

        // 3. Create bidirectional channels.
        //
        // Service → Controller (ServiceMessage):
        //   service_tx is given to the EmbeddedTransport
        //   service_rx is consumed by the run closure (future: processor)
        //
        // Controller → Service (ControllerMessage):
        //   ctrl_tx is used by the response forwarder
        //   ctrl_rx is given to the EmbeddedTransport
        let (service_tx, _service_rx) =
            tokio::sync::mpsc::channel::<uptrakit_internal_wire::ServiceMessage>(32);
        let (ctrl_tx, ctrl_rx) =
            tokio::sync::mpsc::channel::<uptrakit_internal_wire::ControllerMessage>(32);

        let yielded = Arc::new(AtomicBool::new(false));

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
        let transport = EmbeddedTransport::new(service_tx, ctrl_rx, Arc::clone(&yielded));
        let service_cancel = bg.child_token();
        let service_handle = tokio::spawn(run_fn(transport, service_cancel));
        bg.track(label, service_handle);

        // 6. Track the handle.
        {
            let mut services = self.services.lock();
            services.push(EmbeddedServiceHandle {
                _service_id: service_id,
                label,
                yielded,
                yielding_service_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
                capabilities,
                yield_check,
            });
        }

        tracing::info!(
            %service_id,
            label,
            app_name,
            "embedded service registered"
        );

        Ok(service_id)
    }

    /// Evaluate whether a specific embedded service should yield based on
    /// its yield closure and the external service info.
    fn should_yield(handle: &EmbeddedServiceHandle, info: &ExternalServiceInfo) -> bool {
        handle.yield_check.as_ref().is_some_and(|check| check(info))
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
        let info = ExternalServiceInfo {
            service_id,
            capabilities: capabilities.clone(),
            hostname: hostname.map(String::from),
            machine_id: None,
            is_system,
        };

        let services = self.services.lock();
        for handle in services.iter() {
            if Self::should_yield(handle, &info) {
                let mut ids = handle.yielding_service_ids.lock();
                if ids.insert(service_id) && ids.len() == 1 {
                    handle.yielded.store(true, Ordering::Release);
                    tracing::info!(
                        embedded_label = handle.label,
                        %service_id,
                        "embedded service yielding to external"
                    );
                }
            }
        }
    }

    fn on_external_disconnected(&self, service_id: &Uuid) {
        let services = self.services.lock();
        for handle in services.iter() {
            let mut ids = handle.yielding_service_ids.lock();
            if ids.remove(service_id) && ids.is_empty() {
                handle.yielded.store(false, Ordering::Release);
                tracing::info!(
                    embedded_label = handle.label,
                    %service_id,
                    "embedded service resuming (no more active yielders)"
                );
            }
        }
    }

    fn on_machine_id_reported(&self, _service_id: &Uuid, _machine_id: &str) {
        // Reserved for future coexistence policies that use machine_id
        // matching (e.g. YieldOnSameHost).
    }

    fn is_capability_yielded(&self, capability: &Capability) -> bool {
        let services = self.services.lock();
        services
            .iter()
            .any(|h| h.capabilities.contains(capability) && h.yielded.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_scheduler_handle() -> EmbeddedServiceHandle {
        EmbeddedServiceHandle {
            _service_id: Uuid::nil(),
            label: "scheduler",
            yielded: Arc::new(AtomicBool::new(false)),
            yielding_service_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            capabilities: [Capability::Scheduler].into(),
            yield_check: Some(Box::new(|info: &ExternalServiceInfo| {
                info.capabilities.contains(&Capability::Scheduler)
            })),
        }
    }

    fn ext_info(service_id: Uuid, caps: BTreeSet<Capability>) -> ExternalServiceInfo {
        ExternalServiceInfo {
            service_id,
            capabilities: caps,
            hostname: None,
            machine_id: None,
            is_system: true,
        }
    }

    #[test]
    fn yield_check_matches_scheduler_capability() {
        let handle = make_scheduler_handle();
        let info = ext_info(
            Uuid::nil(),
            [Capability::Scheduler, Capability::DatabaseAccess].into(),
        );
        assert!(EmbeddedServiceHost::should_yield(&handle, &info));
    }

    #[test]
    fn yield_check_does_not_match_non_scheduler_service() {
        let handle = make_scheduler_handle();
        // A service with GracefulShutdown only must NOT trigger a yield.
        let info = ext_info(Uuid::nil(), [Capability::GracefulShutdown].into());
        assert!(!EmbeddedServiceHost::should_yield(&handle, &info));
    }

    #[test]
    fn no_yield_check_never_yields() {
        let handle = EmbeddedServiceHandle {
            _service_id: Uuid::nil(),
            label: "test",
            yielded: Arc::new(AtomicBool::new(false)),
            yielding_service_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            capabilities: [Capability::Scheduler].into(),
            yield_check: None,
        };
        let info = ext_info(Uuid::nil(), [Capability::Scheduler].into());
        assert!(!EmbeddedServiceHost::should_yield(&handle, &info));
    }

    #[test]
    fn yield_check_closure_always_true() {
        let handle = EmbeddedServiceHandle {
            _service_id: Uuid::nil(),
            label: "test",
            yielded: Arc::new(AtomicBool::new(false)),
            yielding_service_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            capabilities: [Capability::Scheduler].into(),
            yield_check: Some(Box::new(|_| true)),
        };
        let info = ext_info(Uuid::nil(), BTreeSet::new());
        assert!(EmbeddedServiceHost::should_yield(&handle, &info));
    }

    #[test]
    fn on_external_connected_sets_yielded_flag() {
        let host = EmbeddedServiceHost::new();
        host.services.lock().push(make_scheduler_handle());

        let ext_caps: BTreeSet<Capability> = [Capability::Scheduler].into();
        host.on_external_connected(Uuid::now_v7(), &ext_caps, None, true);

        assert!(host.is_capability_yielded(&Capability::Scheduler));
    }

    #[test]
    fn non_scheduler_service_does_not_yield_embedded_scheduler() {
        let host = EmbeddedServiceHost::new();
        host.services.lock().push(make_scheduler_handle());

        // Agent connects with GracefulShutdown only — must NOT trigger yield.
        let ext_caps: BTreeSet<Capability> = [Capability::GracefulShutdown].into();
        host.on_external_connected(Uuid::now_v7(), &ext_caps, None, false);

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

    #[test]
    fn multiple_yielders_one_disconnect_yield_remains() {
        let host = EmbeddedServiceHost::new();
        host.services.lock().push(make_scheduler_handle());

        let id_a = Uuid::now_v7();
        let id_b = Uuid::now_v7();
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

    #[test]
    fn same_service_id_reconnect_does_not_double_count() {
        let host = EmbeddedServiceHost::new();
        host.services.lock().push(make_scheduler_handle());

        let id = Uuid::now_v7();
        let caps: BTreeSet<Capability> = [Capability::Scheduler].into();

        // Register twice with the same ID (e.g. reconnect without disconnect).
        host.on_external_connected(id, &caps, None, true);
        host.on_external_connected(id, &caps, None, true);
        assert!(host.is_capability_yielded(&Capability::Scheduler));

        // Single disconnect must clear the flag since the set has exactly one entry.
        host.on_external_disconnected(&id);
        assert!(!host.is_capability_yielded(&Capability::Scheduler));
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
