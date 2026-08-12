use super::shared_types::{is_valid_service_config_scope, system_service_tenant_binding};
use super::test_support::*;

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use uptrakit_plugin_infrastructure_core::testing::instance_surface_fixture as fixture;
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_wire::limits::MAX_SHORT_STRING_LEN;
use uptrakit_wire::report_tracker::ReportTracker;
use uptrakit_wire::surfaces;
use uptrakit_wire::{Capability, ControllerMessage, ErrorCode, ServiceMessage};
use uuid::Uuid;

use super::message_processor::MessageProcessor;
use super::shared_types::ProcessorAction;

#[test]
fn system_service_tenant_binding_only_targets_mqtt() {
    let tenant_id = uuid::Uuid::now_v7();
    assert_eq!(
        system_service_tenant_binding(Some("uptrakit-mqtt"), tenant_id),
        Some(tenant_id)
    );
    assert_eq!(
        system_service_tenant_binding(Some("uptrakit-scheduler"), tenant_id),
        None
    );
    assert_eq!(system_service_tenant_binding(None, tenant_id), None);
}

#[test]
fn service_config_scope_validation_requires_exact_tenant_for_bound_sessions() {
    let tenant_id = uuid::Uuid::now_v7();
    assert!(is_valid_service_config_scope(
        Some(tenant_id),
        Some(tenant_id)
    ));
    assert!(!is_valid_service_config_scope(Some(tenant_id), None));
    assert!(!is_valid_service_config_scope(
        Some(tenant_id),
        Some(uuid::Uuid::now_v7())
    ));
    assert!(is_valid_service_config_scope(None, None));
    assert!(is_valid_service_config_scope(None, Some(tenant_id)));
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn store_service_config_scope_violation_emits_denied_tenant_audit_row() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
    let service_id = Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-mqtt").await;
    let mut processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id,
        cert: None,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: false,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-mqtt".to_string()),
        service_tenant_id: Some(tenant_id),
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };

    let response = processor
        .dispatch(
            ServiceMessage::StoreServiceConfig(uptrakit_wire::StoreServiceConfigPayload::new(
                "req-store-denied".to_string(),
                None,
                "clients.primary".to_string(),
                serde_json::json!({"enabled": true}),
                true,
            )),
            None,
        )
        .await;

    let [ControllerMessage::ServiceConfigAck(ack)] = response.replies.as_slice() else {
        panic!("expected exactly one ServiceConfigAck reply");
    };
    assert_eq!(ack.request_id, "req-store-denied");
    assert!(!ack.success);
    assert_eq!(
        ack.error.as_deref(),
        Some("service cannot write config outside its tenant binding")
    );

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SERVICE_CONFIG_STORE,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(service_id));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    assert_eq!(row.request_id.as_deref(), Some("req-store-denied"));
    assert_eq!(row.target_type.as_deref(), Some("service_config"));
    assert_eq!(row.target_display.as_deref(), Some("clients.primary"));
    let details = row
        .details_json
        .as_ref()
        .expect("scope denial audit should include details");
    assert_eq!(details["service_app_name"], "uptrakit-mqtt");
    assert_eq!(details["requested_scope"], "global");
    assert_eq!(details["service_tenant_id"], tenant_id.to_string());
    assert_eq!(details["requested_tenant_id"], serde_json::Value::Null);
    assert_eq!(details["reason_code"], "outside_tenant_binding");
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn delete_service_config_scope_violation_emits_denied_tenant_audit_row() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
    let service_id = Uuid::now_v7();
    let requested_tenant_id = Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-mqtt").await;
    let mut processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id,
        cert: None,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: false,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-mqtt".to_string()),
        service_tenant_id: Some(tenant_id),
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };

    let response = processor
        .dispatch(
            ServiceMessage::DeleteServiceConfig(uptrakit_wire::DeleteServiceConfigPayload::new(
                "req-delete-denied".to_string(),
                Some(requested_tenant_id),
                "clients.primary".to_string(),
            )),
            None,
        )
        .await;

    let [ControllerMessage::ServiceConfigAck(ack)] = response.replies.as_slice() else {
        panic!("expected exactly one ServiceConfigAck reply");
    };
    assert_eq!(ack.request_id, "req-delete-denied");
    assert!(!ack.success);
    assert_eq!(
        ack.error.as_deref(),
        Some("service cannot delete config outside its tenant binding")
    );

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SERVICE_CONFIG_DELETE,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(service_id));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    assert_eq!(row.request_id.as_deref(), Some("req-delete-denied"));
    assert_eq!(row.target_type.as_deref(), Some("service_config"));
    assert_eq!(row.target_display.as_deref(), Some("clients.primary"));
    let details = row
        .details_json
        .as_ref()
        .expect("scope denial audit should include details");
    assert_eq!(details["service_app_name"], "uptrakit-mqtt");
    assert_eq!(details["requested_scope"], "tenant");
    assert_eq!(details["service_tenant_id"], tenant_id.to_string());
    assert_eq!(
        details["requested_tenant_id"],
        requested_tenant_id.to_string()
    );
    assert_eq!(details["reason_code"], "outside_tenant_binding");
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn surface_action_scope_violation_emits_denied_tenant_audit_row() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let requested_tenant_id = Uuid::now_v7();
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
    let service_id = Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-mqtt").await;
    let processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id,
        cert: None,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: false,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-mqtt".to_string()),
        service_tenant_id: Some(tenant_id),
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };
    let request_id = Uuid::now_v7();

    let response = processor
        .handle_surface_action_request(surfaces::SurfaceActionRequest {
            request_id,
            tenant_id: requested_tenant_id.to_string(),
            surface_id: surfaces::SurfaceId::new("notifications.email").unwrap(),
            interaction_id: surfaces::InteractionId::new("smtp").unwrap(),
            method: Default::default(),
            idempotency_key: "scope-violation".to_string(),
            target_provider_id: Some("service.provider-a".to_string()),
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "service.provider-a".to_string(),
            },
            params: serde_json::Map::from_iter([(
                "host".to_string(),
                serde_json::Value::String("smtp.example.invalid".to_string()),
            )]),
            encrypted_sensitive_params: None,
        })
        .await;

    let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
        panic!("expected exactly one SurfaceActionResponse reply");
    };
    assert_eq!(reply.request_id, request_id);
    assert!(!reply.success);
    let error = reply.error.as_ref().expect("error payload should exist");
    assert_eq!(
        error.code,
        surfaces::SurfaceActionErrorCode::PermissionDenied
    );
    assert_eq!(
        error.message,
        "service cannot invoke actions outside its tenant"
    );

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(service_id));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    let request_id_string = request_id.to_string();
    assert_eq!(row.request_id.as_deref(), Some(request_id_string.as_str()));
    assert_eq!(row.target_type.as_deref(), Some("surface_action"));
    assert_eq!(row.target_id, None);
    assert_eq!(
        row.target_display.as_deref(),
        Some("notifications.email/smtp")
    );
    let details = row
        .details_json
        .as_ref()
        .expect("scope denial audit should include details");
    assert_eq!(details["service_app_name"], "uptrakit-mqtt");
    assert_eq!(details["surface_id"], "notifications.email");
    assert_eq!(details["interaction_id"], "smtp");
    assert_eq!(details["target_provider_id"], "service.provider-a");
    assert_eq!(details["service_tenant_id"], tenant_id.to_string());
    assert_eq!(
        details["requested_tenant_id"],
        requested_tenant_id.to_string()
    );
    assert_eq!(details["reason_code"], "outside_tenant_binding");
    assert!(details.get("params").is_none());
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn surface_action_invalid_payload_emits_validation_failed_tenant_audit_row() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
    let service_id = Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
    let processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id,
        cert: None,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: true,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-agent-ssh".to_string()),
        service_tenant_id: Some(tenant_id),
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };
    let request_id = Uuid::now_v7();

    let response = processor
        .handle_surface_action_request(surfaces::SurfaceActionRequest {
            request_id,
            tenant_id: tenant_id.to_string(),
            surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
            interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
            method: Default::default(),
            idempotency_key: "x".repeat(MAX_SHORT_STRING_LEN + 1),
            target_provider_id: Some("service.provider-a".to_string()),
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "service.provider-a".to_string(),
            },
            params: serde_json::Map::new(),
            encrypted_sensitive_params: None,
        })
        .await;

    let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
        panic!("expected exactly one SurfaceActionResponse reply");
    };
    assert_eq!(reply.request_id, request_id);
    assert!(!reply.success);
    let error = reply.error.as_ref().expect("error payload should exist");
    assert_eq!(error.code, surfaces::SurfaceActionErrorCode::InvalidRequest);

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(service_id));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
    );
    let request_id_string = request_id.to_string();
    assert_eq!(row.request_id.as_deref(), Some(request_id_string.as_str()));
    assert_eq!(row.target_type.as_deref(), Some("surface_action"));
    assert_eq!(
        row.target_display.as_deref(),
        Some("ssh.guest.panel/refresh")
    );
    let details = row
        .details_json
        .as_ref()
        .expect("invalid payload audit should include details");
    assert_eq!(details["surface_id"], "ssh.guest.panel");
    assert_eq!(details["interaction_id"], "refresh");
    assert_eq!(details["target_provider_id"], "service.provider-a");
    assert_eq!(details["reason_code"], "invalid_request");
    assert!(details.get("params").is_none());
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn surface_action_invalid_tenant_emits_validation_failed_tenant_audit_row() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
    let service_id = Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
    let processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id,
        cert: None,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: true,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-agent-ssh".to_string()),
        service_tenant_id: Some(tenant_id),
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };
    let request_id = Uuid::now_v7();

    let response = processor
        .handle_surface_action_request(surfaces::SurfaceActionRequest {
            request_id,
            tenant_id: "not-a-uuid".to_string(),
            surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
            interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
            method: Default::default(),
            idempotency_key: "invalid-tenant".to_string(),
            target_provider_id: Some("service.provider-a".to_string()),
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "service.provider-a".to_string(),
            },
            params: serde_json::Map::new(),
            encrypted_sensitive_params: None,
        })
        .await;

    let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
        panic!("expected exactly one SurfaceActionResponse reply");
    };
    assert_eq!(reply.request_id, request_id);
    assert!(!reply.success);
    let error = reply.error.as_ref().expect("error payload should exist");
    assert_eq!(error.code, surfaces::SurfaceActionErrorCode::InvalidRequest);

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(service_id));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
    );
    let request_id_string = request_id.to_string();
    assert_eq!(row.request_id.as_deref(), Some(request_id_string.as_str()));
    let details = row
        .details_json
        .as_ref()
        .expect("invalid tenant audit should include details");
    assert_eq!(details["surface_id"], "ssh.guest.panel");
    assert_eq!(details["interaction_id"], "refresh");
    assert_eq!(details["target_provider_id"], "service.provider-a");
    assert_eq!(details["reason_code"], "invalid_tenant_id");
    assert!(details.get("params").is_none());
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn surface_action_lookup_failure_emits_validation_failed_tenant_audit_row() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let state = build_db_audited_state(db.clone(), tenant_id).await;
    let service_id = Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
    let mut registration = test_surface_registration("service.provider-a", tenant_id);
    registration.surfaces[0].interactions[0].required_action = None;
    state
        .surface_proxy_deps
        .registry
        .register_service(
            service_id,
            "uptrakit-agent-ssh",
            Some(tenant_id),
            registration,
        )
        .expect("surface registration should succeed");
    let processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id,
        cert: None,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: true,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-agent-ssh".to_string()),
        service_tenant_id: Some(tenant_id),
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };
    let request_id = Uuid::now_v7();

    let response = processor
        .handle_surface_action_request(surfaces::SurfaceActionRequest {
            request_id,
            tenant_id: tenant_id.to_string(),
            surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
            interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
            method: Default::default(),
            idempotency_key: "lookup-failure".to_string(),
            target_provider_id: Some("missing-provider".to_string()),
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "service.provider-a".to_string(),
            },
            params: serde_json::Map::new(),
            encrypted_sensitive_params: None,
        })
        .await;

    let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
        panic!("expected exactly one SurfaceActionResponse reply");
    };
    assert_eq!(reply.request_id, request_id);
    assert!(!reply.success);
    let error = reply.error.as_ref().expect("error payload should exist");
    assert_eq!(error.code, surfaces::SurfaceActionErrorCode::InvalidRequest);

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(service_id));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
    );
    let request_id_string = request_id.to_string();
    assert_eq!(row.request_id.as_deref(), Some(request_id_string.as_str()));
    let details = row
        .details_json
        .as_ref()
        .expect("lookup failure audit should include details");
    assert_eq!(details["surface_id"], "ssh.guest.panel");
    assert_eq!(details["interaction_id"], "refresh");
    assert_eq!(details["target_provider_id"], "missing-provider");
    assert_eq!(details["reason_code"], "invalid_provider");
    assert!(details.get("provider_kind").is_none());
    assert!(details.get("params").is_none());
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn surface_action_success_emits_success_tenant_audit_row() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let state = build_db_audited_state(db.clone(), tenant_id).await;
    let service_id = Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
    let mut registration = test_surface_registration("service.provider-a", tenant_id);
    registration.surfaces[0].interactions[0].required_action = None;
    // Routing/audit-focused test: fully ungated fixture. The gated-descriptor
    // provider-origin class is covered by the descriptor-gate tests.
    registration.surfaces[0].descriptor.required_action = None;
    state
        .surface_proxy_deps
        .registry
        .register_service(
            service_id,
            "uptrakit-agent-ssh",
            Some(tenant_id),
            registration,
        )
        .expect("surface registration should succeed");
    let (mut rx, _cancel) = state
        .service_connections
        .register(
            service_id,
            BTreeSet::from([Capability::UiSurfaces]),
            None,
            None,
            Some("uptrakit-agent-ssh".to_string()),
        )
        .await;
    let proxy = Arc::clone(&state.surface_proxy_deps.proxy);
    tokio::spawn(async move {
        if let Some(ControllerMessage::SurfaceActionRequest(request)) = rx.recv().await {
            proxy.complete(
                request.request_id,
                surfaces::SurfaceActionResponse {
                    request_id: request.request_id,
                    success: true,
                    result: Some(serde_json::json!({"ok": true})),
                    error: None,
                },
            );
        }
    });
    let processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id,
        cert: None,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: true,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-agent-ssh".to_string()),
        service_tenant_id: Some(tenant_id),
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };
    let request_id = Uuid::now_v7();

    let response = processor
        .handle_surface_action_request(surfaces::SurfaceActionRequest {
            request_id,
            tenant_id: tenant_id.to_string(),
            surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
            interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
            method: Default::default(),
            idempotency_key: "surface-success".to_string(),
            target_provider_id: Some("service.provider-a".to_string()),
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "service.provider-a".to_string(),
            },
            params: serde_json::Map::new(),
            encrypted_sensitive_params: None,
        })
        .await;

    let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
        panic!("expected exactly one SurfaceActionResponse reply");
    };
    assert_eq!(reply.request_id, request_id);
    assert!(reply.success);

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(service_id));
    assert_eq!(row.actor_display.as_deref(), Some("uptrakit-agent-ssh"));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    let request_id_string = request_id.to_string();
    assert_eq!(row.request_id.as_deref(), Some(request_id_string.as_str()));
    assert_eq!(row.target_type.as_deref(), Some("surface_action"));
    assert_eq!(
        row.target_display.as_deref(),
        Some("ssh.guest.panel/refresh")
    );
    let details = row
        .details_json
        .as_ref()
        .expect("success audit should include details");
    assert_eq!(details["surface_id"], "ssh.guest.panel");
    assert_eq!(details["interaction_id"], "refresh");
    assert_eq!(details["target_provider_id"], "service.provider-a");
    assert_eq!(details["provider_kind"], "service");
    assert_eq!(details["provider_service_app_name"], "uptrakit-agent-ssh");
    assert!(details.get("reason_code").is_none());
    assert!(details.get("params").is_none());
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn surface_action_provider_unavailable_emits_failed_tenant_audit_row() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let state = build_db_audited_state(db.clone(), tenant_id).await;
    let service_id = Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
    let mut registration = test_surface_registration("service.provider-a", tenant_id);
    registration.surfaces[0].interactions[0].required_action = None;
    // Routing/audit-focused test: fully ungated fixture. The gated-descriptor
    // provider-origin class is covered by the descriptor-gate tests.
    registration.surfaces[0].descriptor.required_action = None;
    state
        .surface_proxy_deps
        .registry
        .register_service(
            service_id,
            "uptrakit-agent-ssh",
            Some(tenant_id),
            registration,
        )
        .expect("surface registration should succeed");
    let (rx, _cancel) = state
        .service_connections
        .register(
            service_id,
            BTreeSet::from([Capability::UiSurfaces]),
            None,
            None,
            Some("uptrakit-agent-ssh".to_string()),
        )
        .await;
    drop(rx);
    let processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id,
        cert: None,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: true,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-agent-ssh".to_string()),
        service_tenant_id: Some(tenant_id),
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };
    let request_id = Uuid::now_v7();

    let response = processor
        .handle_surface_action_request(surfaces::SurfaceActionRequest {
            request_id,
            tenant_id: tenant_id.to_string(),
            surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
            interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
            method: Default::default(),
            idempotency_key: "surface-provider-unavailable".to_string(),
            target_provider_id: Some("service.provider-a".to_string()),
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "service.provider-a".to_string(),
            },
            params: serde_json::Map::new(),
            encrypted_sensitive_params: None,
        })
        .await;

    let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
        panic!("expected exactly one SurfaceActionResponse reply");
    };
    assert_eq!(reply.request_id, request_id);
    assert!(!reply.success);
    let error = reply.error.as_ref().expect("error payload should exist");
    assert_eq!(
        error.code,
        surfaces::SurfaceActionErrorCode::ProviderUnavailable
    );

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(service_id));
    assert_eq!(row.actor_display.as_deref(), Some("uptrakit-agent-ssh"));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Failed.as_str()
    );
    let request_id_string = request_id.to_string();
    assert_eq!(row.request_id.as_deref(), Some(request_id_string.as_str()));
    let details = row
        .details_json
        .as_ref()
        .expect("failed audit should include details");
    assert_eq!(details["surface_id"], "ssh.guest.panel");
    assert_eq!(details["interaction_id"], "refresh");
    assert_eq!(details["target_provider_id"], "service.provider-a");
    assert_eq!(details["provider_kind"], "service");
    assert_eq!(details["provider_service_app_name"], "uptrakit-agent-ssh");
    assert_eq!(details["reason_code"], "provider_unavailable");
    assert!(details.get("params").is_none());
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn invalid_surface_registration_emits_validation_failed_tenant_audit_row() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let state = build_db_audited_state(db.clone(), tenant_id).await;
    let service_id = Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
    let processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id,
        cert: None,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: true,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-agent-ssh".to_string()),
        service_tenant_id: Some(tenant_id),
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };
    let mut registration = test_surface_registration("service.provider-a", tenant_id);
    registration.effective_tenant_binding.tenant_id = None;

    let response = processor.handle_surface_registration(registration).await;

    let [ControllerMessage::Error(reply)] = response.replies.as_slice() else {
        panic!("expected exactly one Error reply");
    };
    assert_eq!(reply.code, ErrorCode::BadRequest);
    assert!(reply.message.contains("invalid surface registration"));

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SURFACE_PROVIDER_REGISTER,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(service_id));
    assert_eq!(row.actor_display.as_deref(), Some("uptrakit-agent-ssh"));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("surface_provider"));
    assert_eq!(row.target_id.as_deref(), Some("service.provider-a"));
    assert_eq!(row.target_display.as_deref(), Some("service.provider-a"));
    let details = row
        .details_json
        .as_ref()
        .expect("validation failure audit should include details");
    assert_eq!(details["provider_id"], "service.provider-a");
    assert_eq!(details["provider_kind"], "service");
    assert_eq!(details["framework_generation"], "1.0");
    assert_eq!(details["capability_count"], 4);
    assert_eq!(details["surface_count"], 1);
    assert_eq!(details["reason_code"], "invalid_tenant_binding");
    assert!(details.get("surfaces").is_none());
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn incompatible_surface_registration_emits_denied_tenant_audit_row() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let state = build_db_audited_state(db.clone(), tenant_id).await;
    let service_id = Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
    let processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id,
        cert: None,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: true,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-agent-ssh".to_string()),
        service_tenant_id: Some(tenant_id),
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };
    let mut registration = test_surface_registration("service.provider-a", tenant_id);
    registration.framework_generation = surfaces::FrameworkGeneration::new(2, 0);

    let response = processor.handle_surface_registration(registration).await;

    let [ControllerMessage::Error(reply)] = response.replies.as_slice() else {
        panic!("expected exactly one Error reply");
    };
    assert_eq!(reply.code, ErrorCode::BadRequest);
    assert!(reply.message.contains("UnsupportedGeneration"));

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SURFACE_PROVIDER_REGISTER,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(service_id));
    assert_eq!(row.actor_display.as_deref(), Some("uptrakit-agent-ssh"));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("surface_provider"));
    assert_eq!(row.target_id.as_deref(), Some("service.provider-a"));
    assert_eq!(row.target_display.as_deref(), Some("service.provider-a"));
    let details = row
        .details_json
        .as_ref()
        .expect("rejection audit should include details");
    assert_eq!(details["provider_id"], "service.provider-a");
    assert_eq!(details["provider_kind"], "service");
    assert_eq!(details["framework_generation"], "2.0");
    assert_eq!(details["capability_count"], 4);
    assert_eq!(details["surface_count"], 1);
    assert_eq!(details["reason_code"], "unsupported_generation");
    assert!(details.get("surfaces").is_none());
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn successful_system_surface_registration_emits_success_system_audit_row() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let state = build_db_audited_state(db.clone(), tenant_id).await;
    let service_id = Uuid::now_v7();
    insert_test_system_service_row(&db, service_id, "uptrakit-scheduler").await;
    let processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id,
        cert: None,
        is_system: true,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: true,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-scheduler".to_string()),
        service_tenant_id: None,
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };
    let mut registration = test_surface_registration("service.provider-system", tenant_id);
    registration.effective_tenant_binding.scope = surfaces::Scope::Global;
    registration.effective_tenant_binding.tenant_id = None;

    let response = processor.handle_surface_registration(registration).await;

    assert!(response.replies.is_empty());
    assert!(matches!(response.action, ProcessorAction::Continue));

    let row = system_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SURFACE_PROVIDER_REGISTER,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(service_id));
    assert_eq!(row.actor_display.as_deref(), Some("uptrakit-scheduler"));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("surface_provider"));
    assert_eq!(row.target_id.as_deref(), Some("service.provider-system"));
    assert_eq!(
        row.target_display.as_deref(),
        Some("service.provider-system")
    );
    let details = row
        .details_json
        .as_ref()
        .expect("success audit should include details");
    assert_eq!(details["provider_id"], "service.provider-system");
    assert_eq!(details["provider_kind"], "service");
    assert_eq!(details["framework_generation"], "1.0");
    assert_eq!(details["capability_count"], 4);
    assert_eq!(details["surface_count"], 1);
    assert!(details.get("reason_code").is_none());
    assert!(details.get("surfaces").is_none());
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn surface_registration_success_broadcasts_surfaces_changed() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let state = build_db_audited_state(db.clone(), tenant_id).await;
    let service_id = uuid::Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;

    let mut rx = state
        .notification
        .event_broadcaster
        .subscribe(tenant_id)
        .await;

    let processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id,
        cert: None,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: true,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-agent-ssh".to_string()),
        service_tenant_id: Some(tenant_id),
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };
    let response = processor
        .handle_surface_registration(test_surface_registration("service.provider-a", tenant_id))
        .await;

    assert!(response.replies.is_empty(), "success path returns cont()");

    match rx.try_recv() {
        Ok(AdminEvent::SurfacesChanged) => {}
        other => panic!("expected SurfacesChanged on success, got {other:?}"),
    }
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn surface_registration_rejection_does_not_broadcast() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let state = build_db_audited_state(db.clone(), tenant_id).await;
    let service_id = uuid::Uuid::now_v7();
    let service_id_b = uuid::Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
    insert_test_service_row(&db, tenant_id, service_id_b, "uptrakit-agent-ssh-2").await;

    // Register service.provider-a from service_id (succeeds).
    state
        .surface_proxy_deps
        .registry
        .register_service(
            service_id,
            "uptrakit-agent-ssh",
            Some(tenant_id),
            test_surface_registration("service.provider-a", tenant_id),
        )
        .expect("first registration should succeed");

    let mut rx = state
        .notification
        .event_broadcaster
        .subscribe(tenant_id)
        .await;

    // Try to claim the SAME provider ID ("service.provider-a") from a different service
    // (service_id_b). The registry rejects this because service.provider-a is already bound
    // to service_id — two different services cannot share the same provider ID.
    let processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id: service_id_b,
        cert: None,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: true,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-agent-ssh-2".to_string()),
        service_tenant_id: Some(tenant_id),
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };
    let response = processor
        .handle_surface_registration(test_surface_registration("service.provider-a", tenant_id))
        .await;

    assert!(
        !response.replies.is_empty(),
        "rejection path returns an error reply"
    );
    assert!(
        rx.try_recv().is_err(),
        "no broadcast expected on rejected registration"
    );
}

