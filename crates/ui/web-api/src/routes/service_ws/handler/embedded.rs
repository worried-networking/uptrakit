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
use super::shared_types::ProcessorAction;
use super::{load_session_host_ids, workload};
use crate::AppState;

// ---------------------------------------------------------------------------
// Embedded service message handler
// ---------------------------------------------------------------------------

/// Run a message handler loop for an embedded service.
///
/// This creates a [`MessageProcessor`] configured for an embedded (in-process)
/// service and reads messages from the provided channel. Replies are pushed
/// back through the [`ServiceConnectionRegistry`].
///
/// Used by `embedded_support::run_embedded_message_handler`.
pub(crate) async fn run_embedded_message_handler(
    state: Arc<AppState>,
    service_id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    capabilities: &BTreeSet<Capability>,
    app_name: &str,
    service_rx: tokio::sync::mpsc::Receiver<ServiceMessage>,
    cancel: tokio_util::sync::CancellationToken,
) {
    run_embedded_session(
        state,
        service_id,
        false,
        Some(tenant_id),
        capabilities,
        app_name,
        service_rx,
        cancel,
    )
    .await;
}

pub(crate) async fn run_embedded_system_message_handler(
    state: Arc<AppState>,
    service_id: uuid::Uuid,
    service_tenant_id: Option<uuid::Uuid>,
    capabilities: &BTreeSet<Capability>,
    app_name: &str,
    service_rx: tokio::sync::mpsc::Receiver<ServiceMessage>,
    cancel: tokio_util::sync::CancellationToken,
) {
    run_embedded_session(
        state,
        service_id,
        true,
        service_tenant_id,
        capabilities,
        app_name,
        service_rx,
        cancel,
    )
    .await;
}

