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

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio_util::sync::CancellationToken;
use uptrakit_internal_wire::Capability;
use uptrakit_web_api::embedded_support::EmbeddedServiceNotifier;
use uuid::Uuid;

use crate::tasks::BackgroundTasks;
use types::{CoexistencePolicy, EmbeddedTransport, ExternalServiceInfo};

// ---------------------------------------------------------------------------
// EmbeddedServiceHandle
// ---------------------------------------------------------------------------

/// Internal handle for a single embedded service tracked by the host.
struct EmbeddedServiceHandle {
    service_id: Uuid,
    label: &'static str,
    yielded: Arc<AtomicBool>,
    coexistence_policy: CoexistencePolicy,
    capabilities: BTreeSet<Capability>,
    yield_check: Option<Box<dyn Fn(&ExternalServiceInfo) -> bool + Send + Sync>>,
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
        coexistence_policy: CoexistencePolicy,
        yield_check: Option<Box<dyn Fn(&ExternalServiceInfo) -> bool + Send + Sync>>,
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
            .register(service_id, capabilities.clone(), None, None)
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
                service_id,
                label,
                yielded,
                coexistence_policy,
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
    /// its policy and the external service info.
    fn should_yield(handle: &EmbeddedServiceHandle, info: &ExternalServiceInfo) -> bool {
        // Check custom yield closure first.
        if let Some(ref check) = handle.yield_check {
            return check(info);
        }

        match handle.coexistence_policy {
            CoexistencePolicy::YieldAlways => {
                // Yield if any capability overlaps.
                handle
                    .capabilities
                    .intersection(&info.capabilities)
                    .next()
                    .is_some()
            }
            CoexistencePolicy::NeverYield => false,
        }
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
            if Self::should_yield(handle, &info) && !handle.yielded.load(Ordering::Relaxed) {
                handle.yielded.store(true, Ordering::Relaxed);
                tracing::info!(
                    embedded_label = handle.label,
                    external_service_id = %service_id,
                    "embedded service yielding to external"
                );
            }
        }
    }

    fn on_external_disconnected(&self, service_id: &Uuid) {
        let services = self.services.lock();
        for handle in services.iter() {
            if handle.yielded.load(Ordering::Relaxed) {
                // For simplicity, reset yield on any disconnect.
                // A more sophisticated implementation would track which
                // external service caused the yield.
                handle.yielded.store(false, Ordering::Relaxed);
                tracing::info!(
                    embedded_label = handle.label,
                    external_service_id = %service_id,
                    "embedded service resuming (external disconnected)"
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

    #[test]
    fn yield_always_matches_overlapping_capabilities() {
        let handle = EmbeddedServiceHandle {
            service_id: Uuid::nil(),
            label: "test",
            yielded: Arc::new(AtomicBool::new(false)),
            coexistence_policy: CoexistencePolicy::YieldAlways,
            capabilities: [Capability::Scheduler].into(),
            yield_check: None,
        };

        let info = ExternalServiceInfo {
            service_id: Uuid::nil(),
            capabilities: [Capability::Scheduler, Capability::DatabaseAccess].into(),
            hostname: None,
            machine_id: None,
            is_system: true,
        };

        assert!(EmbeddedServiceHost::should_yield(&handle, &info));
    }

    #[test]
    fn yield_always_does_not_match_disjoint_capabilities() {
        let handle = EmbeddedServiceHandle {
            service_id: Uuid::nil(),
            label: "test",
            yielded: Arc::new(AtomicBool::new(false)),
            coexistence_policy: CoexistencePolicy::YieldAlways,
            capabilities: [Capability::Scheduler].into(),
            yield_check: None,
        };

        let info = ExternalServiceInfo {
            service_id: Uuid::nil(),
            capabilities: [Capability::DatabaseAccess].into(),
            hostname: None,
            machine_id: None,
            is_system: false,
        };

        assert!(!EmbeddedServiceHost::should_yield(&handle, &info));
    }

    #[test]
    fn never_yield_ignores_overlapping_capabilities() {
        let handle = EmbeddedServiceHandle {
            service_id: Uuid::nil(),
            label: "test",
            yielded: Arc::new(AtomicBool::new(false)),
            coexistence_policy: CoexistencePolicy::NeverYield,
            capabilities: [Capability::Scheduler].into(),
            yield_check: None,
        };

        let info = ExternalServiceInfo {
            service_id: Uuid::nil(),
            capabilities: [Capability::Scheduler].into(),
            hostname: None,
            machine_id: None,
            is_system: true,
        };

        assert!(!EmbeddedServiceHost::should_yield(&handle, &info));
    }

    #[test]
    fn custom_yield_check_overrides_policy() {
        let handle = EmbeddedServiceHandle {
            service_id: Uuid::nil(),
            label: "test",
            yielded: Arc::new(AtomicBool::new(false)),
            coexistence_policy: CoexistencePolicy::NeverYield,
            capabilities: [Capability::Scheduler].into(),
            yield_check: Some(Box::new(|_| true)),
        };

        let info = ExternalServiceInfo {
            service_id: Uuid::nil(),
            capabilities: BTreeSet::new(),
            hostname: None,
            machine_id: None,
            is_system: false,
        };

        assert!(EmbeddedServiceHost::should_yield(&handle, &info));
    }

    #[test]
    fn on_external_connected_sets_yielded_flag() {
        let host = EmbeddedServiceHost::new();
        {
            let mut services = host.services.lock();
            services.push(EmbeddedServiceHandle {
                service_id: Uuid::nil(),
                label: "scheduler",
                yielded: Arc::new(AtomicBool::new(false)),
                coexistence_policy: CoexistencePolicy::YieldAlways,
                capabilities: [Capability::Scheduler].into(),
                yield_check: None,
            });
        }

        let ext_caps: BTreeSet<Capability> = [Capability::Scheduler].into();
        host.on_external_connected(Uuid::now_v7(), &ext_caps, None, true);

        assert!(host.is_capability_yielded(&Capability::Scheduler));
    }

    #[test]
    fn on_external_disconnected_clears_yielded_flag() {
        let host = EmbeddedServiceHost::new();
        {
            let mut services = host.services.lock();
            services.push(EmbeddedServiceHandle {
                service_id: Uuid::nil(),
                label: "scheduler",
                yielded: Arc::new(AtomicBool::new(true)),
                coexistence_policy: CoexistencePolicy::YieldAlways,
                capabilities: [Capability::Scheduler].into(),
                yield_check: None,
            });
        }

        host.on_external_disconnected(&Uuid::now_v7());

        assert!(!host.is_capability_yielded(&Capability::Scheduler));
    }
}