// ---------------------------------------------------------------------------
// Provider-origin e2e coverage: real plugin (proxmox) surface actions invoked
// end to end through the shared-surface stack — bootstrap_plugin +
// PluginSurfaceLocalExecutor wired via
// `crate::test_harness::build_test_state_with_plugin_surfaces`. See
// `.superpowers/sdd/task-10-brief.md`.
// ---------------------------------------------------------------------------

/// Seed a `plugin_configs` row (`plugin_type = "infrastructure.proxmox"`) —
/// the FK parent required by `proxmox_host_mapping.plugin_config_id`.
#[cfg(feature = "db-sqlite")]
async fn insert_test_proxmox_plugin_config(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
) -> Uuid {
    use sea_orm::{ActiveModelTrait, Set};
    use uptrakit_shared_db::entity::plugin_config;

    // Tests never initialize a real master key; plaintext mode lets
    // `EncryptedPluginConfig::from_json` below work without one. Safe to call
    // repeatedly, and it must live in this helper so every caller reaches it
    // under a process-per-test runner.
    uptrakit_crypto::enable_plaintext_mode();
    let id = Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();
    plugin_config::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        name: Set("Test Proxmox Config".to_string()),
        plugin_type: Set("infrastructure.proxmox".to_string()),
        config: Set(
            uptrakit_shared_db::encrypted_columns::EncryptedPluginConfig::from_json(
                &serde_json::json!({}),
            )
            .expect("encrypt test config"),
        ),
        enabled: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
        credential_updated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert test plugin_config");
    id
}

