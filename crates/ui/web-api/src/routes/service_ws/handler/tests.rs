use super::test_support::*;
use super::*;

use std::collections::HashSet;
use std::sync::Arc;
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_wire::limits::MAX_SHORT_STRING_LEN;
use uptrakit_wire::report_tracker::ReportTracker;
use uptrakit_wire::surfaces;
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
            interaction_id: surfaces::InteractionId::new("configure_smtp").unwrap(),
            idempotency_key: "scope-violation".to_string(),
            target_provider_id: Some("provider-a".to_string()),
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "provider-a".to_string(),
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
        Some("notifications.email/configure_smtp")
    );
    let details = row
        .details_json
        .as_ref()
        .expect("scope denial audit should include details");
    assert_eq!(details["service_app_name"], "uptrakit-mqtt");
    assert_eq!(details["surface_id"], "notifications.email");
    assert_eq!(details["interaction_id"], "configure_smtp");
    assert_eq!(details["target_provider_id"], "provider-a");
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
            idempotency_key: "x".repeat(MAX_SHORT_STRING_LEN + 1),
            target_provider_id: Some("provider-a".to_string()),
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "provider-a".to_string(),
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
    assert_eq!(details["target_provider_id"], "provider-a");
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
            idempotency_key: "invalid-tenant".to_string(),
            target_provider_id: Some("provider-a".to_string()),
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "provider-a".to_string(),
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
    assert_eq!(details["target_provider_id"], "provider-a");
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
    let mut registration = test_surface_registration("provider-a", tenant_id);
    registration.surfaces[0].interactions[0].required_permission = None;
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
            idempotency_key: "lookup-failure".to_string(),
            target_provider_id: Some("missing-provider".to_string()),
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "provider-a".to_string(),
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
    let mut registration = test_surface_registration("provider-a", tenant_id);
    registration.surfaces[0].interactions[0].required_permission = None;
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
            idempotency_key: "surface-success".to_string(),
            target_provider_id: Some("provider-a".to_string()),
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "provider-a".to_string(),
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
    assert_eq!(details["target_provider_id"], "provider-a");
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
    let mut registration = test_surface_registration("provider-a", tenant_id);
    registration.surfaces[0].interactions[0].required_permission = None;
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
            idempotency_key: "surface-provider-unavailable".to_string(),
            target_provider_id: Some("provider-a".to_string()),
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "provider-a".to_string(),
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
    assert_eq!(details["target_provider_id"], "provider-a");
    assert_eq!(details["provider_kind"], "service");
    assert_eq!(details["provider_service_app_name"], "uptrakit-agent-ssh");
    assert_eq!(details["reason_code"], "provider_unavailable");
    assert!(details.get("params").is_none());
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn setup_enrolled_session_emits_enrollment_completed_audit_for_already_approved_service() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
    let service_id = Uuid::now_v7();
    insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent").await;

    let session = setup_enrolled_session(&state, service_id, false).await;
    assert!(session.approved);

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SERVICE_ENROLLMENT_COMPLETED,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(service_id));
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
    let mut registration = test_surface_registration("provider-a", tenant_id);
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
    assert_eq!(row.target_id.as_deref(), Some("provider-a"));
    assert_eq!(row.target_display.as_deref(), Some("provider-a"));
    let details = row
        .details_json
        .as_ref()
        .expect("validation failure audit should include details");
    assert_eq!(details["provider_id"], "provider-a");
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
    let mut registration = test_surface_registration("provider-a", tenant_id);
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
    assert_eq!(row.target_id.as_deref(), Some("provider-a"));
    assert_eq!(row.target_display.as_deref(), Some("provider-a"));
    let details = row
        .details_json
        .as_ref()
        .expect("rejection audit should include details");
    assert_eq!(details["provider_id"], "provider-a");
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
    let mut registration = test_surface_registration("provider-system", tenant_id);
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
    assert_eq!(row.target_id.as_deref(), Some("provider-system"));
    assert_eq!(row.target_display.as_deref(), Some("provider-system"));
    let details = row
        .details_json
        .as_ref()
        .expect("success audit should include details");
    assert_eq!(details["provider_id"], "provider-system");
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
        .handle_surface_registration(test_surface_registration("provider-a", tenant_id))
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

    // Register provider-a from service_id (succeeds).
    state
        .surface_proxy_deps
        .registry
        .register_service(
            service_id,
            "uptrakit-agent-ssh",
            Some(tenant_id),
            test_surface_registration("provider-a", tenant_id),
        )
        .expect("first registration should succeed");

    let mut rx = state
        .notification
        .event_broadcaster
        .subscribe(tenant_id)
        .await;

    // Try to claim the SAME provider ID ("provider-a") from a different service
    // (service_id_b). The registry rejects this because provider-a is already bound
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
        .handle_surface_registration(test_surface_registration("provider-a", tenant_id))
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
