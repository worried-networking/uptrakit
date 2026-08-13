//! Embedded-mode WebSocket message handler.
//!
//! This module implements the handler loop for services running inside the
//! controller binary in embedded mode, sharing the [`DatabaseConnection`].

use std::collections::BTreeSet;
use std::sync::Arc;

use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_wire::report_tracker::ReportTracker;
use uptrakit_wire::{Capability, ServiceMessage};

use super::message_processor::MessageProcessor;
use super::session_authenticated::load_session_host_ids;
use super::shared_types::ProcessorAction;
use super::workload;
use crate::AppState;

// ---------------------------------------------------------------------------
// Embedded service message handler
// ---------------------------------------------------------------------------

/// Shared transport parameters for a single embedded handler invocation.
///
/// Bundles the fields common to [`run_embedded_message_handler`] and
/// [`run_embedded_system_message_handler`] so each stays within the
/// `clippy::too_many_arguments` limit. [`run_embedded_message_handler`] takes
/// a tenant id alongside this struct (tenant services);
/// [`run_embedded_system_message_handler`] takes none — system services are
/// always built with `service_tenant_id: None`.
pub(crate) struct EmbeddedHandlerCallParams<'a> {
    pub(crate) state: Arc<AppState>,
    pub(crate) service_id: uuid::Uuid,
    pub(crate) connection_id: uuid::Uuid,
    pub(crate) capabilities: &'a BTreeSet<Capability>,
    pub(crate) app_name: &'a str,
    pub(crate) service_rx: tokio::sync::mpsc::Receiver<ServiceMessage>,
    pub(crate) cancel: tokio_util::sync::CancellationToken,
}

/// Run a message handler loop for an embedded service.
///
/// This creates a [`MessageProcessor`] configured for an embedded (in-process)
/// service and reads messages from the provided channel. Replies are pushed
/// back through the [`ServiceConnectionRegistry`].
///
/// Used by `embedded_support::run_embedded_message_handler`.
pub(crate) async fn run_embedded_message_handler(
    params: EmbeddedHandlerCallParams<'_>,
    tenant_id: uuid::Uuid,
) {
    let EmbeddedHandlerCallParams {
        state,
        service_id,
        connection_id,
        capabilities,
        app_name,
        service_rx,
        cancel,
    } = params;
    run_embedded_message_handler_inner(
        state,
        EmbeddedHandlerSession {
            service_id,
            connection_id,
            is_system: false,
            service_tenant_id: Some(tenant_id),
            app_name,
        },
        capabilities,
        service_rx,
        cancel,
    )
    .await;
}

/// System services are untenanted by construction: the inner session is
/// always built with `service_tenant_id: None`, regardless of any tenant
/// context the caller might otherwise have available.
pub(crate) async fn run_embedded_system_message_handler(params: EmbeddedHandlerCallParams<'_>) {
    let EmbeddedHandlerCallParams {
        state,
        service_id,
        connection_id,
        capabilities,
        app_name,
        service_rx,
        cancel,
    } = params;
    run_embedded_message_handler_inner(
        state,
        EmbeddedHandlerSession {
            service_id,
            connection_id,
            is_system: true,
            service_tenant_id: None,
            app_name,
        },
        capabilities,
        service_rx,
        cancel,
    )
    .await;
}

pub(super) struct EmbeddedHandlerSession<'a> {
    service_id: uuid::Uuid,
    connection_id: uuid::Uuid,
    is_system: bool,
    service_tenant_id: Option<uuid::Uuid>,
    app_name: &'a str,
}