/// Register the calling service as a surface provider so
/// `caller_origin_for_request` can resolve `SurfaceCallerOrigin::Provider {
/// service_id }` — required unconditionally for every `CallerOrigin::Provider`
/// invocation, even when the request also carries an explicit
/// `target_provider_id` for the real plugin provider.
#[cfg(feature = "db-sqlite")]
fn register_calling_service_as_provider(
    state: &Arc<crate::AppState>,
    service_id: Uuid,
    tenant_id: Uuid,
) {
    state
        .surface_proxy_deps
        .registry
        .register_service(
            service_id,
            "uptrakit-agent-ssh",
            Some(tenant_id),
            test_surface_registration("service.provider-a", tenant_id),
        )
        .expect("calling service should register as a surface provider");
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn provider_origin_denied_for_unflagged_permissioned_interaction() {
    let db = crate::test_harness::setup_migrated_db_with_plugins().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) =
        crate::test_harness::build_test_state_with_plugin_surfaces(db.clone(), tenant_id, None)
            .await;
    let service_id = Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
    register_calling_service_as_provider(&state, service_id, tenant_id);

    let processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id,
        cert: None,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: true,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-agent-ssh".to_string()),
        service_tenant_id: Some(tenant_id),
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };
    let request_id = Uuid::now_v7();

    // "discover" is a registered, permissioned (`UpdateHosts`) ControllerLocal
    // interaction on `proxmox.hosts` that is NOT `provider_invocable` — a
    // provider-origin caller must be denied even though the surface/interaction
    // both resolve successfully.
    let response = processor
        .handle_surface_action_request(surfaces::SurfaceActionRequest {
            request_id,
            tenant_id: tenant_id.to_string(),
            surface_id: surfaces::SurfaceId::new("proxmox.hosts").unwrap(),
            interaction_id: surfaces::InteractionId::new("discover").unwrap(),
            method: Default::default(),
            idempotency_key: "provider-origin-discover-denied".to_string(),
            target_provider_id: Some("infrastructure.proxmox".to_string()),
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "service.provider-a".to_string(),
            },
            params: serde_json::Map::new(),
            encrypted_sensitive_params: None,
        })
        .await;

    let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
        panic!("expected exactly one SurfaceActionResponse reply");
    };
    assert_eq!(reply.request_id, request_id);
    assert!(!reply.success);
    let error = reply.error.as_ref().expect("error payload should exist");
    assert_eq!(
        error.code,
        surfaces::SurfaceActionErrorCode::PermissionDenied
    );

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(service_id));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    let details = row
        .details_json
        .as_ref()
        .expect("permission denial audit should include details");
    assert_eq!(details["surface_id"], "proxmox.hosts");
    assert_eq!(details["interaction_id"], "discover");
    assert_eq!(details["reason_code"], "permission_denied");
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn provider_origin_unmatched_guests_executes_and_audits_service_actor() {
    let db = crate::test_harness::setup_migrated_db_with_plugins().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let plugin_config_id = insert_test_proxmox_plugin_config(&db, tenant_id).await;
    let mapping_id =
        uptrakit_plugin_infrastructure_proxmox::testing::insert_unmatched_host_mapping(
            &db,
            tenant_id,
            plugin_config_id,
            "pve-node-1",
            101,
            "qemu",
            "unmatched-guest",
        )
        .await;
    let (state, _jwt) =
        crate::test_harness::build_test_state_with_plugin_surfaces(db.clone(), tenant_id, None)
            .await;
    let service_id = Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
    register_calling_service_as_provider(&state, service_id, tenant_id);

    let processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id,
        cert: None,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: true,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-agent-ssh".to_string()),
        service_tenant_id: Some(tenant_id),
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };
    let request_id = Uuid::now_v7();

    let response = processor
        .handle_surface_action_request(surfaces::SurfaceActionRequest {
            request_id,
            tenant_id: tenant_id.to_string(),
            surface_id: surfaces::SurfaceId::new("proxmox.hosts").unwrap(),
            interaction_id: surfaces::InteractionId::new("unmatched-guests").unwrap(),
            method: Default::default(),
            idempotency_key: "provider-origin-unmatched-guests".to_string(),
            target_provider_id: Some("infrastructure.proxmox".to_string()),
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "service.provider-a".to_string(),
            },
            params: serde_json::Map::from_iter([(
                "per_page".to_string(),
                serde_json::Value::Number(1000.into()),
            )]),
            encrypted_sensitive_params: None,
        })
        .await;

    let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
        panic!("expected exactly one SurfaceActionResponse reply");
    };
    assert_eq!(reply.request_id, request_id);
    assert!(
        reply.success,
        "unmatched-guests should execute successfully: {:?}",
        reply.error
    );
    let result = reply
        .result
        .as_ref()
        .expect("success response has a result");
    let items = result["items"]
        .as_array()
        .expect("result.items should be an array");
    assert!(
        items
            .iter()
            .any(|item| item["mapping_id"] == mapping_id.to_string()),
        "seeded unmatched mapping should appear in result.items: {items:?}"
    );

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(service_id));
    assert_eq!(row.actor_display.as_deref(), Some("uptrakit-agent-ssh"));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
}