/// Shared dispatch path for both embedded handler entry points.
///
/// Constructs the [`EmbeddedHandlerSession`] from the resolved `is_system` flag
/// and `service_tenant_id`, then delegates to [`run_embedded_message_handler_inner`].
#[expect(
    clippy::too_many_arguments,
    reason = "consolidates two identical 7-arg wrapper bodies; struct overhead not warranted for a private helper"
)]
async fn run_embedded_session(
    state: Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
    service_tenant_id: Option<uuid::Uuid>,
    capabilities: &BTreeSet<Capability>,
    app_name: &str,
    service_rx: tokio::sync::mpsc::Receiver<ServiceMessage>,
    cancel: tokio_util::sync::CancellationToken,
) {
    run_embedded_message_handler_inner(
        state,
        EmbeddedHandlerSession {
            service_id,
            is_system,
            service_tenant_id,
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
        if let Some(tid) = tenant_id
            && !state.shutdown_token.is_cancelled()
        {
            state
                .notification
                .event_broadcaster
                .send(tid, AdminEvent::SurfacesChanged)
                .await;
        }
    }
    state
        .surface_proxy_deps
        .registry
        .unregister_service(&service_id);

    state.service_connections.unregister(&service_id).await;

    if let Some(ref notifier) = state.embedded_service_notifier {
        notifier.on_external_disconnected(&service_id);
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

        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        use tokio_util::sync::CancellationToken;
        use uptrakit_shared_db::entity::{service_host, update_history};
        use uptrakit_wire::Capability;
        use uuid::Uuid;

        use super::super::super::updates;

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
            let _ = state
                .service_connections
                .register(
                    service_id,
                    mqtt_capabilities.clone(),
                    None,
                    None,
                    Some("uptrakit-mqtt".to_string()),
                )
                .await;

            state
                .surface_proxy_deps
                .registry
                .register_service(
                    service_id,
                    "uptrakit-mqtt",
                    Some(tenant_id),
                    test_surface_registration("provider-mqtt", tenant_id),
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
                Some("provider-mqtt".to_string())
            );

            let (service_tx, service_rx) = tokio::sync::mpsc::channel(1);
            drop(service_tx);

            run_embedded_system_message_handler(
                state.clone(),
                service_id,
                Some(tenant_id),
                &mqtt_capabilities,
                "uptrakit-mqtt",
                service_rx,
                CancellationToken::new(),
            )
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

        #[tokio::test]
        async fn reconnect_cleanup_same_instance_leaves_owned_update_in_progress() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
            let service_id = Uuid::now_v7();
            let runtime_id = Uuid::now_v7();
            let capabilities: BTreeSet<Capability> =
                [Capability::SoftwareDiscovery, Capability::UpdateHooks]
                    .into_iter()
                    .collect();
            let (host_id, software_item_id) =
                insert_linked_host_and_item(state.db(), tenant_id, service_id).await;
            let update_history_id = insert_owned_in_progress_update(
                state.db(),
                tenant_id,
                host_id,
                software_item_id,
                service_id,
                Some(runtime_id),
            )
            .await;

            run_embedded_register_once(
                Arc::clone(&state),
                service_id,
                tenant_id,
                capabilities,
                runtime_id,
            )
            .await;

            let row = update_history::Entity::find_by_id(update_history_id)
                .one(state.db())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.status, update_history::UpdateStatus::InProgress);
        }

        #[tokio::test]
        async fn reconnect_cleanup_new_instance_fails_prior_owned_update_even_without_host_links() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
            let service_id = Uuid::now_v7();
            let old_runtime_id = Uuid::now_v7();
            let new_runtime_id = Uuid::now_v7();
            let capabilities: BTreeSet<Capability> =
                [Capability::SoftwareDiscovery, Capability::UpdateHooks]
                    .into_iter()
                    .collect();
            let (host_id, software_item_id) =
                insert_linked_host_and_item(state.db(), tenant_id, service_id).await;
            let update_history_id = insert_owned_in_progress_update(
                state.db(),
                tenant_id,
                host_id,
                software_item_id,
                service_id,
                Some(old_runtime_id),
            )
            .await;

            service_host::Entity::delete_many()
                .filter(service_host::Column::ServiceId.eq(service_id))
                .exec(state.db())
                .await
                .unwrap();

            run_embedded_register_once(
                Arc::clone(&state),
                service_id,
                tenant_id,
                capabilities,
                new_runtime_id,
            )
            .await;

            let row = update_history::Entity::find_by_id(update_history_id)
                .one(state.db())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.status, update_history::UpdateStatus::Interrupted);
            assert_eq!(
                row.output,
                "Update interrupted: agent restarted (outcome unknown)"
            );
        }

        #[tokio::test]
        async fn connect_phase_does_not_fail_update_owned_by_different_linked_service() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
            let owner_service_id = Uuid::now_v7();
            let reconnecting_service_id = Uuid::now_v7();
            let old_runtime_id = Uuid::now_v7();
            let new_runtime_id = Uuid::now_v7();
            let (host_id, software_item_id) =
                insert_linked_host_and_item(state.db(), tenant_id, owner_service_id).await;
            insert_service_row(
                state.db(),
                tenant_id,
                reconnecting_service_id,
                "uptrakit-agent",
            )
            .await;
            relink_service_host(state.db(), reconnecting_service_id, host_id).await;
            let update_history_id = insert_owned_in_progress_update(
                state.db(),
                tenant_id,
                host_id,
                software_item_id,
                owner_service_id,
                Some(old_runtime_id),
            )
            .await;

            updates::recover_owned_updates_on_connect_with_dispatch_mode(
                &state,
                reconnecting_service_id,
                Some(new_runtime_id),
                updates::ReconnectSuccessorDispatchMode::Immediate,
            )
            .await
            .unwrap();
            let _ = updates::load_pending_update_records(&state, reconnecting_service_id)
                .await
                .unwrap();

            let row = update_history::Entity::find_by_id(update_history_id)
                .one(state.db())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.status, update_history::UpdateStatus::InProgress);
        }
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
    async fn cleanup_embedded_session_skips_broadcast_when_no_tenant_id() {
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
            "uptrakit-agent-ssh",
            false,
            None, // system service — no tenant
        )
        .await;

        assert!(
            rx.try_recv().is_err(),
            "no broadcast expected for system service"
        );
    }
}