pub(super) async fn run_embedded_message_handler_inner(
    state: Arc<AppState>,
    session: EmbeddedHandlerSession<'_>,
    capabilities: &BTreeSet<Capability>,
    mut service_rx: tokio::sync::mpsc::Receiver<ServiceMessage>,
    cancel: tokio_util::sync::CancellationToken,
) {
    let has_software_discovery = capabilities.contains(&Capability::SoftwareDiscovery);
    let has_update_hooks = capabilities.contains(&Capability::UpdateHooks);
    let has_ui_surfaces = capabilities.contains(&Capability::UiSurfaces);
    let has_workload_claims = capabilities.contains(&Capability::WorkloadClaims);
    let has_update_tracking = capabilities.contains(&Capability::UpdateTracking);

    let linked_host_ids =
        load_session_host_ids(&state, session.service_id, has_software_discovery).await;

    let mut processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id: session.service_id,
        cert: None,
        is_system: session.is_system,
        has_update_tracking,
        has_software_discovery,
        has_update_hooks,
        has_ui_surfaces,
        has_workload_claims,
        runtime_instance_id: None,
        service_app_name: Some(session.app_name.to_string()),
        service_tenant_id: session.service_tenant_id,
        linked_host_ids,
        report_tracker: ReportTracker::new(),
    };

    'msg_loop: loop {
        let msg = tokio::select! {
            biased;
            () = cancel.cancelled() => break 'msg_loop,
            msg = service_rx.recv() => match msg {
                Some(m) => m,
                None => break 'msg_loop,
            },
        };

        // dispatch and reply-send are wrapped in separate cancellable selects so
        // that drain/abort can interrupt even when a SeaORM query or a channel
        // send is in progress. Dropping dispatch mid-flight cancels any in-flight
        // DB query; the connection is returned to the pool and the transaction
        // rolled back. cleanup_embedded_service_session handles workload release.
        let response = tokio::select! {
            biased;
            () = cancel.cancelled() => break 'msg_loop,
            r = processor.dispatch(msg, None) => r,
        };

        for reply in response.replies {
            tokio::select! {
                biased;
                () = cancel.cancelled() => break 'msg_loop,
                _ = state.service_connections.send(&session.service_id, reply) => {}
            }
        }

        match response.action {
            ProcessorAction::Continue => {}
            ProcessorAction::Break | ProcessorAction::CloseWithReason(_) => {
                tracing::info!(
                    service_id = %session.service_id,
                    app_name = session.app_name,
                    "embedded message handler stopping (processor requested break)"
                );
                break 'msg_loop;
            }
        }
    }

    cleanup_embedded_service_session(
        &state,
        session.service_id,
        session.connection_id,
        session.app_name,
        has_workload_claims,
        session.service_tenant_id,
    )
    .await;

    tracing::debug!(
        service_id = %session.service_id,
        app_name = session.app_name,
        "embedded message handler exited"
    );
}