/// The real agent path: nested provider→plugin calls arrive with
/// `target_provider_id: None` (the agent cannot know the controller-side plugin's
/// provider id). Same as `provider_origin_unmatched_guests_executes_and_audits_service_actor`
/// but with `target_provider_id: None`, so implicit resolution — not a hardcoded
/// target — routes to `infrastructure.proxmox`.
#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn provider_origin_unmatched_guests_resolves_target_from_surface() {
    let db = crate::test_harness::setup_migrated_db_with_plugins().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let plugin_config_id = insert_test_proxmox_plugin_config(&db, tenant_id).await;
    let mapping_id =
        uptrakit_plugin_infrastructure_proxmox::testing::insert_unmatched_host_mapping(
            &db,
            tenant_id,
            plugin_config_id,
            "pve-node-1",
            103,
            "qemu",
            "unmatched-guest-implicit",
        )
        .await;
    let (state, _jwt) =
        crate::test_harness::build_test_state_with_plugin_surfaces(db.clone(), tenant_id, None)
            .await;
    let service_id = Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
    register_calling_service_as_provider(&state, service_id, tenant_id);

    let processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id,
        cert: None,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: true,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-agent-ssh".to_string()),
        service_tenant_id: Some(tenant_id),
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };
    let request_id = Uuid::now_v7();

    let response = processor
        .handle_surface_action_request(surfaces::SurfaceActionRequest {
            request_id,
            tenant_id: tenant_id.to_string(),
            surface_id: surfaces::SurfaceId::new("proxmox.hosts").unwrap(),
            interaction_id: surfaces::InteractionId::new("unmatched-guests").unwrap(),
            method: Default::default(),
            idempotency_key: "provider-origin-unmatched-guests-implicit".to_string(),
            target_provider_id: None,
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "service.provider-a".to_string(),
            },
            params: serde_json::Map::from_iter([(
                "per_page".to_string(),
                serde_json::Value::Number(1000.into()),
            )]),
            encrypted_sensitive_params: None,
        })
        .await;

    let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
        panic!("expected exactly one SurfaceActionResponse reply");
    };
    assert_eq!(reply.request_id, request_id);
    assert!(
        reply.success,
        "unmatched-guests with target=None should resolve and execute: {:?}",
        reply.error
    );
    let result = reply
        .result
        .as_ref()
        .expect("success response has a result");
    let items = result["items"]
        .as_array()
        .expect("result.items should be an array");
    assert!(
        items
            .iter()
            .any(|item| item["mapping_id"] == mapping_id.to_string()),
        "seeded unmatched mapping should appear in result.items: {items:?}"
    );

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(service_id));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn provider_origin_match_completes_handler() {
    let db = crate::test_harness::setup_migrated_db_with_plugins().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let plugin_config_id = insert_test_proxmox_plugin_config(&db, tenant_id).await;
    let mapping_id =
        uptrakit_plugin_infrastructure_proxmox::testing::insert_unmatched_host_mapping(
            &db,
            tenant_id,
            plugin_config_id,
            "pve-node-1",
            102,
            "qemu",
            "to-be-matched-guest",
        )
        .await;
    let host = crate::test_harness::fixtures::insert_host(&db, tenant_id).await;
    let (state, _jwt) =
        crate::test_harness::build_test_state_with_plugin_surfaces(db.clone(), tenant_id, None)
            .await;
    let service_id = Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
    register_calling_service_as_provider(&state, service_id, tenant_id);

    let processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id,
        cert: None,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: true,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-agent-ssh".to_string()),
        service_tenant_id: Some(tenant_id),
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };
    let request_id = Uuid::now_v7();

    let response = processor
        .handle_surface_action_request(surfaces::SurfaceActionRequest {
            request_id,
            tenant_id: tenant_id.to_string(),
            surface_id: surfaces::SurfaceId::new("proxmox.hosts").unwrap(),
            interaction_id: surfaces::InteractionId::new("match").unwrap(),
            method: Default::default(),
            idempotency_key: "provider-origin-match".to_string(),
            target_provider_id: Some("infrastructure.proxmox".to_string()),
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "service.provider-a".to_string(),
            },
            params: serde_json::Map::from_iter([
                ("mapping_id".to_string(), serde_json::json!(mapping_id)),
                ("host_id".to_string(), serde_json::json!(host.id)),
            ]),
            encrypted_sensitive_params: None,
        })
        .await;

    let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
        panic!("expected exactly one SurfaceActionResponse reply");
    };
    assert_eq!(reply.request_id, request_id);
    assert!(
        reply.success,
        "match should complete successfully: {:?}",
        reply.error
    );

    // Gate-pass is not the same as handler-runs (spec D7): confirm the
    // handler actually executed by re-reading the mapping row's `host_id`.
    let bound_host_id =
        uptrakit_plugin_infrastructure_proxmox::testing::host_mapping_host_id(&db, mapping_id)
            .await;
    assert_eq!(bound_host_id, Some(host.id));
}

