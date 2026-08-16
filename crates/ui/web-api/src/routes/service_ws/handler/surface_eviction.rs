//! Eviction of surface providers for services yielded to a newly-connected
//! external counterpart.
//!
//! `validate_contract_collisions` (`uptrakit_surface_proxy::registry`)
//! rejects a second provider for the same `surface_id` with Universal
//! targeting in the same effective scope. When an external service connects,
//! the embedded host flips the colliding embedded service's `yielded` flag
//! *before* any message from the newly-connected service is processed
//! (`EmbeddedServiceNotifier::on_external_connected`); eviction of that
//! embedded service's surface registration must be equally deterministic —
//! not merely eventual — so the external service's own (equivalent)
//! registration is admitted rather than rejected as a collision the moment
//! it registers. Plan section A3.

use std::sync::Arc;

use uptrakit_web_api_types::events::AdminEvent;

use crate::AppState;

/// Unregisters the surface provider of every currently-yielded service (per
/// `state.service_connections.is_yielded`), fails its in-flight surface
/// requests, and broadcasts `AdminEvent::SurfacesChanged` so connected admin
/// UIs refresh their surface catalogue.
///
/// Called immediately after `notifier.on_external_connected(...)` at both
/// WebSocket connect sites
/// (`session_authenticated::register_connection`,
/// `session_enrolled::setup_enrolled_session`) — by the time this returns,
/// no now-yielded embedded service still occupies a `surface_id` the
/// newly-connecting external service is about to register.
///
/// Lock discipline: `SurfaceRegistry::service_ids()` clones ids under the
/// registry's internal lock and releases it before returning. This function
/// only calls `service_connections.is_yielded(...)` after that lock has been
/// released — never call `is_yielded` while holding the registry lock.
#[tracing::instrument(skip_all)]
pub(super) async fn evict_yielded_service_surfaces(state: &Arc<AppState>) {
    let service_ids = state.surface_proxy_deps.registry.service_ids();

    for service_id in service_ids {
        if !state.service_connections.is_yielded(&service_id) {
            continue;
        }

        let Some(provider_id) = state
            .surface_proxy_deps
            .registry
            .provider_id_for_service(&service_id)
        else {
            continue;
        };
        let tenant_id = state
            .surface_proxy_deps
            .registry
            .tenant_id_for_service(&service_id);

        state
            .surface_proxy_deps
            .proxy
            .fail_in_flight_for_provider(&provider_id);
        state
            .surface_proxy_deps
            .registry
            .unregister_service(&service_id);

        tracing::info!(
            %service_id,
            provider_id = %provider_id,
            "evicted surface provider of a yielded service"
        );

        if state.shutdown_token.is_cancelled() {
            continue;
        }

        match tenant_id {
            Some(tenant_id) => {
                state
                    .notification
                    .event_broadcaster
                    .send(tenant_id, AdminEvent::SurfacesChanged)
                    .await;
            }
            None => {
                state
                    .notification
                    .event_broadcaster
                    .send_global(AdminEvent::SurfacesChanged)
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use uuid::Uuid;

    use uptrakit_wire::Capability;

    use super::super::test_support::test_surface_registration;
    use super::*;

    /// Registers `service_id` both on the connection registry (so
    /// `is_yielded`/`set_yielded` have an entry) and on the surface registry
    /// under a Tenant/Targeted `service.{provider_id}` provider — mirrors
    /// `test_support::register_test_runtime_state` but lets each caller pick
    /// a distinct `provider_id`, which that fixed helper does not allow.
    /// Returns the still-open push receiver: dropping it would make any
    /// subsequent `SurfaceProxy::invoke` targeting this service fail with
    /// `SendFailed` before eviction ever runs, which is not what these tests
    /// exercise.
    #[cfg(feature = "db-sqlite")]
    async fn register_test_service(
        state: &Arc<AppState>,
        tenant_id: Uuid,
        provider_id: &str,
    ) -> (
        Uuid,
        tokio::sync::mpsc::Receiver<uptrakit_wire::ControllerMessage>,
    ) {
        let service_id = Uuid::now_v7();
        let (push_rx, _handle) = state
            .service_connections
            .register(
                service_id,
                BTreeSet::from([Capability::UiSurfaces]),
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;
        state
            .surface_proxy_deps
            .registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_id),
                test_surface_registration(provider_id, tenant_id),
            )
            .expect("surface registration should succeed");
        (service_id, push_rx)
    }

    /// Guard-to-delete for a red check: removing the `is_yielded` filter
    /// (i.e. unregistering every enumerated service unconditionally) makes
    /// this test fail because the non-yielded provider's registration would
    /// then also be evicted.
    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn evicts_only_the_yielded_provider() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

        let (yielded_id, _rx_yielded) =
            register_test_service(&state, tenant_id, "service.provider-a").await;
        let (live_id, _rx_live) =
            register_test_service(&state, tenant_id, "service.provider-b").await;

        state.service_connections.set_yielded(&yielded_id, true);

        evict_yielded_service_surfaces(&state).await;

        assert!(
            state
                .surface_proxy_deps
                .registry
                .provider_id_for_service(&yielded_id)
                .is_none(),
            "the yielded service's provider must be unregistered"
        );
        assert!(
            state
                .surface_proxy_deps
                .registry
                .provider_id_for_service(&live_id)
                .is_some(),
            "the non-yielded service's provider must remain registered"
        );
    }

    /// Guard-to-delete for a red check: the eviction loop must actually call
    /// `fail_in_flight_for_provider` for the evicted provider — removing that
    /// call (leaving only `unregister_service`) makes this test fail: the
    /// spawned `invoke` never observes `ServiceDisconnected` and the
    /// `tokio::time::timeout` elapses instead.
    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn evicted_provider_fails_its_in_flight_requests() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

        let (yielded_id, _push_rx) =
            register_test_service(&state, tenant_id, "service.provider-a").await;

        let state_for_invoke = Arc::clone(&state);
        let invoke_task = tokio::spawn(async move {
            state_for_invoke
                .surface_proxy_deps
                .proxy
                .invoke(
                    &state_for_invoke.service_connections,
                    &state_for_invoke.surface_proxy_deps.registry,
                    crate::surface_proxy::SurfaceInvokeRequest::new(
                        tenant_id,
                        "ssh.guest.panel".to_string(),
                        "refresh".to_string(),
                        None,
                        "yield-eviction-test".to_string(),
                        Some("service.provider-a".to_string()),
                        crate::surface_proxy::SurfaceCallerOrigin::UserSession {
                            user_id: uuid::Uuid::now_v7(),
                            session_id: "session-1".to_string(),
                        },
                        serde_json::Map::new(),
                        None,
                    ),
                    Some(std::time::Duration::from_secs(30)),
                )
                .await
        });
        tokio::task::yield_now().await;

        state.service_connections.set_yielded(&yielded_id, true);
        evict_yielded_service_surfaces(&state).await;

        let invoke_result = tokio::time::timeout(std::time::Duration::from_secs(1), invoke_task)
            .await
            .expect("in-flight invoke should resolve after fail_in_flight_for_provider")
            .expect("invoke task should join");
        assert!(
            matches!(
                invoke_result,
                Err(crate::surface_proxy::SurfaceProxyError::ServiceDisconnected)
            ),
            "eviction should have cancelled the in-flight invoke: {invoke_result:?}"
        );
    }

    /// Guard-to-delete for a red check: replacing the `continue` in the
    /// `!is_yielded` branch with unconditional eviction makes this test fail
    /// — both providers stay registered here because neither is yielded, and
    /// the `SurfacesChanged` broadcast that unconditional eviction would emit
    /// is caught by the `try_recv` assertion.
    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn no_yielded_services_is_a_no_op() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

        let (service_a, _rx_a) =
            register_test_service(&state, tenant_id, "service.provider-a").await;
        let (service_b, _rx_b) =
            register_test_service(&state, tenant_id, "service.provider-b").await;

        let mut rx = state
            .notification
            .event_broadcaster
            .subscribe(tenant_id)
            .await;

        evict_yielded_service_surfaces(&state).await;

        assert!(
            rx.try_recv().is_err(),
            "no service is yielded — nothing changed, so no SurfacesChanged broadcast"
        );
        assert!(
            state
                .surface_proxy_deps
                .registry
                .provider_id_for_service(&service_a)
                .is_some(),
            "no service is yielded — provider A must remain registered"
        );
        assert!(
            state
                .surface_proxy_deps
                .registry
                .provider_id_for_service(&service_b)
                .is_some(),
            "no service is yielded — provider B must remain registered"
        );
    }
}