pub(super) async fn cleanup_embedded_service_session(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    connection_id: uuid::Uuid,
    _service_app_name: &str,
    has_workload_claims: bool,
    tenant_id: Option<uuid::Uuid>,
) {
    if has_workload_claims {
        workload::release_all_claims_on_disconnect(state, service_id).await;
    }

    if let Some(provider_id) = state
        .surface_proxy_deps
        .registry
        .provider_id_for_service(&service_id)
    {
        state
            .surface_proxy_deps
            .proxy
            .fail_in_flight_for_provider(&provider_id);
        if !state.shutdown_token.is_cancelled() {
            match tenant_id {
                Some(tid) => {
                    state
                        .notification
                        .event_broadcaster
                        .send(tid, AdminEvent::SurfacesChanged)
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
    state
        .surface_proxy_deps
        .registry
        .unregister_service(&service_id);

    // Race-safe: only remove if this cleanup still owns the current registration.
    if state
        .service_connections
        .unregister_current(&service_id, connection_id)
        .await
    {
        if let Some(ref notifier) = state.embedded_service_notifier {
            notifier.on_external_disconnected(&service_id);
        }
    } else {
        tracing::debug!(
            %service_id,
            %connection_id,
            "embedded cleanup skipped — connection already replaced by a reconnect"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    use uptrakit_web_api_types::events::AdminEvent;

    #[cfg(feature = "db-sqlite")]
    mod db_sqlite {
        use super::super::super::test_support::*;
        use super::*;
        use std::collections::{BTreeMap, BTreeSet};
        use std::sync::Arc;

        use tokio_util::sync::CancellationToken;
        use uptrakit_wire::Capability;
        use uuid::Uuid;

        #[tokio::test]
        async fn embedded_system_handler_cleanup_releases_claims_and_unregisters_state() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
            let notifier = Arc::new(MockEmbeddedNotifier::default());
            let state = Arc::new(AppState {
                embedded_service_notifier: Some(notifier.clone()),
                ..(*state).clone()
            });

            let service_id = Uuid::now_v7();
            let mqtt_capabilities: BTreeSet<Capability> = [
                Capability::SystemService,
                Capability::UiSurfaces,
                Capability::WorkloadClaims,
            ]
            .into_iter()
            .collect();
            let (_, connection_handle) = state
                .service_connections
                .register(
                    service_id,
                    mqtt_capabilities.clone(),
                    None,
                    None,
                    Some("uptrakit-mqtt".to_string()),
                )
                .await;
            let connection_id = connection_handle.connection_id();

            state
                .surface_proxy_deps
                .registry
                .register_service(
                    service_id,
                    "uptrakit-mqtt",
                    Some(tenant_id),
                    test_surface_registration("service.provider-mqtt", tenant_id),
                )
                .expect("service surface registration should succeed");

            let claim_key = format!("clients.{}", Uuid::now_v7());
            let claim_result = state.workload_claim_registry.try_claim(
                service_id,
                state.controller_id,
                BTreeMap::from([(claim_key.clone(), tenant_id)]),
            );
            assert!(claim_result.granted.contains(&claim_key));
            assert!(state.service_connections.is_connected(&service_id).await);
            assert_eq!(
                state
                    .surface_proxy_deps
                    .registry
                    .provider_id_for_service(&service_id),
                Some("service.provider-mqtt".to_string())
            );

            let (service_tx, service_rx) = tokio::sync::mpsc::channel(1);
            drop(service_tx);

            run_embedded_system_message_handler(EmbeddedHandlerCallParams {
                state: state.clone(),
                service_id,
                connection_id,
                capabilities: &mqtt_capabilities,
                app_name: "uptrakit-mqtt",
                service_rx,
                cancel: CancellationToken::new(),
            })
            .await;

            assert!(!state.service_connections.is_connected(&service_id).await);
            assert!(
                state
                    .surface_proxy_deps
                    .registry
                    .provider_id_for_service(&service_id)
                    .is_none()
            );
            assert!(
                state
                    .workload_claim_registry
                    .service_claims(service_id)
                    .is_empty()
            );
            assert_eq!(*notifier.disconnected.lock(), vec![service_id]);
        }
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn cleanup_embedded_session_stale_id_does_not_evict_replacement() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        let service_id = uuid::Uuid::now_v7();

        // Register A, capture its id; then B supersedes A.
        let (_rx_a, handle_a) = state
            .service_connections
            .register(
                service_id,
                std::collections::BTreeSet::new(),
                None,
                None,
                None,
            )
            .await;
        let stale_id = handle_a.connection_id();
        let (_rx_b, _handle_b) = state
            .service_connections
            .register(
                service_id,
                std::collections::BTreeSet::new(),
                None,
                None,
                None,
            )
            .await;

        // A's cleanup runs with A's (now stale) id — must not evict B.
        cleanup_embedded_service_session(
            &state,
            service_id,
            stale_id,
            "uptrakit-agent",
            false,
            Some(tenant_id),
        )
        .await;

        assert!(
            state.service_connections.is_connected(&service_id).await,
            "replacement B must survive embedded cleanup with a stale connection_id"
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn cleanup_embedded_session_broadcasts_surfaces_changed_when_tenant_present() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        register_test_runtime_state(&state, service_id, tenant_id);

        let mut rx = state
            .notification
            .event_broadcaster
            .subscribe(tenant_id)
            .await;

        cleanup_embedded_service_session(
            &state,
            service_id,
            uuid::Uuid::now_v7(),
            "uptrakit-agent-ssh",
            false,
            Some(tenant_id),
        )
        .await;

        match rx.try_recv() {
            Ok(AdminEvent::SurfacesChanged) => {}
            other => panic!("expected SurfacesChanged, got {other:?}"),
        }
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn cleanup_embedded_session_broadcasts_globally_when_no_tenant_id() {
        // An untenanted (system-service) provider has no tenant channel of its
        // own, so cleanup must fall back to `send_global` — otherwise the UI
        // for every tenant would silently never learn the surface was removed.
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        register_test_runtime_state(&state, service_id, tenant_id);

        // `send_global` fans out to every locally subscribed channel
        // regardless of key, so subscribing under any tenant is sufficient
        // to observe the broadcast.
        let mut rx = state
            .notification
            .event_broadcaster
            .subscribe(tenant_id)
            .await;

        cleanup_embedded_service_session(
            &state,
            service_id,
            uuid::Uuid::now_v7(),
            "uptrakit-agent-ssh",
            false,
            None, // system service — no tenant
        )
        .await;

        match rx.try_recv() {
            Ok(AdminEvent::SurfacesChanged) => {}
            other => panic!("expected SurfacesChanged via send_global, got {other:?}"),
        }
    }
}