// ---------------------------------------------------------------------------
// §5.4: provider-origin invocations are gated by LIVE effective enablement,
// not just boot state — driven by the synthetic Instance-scoped fixture
// plugin (`uptrakit_plugin_infrastructure_core::testing::instance_surface_fixture`).
// ---------------------------------------------------------------------------

/// ADR-0033 §5.4(a): a provider-origin invocation against a plugin that is
/// not effectively enabled (boot-enabled, no live row ⇒ disabled) is denied
/// by the proxy's stored filter — no `AuthenticatedUser` exists on this path.
#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn provider_origin_denied_for_live_disabled_plugin() {
    let db = crate::test_harness::setup_migrated_db_with_plugins().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state_with_plugin_surfaces(
        db.clone(),
        tenant_id,
        Some(crate::test_harness::synthetic_instance_catalog(true)),
    )
    .await;
    let service_id = Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
    register_calling_service_as_provider(&state, service_id, tenant_id);

    let processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id,
        cert: None,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: true,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-agent-ssh".to_string()),
        service_tenant_id: Some(tenant_id),
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };
    let request_id = Uuid::now_v7();

    let response = processor
        .handle_surface_action_request(surfaces::SurfaceActionRequest {
            request_id,
            tenant_id: tenant_id.to_string(),
            surface_id: surfaces::SurfaceId::new(fixture::SURFACE_ID).unwrap(),
            interaction_id: surfaces::InteractionId::new(fixture::INTERACTION_ID).unwrap(),
            method: Default::default(),
            idempotency_key: "provider-origin-live-disabled".to_string(),
            target_provider_id: None,
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "service.provider-a".to_string(),
            },
            params: serde_json::Map::new(),
            encrypted_sensitive_params: None,
        })
        .await;

    let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
        panic!("expected exactly one SurfaceActionResponse reply");
    };
    assert_eq!(reply.request_id, request_id);
    assert!(
        !reply.success,
        "boot-enabled fixture with no live row must be denied"
    );
    assert!(
        reply.error.is_some(),
        "denial reply must carry an error payload"
    );
}

/// ADR-0033 §5.4(b): disable via the instance-plugins route AFTER the proxy
/// was constructed, then invoke — a filter frozen at construction would still
/// serve the request; the live-handle filter must deny it.
#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn provider_origin_toggle_then_invoke_without_restart_is_denied() {
    let db = crate::test_harness::setup_migrated_db_with_plugins().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state_with_plugin_surfaces(
        db.clone(),
        tenant_id,
        Some(crate::test_harness::synthetic_instance_catalog(true)),
    )
    .await;
    let service_id = Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
    register_calling_service_as_provider(&state, service_id, tenant_id);

    let processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id,
        cert: None,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_update_hooks: false,
        has_ui_surfaces: true,
        has_workload_claims: false,
        runtime_instance_id: None,
        service_app_name: Some("uptrakit-agent-ssh".to_string()),
        service_tenant_id: Some(tenant_id),
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        report_tracker: ReportTracker::new(),
    };

    // 1. Build a router over the SAME state and enable the plugin live
    //    through the production write path (never poke the ArcSwap directly).
    let router = crate::build_router(Arc::clone(&state));
    let client = crate::test_harness::http_client::TestClient::new(router);
    let (register_status, admin_auth) = crate::test_harness::fixtures::register_user(
        &client,
        "owner@test.local",
        "TestPassword123!",
    )
    .await;
    assert_eq!(
        register_status,
        http::StatusCode::CREATED,
        "admin registration must succeed"
    );
    let admin_token = admin_auth.access_token.expose_secret().to_string();

    let enable_status = client
        .put_json(
            &format!("/api/v1/instance-plugins/{}/enabled", fixture::TYPE_ID),
            &serde_json::json!({ "enabled": true }),
        )
        .bearer(&admin_token)
        .send_status()
        .await;
    assert_eq!(
        enable_status,
        http::StatusCode::OK,
        "instance-plugin enable must succeed"
    );

    // 2. Control: fire the invocation while live-enabled — proves the enabled
    //    path works, which is what makes step 4's denial assertion meaningful.
    let enabled_response = processor
        .handle_surface_action_request(surfaces::SurfaceActionRequest {
            request_id: Uuid::now_v7(),
            tenant_id: tenant_id.to_string(),
            surface_id: surfaces::SurfaceId::new(fixture::SURFACE_ID).unwrap(),
            interaction_id: surfaces::InteractionId::new(fixture::INTERACTION_ID).unwrap(),
            method: Default::default(),
            idempotency_key: "provider-origin-toggle-enabled".to_string(),
            target_provider_id: None,
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "service.provider-a".to_string(),
            },
            params: serde_json::Map::new(),
            encrypted_sensitive_params: None,
        })
        .await;
    let [ControllerMessage::SurfaceActionResponse(enabled_reply)] =
        enabled_response.replies.as_slice()
    else {
        panic!("expected exactly one SurfaceActionResponse reply");
    };
    assert!(
        enabled_reply.success,
        "control invocation while live-enabled must succeed: {:?}",
        enabled_reply.error
    );
    let enabled_result = enabled_reply
        .result
        .as_ref()
        .expect("success response has a result");
    assert_eq!(
        enabled_result["pong"], true,
        "fixture handler result must round-trip"
    );

    // 3. Disable through the same production write path.
    let disable_status = client
        .put_json(
            &format!("/api/v1/instance-plugins/{}/enabled", fixture::TYPE_ID),
            &serde_json::json!({ "enabled": false }),
        )
        .bearer(&admin_token)
        .send_status()
        .await;
    assert_eq!(
        disable_status,
        http::StatusCode::OK,
        "instance-plugin disable must succeed"
    );

    // 4. Fire the SAME invocation again, with no reconstruction of the proxy
    //    or state — a filter frozen at construction would still serve this.
    let disabled_response = processor
        .handle_surface_action_request(surfaces::SurfaceActionRequest {
            request_id: Uuid::now_v7(),
            tenant_id: tenant_id.to_string(),
            surface_id: surfaces::SurfaceId::new(fixture::SURFACE_ID).unwrap(),
            interaction_id: surfaces::InteractionId::new(fixture::INTERACTION_ID).unwrap(),
            method: Default::default(),
            idempotency_key: "provider-origin-toggle-disabled".to_string(),
            target_provider_id: None,
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "service.provider-a".to_string(),
            },
            params: serde_json::Map::new(),
            encrypted_sensitive_params: None,
        })
        .await;
    let [ControllerMessage::SurfaceActionResponse(disabled_reply)] =
        disabled_response.replies.as_slice()
    else {
        panic!("expected exactly one SurfaceActionResponse reply");
    };
    assert!(
        !disabled_reply.success,
        "invocation after live-disable without restart must be denied"
    );
    assert!(
        disabled_reply.error.is_some(),
        "denial reply must carry an error payload"
    );
}
