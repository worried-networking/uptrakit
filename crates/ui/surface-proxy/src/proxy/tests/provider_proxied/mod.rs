use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use std::time::Duration;

use uptrakit_wire::{ControllerMessage, surfaces};
use uuid::Uuid;

use super::super::{SurfaceCallerOrigin, SurfaceInvokeRequest, SurfaceProxy, SurfaceProxyError};
use super::{tenant_id, user_id};
use crate::registry::{SurfaceRegistry, SurfaceRegistryConfig};
use uptrakit_service_connections::ServiceConnectionRegistry;

mod rollout;

fn registration(provider_id: &str, service_tenant: Uuid) -> surfaces::SurfaceRegistration {
    surfaces::SurfaceRegistration {
        provider: surfaces::ProviderIdentity {
            provider_id: provider_id.to_string(),
            provider_kind: surfaces::ProviderKind::Service,
            provider_namespace: "service".to_string(),
        },
        framework_generation: surfaces::FrameworkGeneration::new(1, 0),
        capabilities: surfaces::CapabilitySet::from_capabilities([
            surfaces::Capability::TextBlockNode,
            surfaces::Capability::TargetedTargeting,
            surfaces::Capability::MutationAction,
            surfaces::Capability::SensitiveFields,
            surfaces::Capability::ProviderInitiatedActions,
        ]),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Tenant,
            tenant_id: Some(service_tenant.to_string()),
        },
        surfaces: vec![surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor {
                surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                label: "SSH".to_string(),
                priority: 100,
                slot: "software.tabs".to_string(),
                scope: surfaces::Scope::Tenant,
                targeting: surfaces::Targeting::Targeted,
                required_permission: Some("view_software".to_string()),
                provider_kind: surfaces::ProviderKind::Service,
                required_capabilities: surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::TextBlockNode,
                    surfaces::Capability::MutationAction,
                    surfaces::Capability::TargetedTargeting,
                ]),
                root_node: surfaces::SurfaceNode::TextBlock {
                    text: "ok".to_string(),
                },
            },
            interactions: vec![surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                kind: surfaces::InteractionKind::MutationAction,
                label: None,
                required_permission: Some("update_software".to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Object),
                sensitive_fields: vec!["token".to_string()],
                timeout_seconds: Some(2),
                confirmation: None,
                transport: surfaces::InteractionTransport::ProviderProxied,
                workflow_steps: vec![],
                form_ui: None,
            }],
            data_sources: vec![],
        }],
        encryption_metadata: Some(surfaces::ProviderEncryptionMetadata {
            key_id: "key-1".to_string(),
            algorithm: surfaces::ProviderEncryptionAlgorithm::EciesP256,
            public_key: "pubkey".to_string(),
        }),
    }
}

fn registry() -> SurfaceRegistry {
    SurfaceRegistry::new(SurfaceRegistryConfig {
        allowed_controller_queries: HashSet::new(),
        allowed_sse_topics: HashSet::new(),
        allowed_direct_builtin_operations: HashSet::new(),
        ..SurfaceRegistryConfig::default()
    })
}

fn request_with_idem(idempotency_key: &str) -> SurfaceInvokeRequest {
    SurfaceInvokeRequest {
        tenant_id: tenant_id(),
        surface_id: "ssh.guest.panel".to_string(),
        interaction_id: "refresh".to_string(),
        idempotency_key: idempotency_key.to_string(),
        target_provider_id: Some("provider-a".to_string()),
        caller_origin: SurfaceCallerOrigin::UserSession {
            user_id: user_id(),
            session_id: "session-1".to_string(),
        },
        params: serde_json::Map::new(),
        encrypted_sensitive_params: Some(surfaces::EncryptedSensitiveParams {
            key_id: "key-1".to_string(),
            algorithm: surfaces::ProviderEncryptionAlgorithm::EciesP256,
            ciphertext_b64: "AAAA".to_string(),
        }),
    }
}

async fn register_service_for_proxy(
    registry: &SurfaceRegistry,
    service_connections: &ServiceConnectionRegistry,
) -> (Uuid, tokio::sync::mpsc::Receiver<ControllerMessage>) {
    let service_id = Uuid::now_v7();
    registry
        .register_service(
            service_id,
            "uptrakit-agent-ssh",
            Some(tenant_id()),
            registration("provider-a", tenant_id()),
        )
        .expect("registration should succeed");

    let (rx, _cancel) = service_connections
        .register(
            service_id,
            BTreeSet::new(),
            None,
            None,
            Some("uptrakit-agent-ssh".to_string()),
        )
        .await;

    (service_id, rx)
}

#[tokio::test(start_paused = true)]
async fn invoke_correlates_request_and_response() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = Arc::new(SurfaceProxy::new());

    let (_service_id, mut rx) = register_service_for_proxy(&registry, &service_connections).await;
    let proxy_clone = Arc::clone(&proxy);

    tokio::spawn(async move {
        if let Some(ControllerMessage::SurfaceActionRequest(request)) = rx.recv().await {
            proxy_clone.complete(
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

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            request_with_idem("idem-1"),
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("invoke should succeed");

    assert!(response.success);
}

#[tokio::test(start_paused = true)]
async fn invoke_rejects_duplicate_idempotency_deterministically() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = Arc::new(SurfaceProxy::new());
    let (_service_id, mut rx) = register_service_for_proxy(&registry, &service_connections).await;

    let proxy_clone = Arc::clone(&proxy);
    tokio::spawn(async move {
        if let Some(ControllerMessage::SurfaceActionRequest(request)) = rx.recv().await {
            proxy_clone.complete(
                request.request_id,
                surfaces::SurfaceActionResponse {
                    request_id: request.request_id,
                    success: true,
                    result: Some(serde_json::json!({"cached": true})),
                    error: None,
                },
            );
        }
    });

    let first = proxy
        .invoke(
            &service_connections,
            &registry,
            request_with_idem("idem-dup"),
            Some(Duration::from_secs(5)),
        )
        .await;
    assert!(first.is_ok(), "first invocation should succeed");

    let second = proxy
        .invoke(
            &service_connections,
            &registry,
            request_with_idem("idem-dup"),
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("second invocation should be deterministic");
    assert!(
        second.success,
        "duplicate should return deterministic previous result"
    );
}

#[tokio::test(start_paused = true)]
async fn invoke_rejects_cleartext_sensitive_fields() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = SurfaceProxy::new();
    let (_service_id, _rx) = register_service_for_proxy(&registry, &service_connections).await;

    let mut request = request_with_idem("idem-cleartext");
    request
        .params
        .insert("token".to_string(), serde_json::json!("clear"));

    let err = proxy
        .invoke(
            &service_connections,
            &registry,
            request,
            Some(Duration::from_secs(5)),
        )
        .await
        .expect_err("cleartext sensitive field should be rejected");
    assert!(matches!(err, SurfaceProxyError::SensitiveFieldRejected(_)));
}

#[tokio::test(start_paused = true)]
async fn invoke_allows_provider_proxied_requests_without_sensitive_payload() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = Arc::new(SurfaceProxy::new());
    let (_service_id, mut rx) = register_service_for_proxy(&registry, &service_connections).await;

    let invoke_task = tokio::spawn({
        let request = SurfaceInvokeRequest {
            tenant_id: tenant_id(),
            surface_id: "ssh.guest.panel".to_string(),
            interaction_id: "refresh".to_string(),
            idempotency_key: "idem-no-sensitive-payload".to_string(),
            target_provider_id: Some("provider-a".to_string()),
            caller_origin: SurfaceCallerOrigin::UserSession {
                user_id: user_id(),
                session_id: "session-1".to_string(),
            },
            params: serde_json::Map::from_iter([(
                "note".to_string(),
                serde_json::json!("no-secret-change"),
            )]),
            encrypted_sensitive_params: None,
        };
        let proxy = Arc::clone(&proxy);
        async move {
            proxy
                .invoke(
                    &service_connections,
                    &registry,
                    request,
                    Some(Duration::from_secs(5)),
                )
                .await
        }
    });

    let Some(ControllerMessage::SurfaceActionRequest(forwarded_request)) = rx.recv().await else {
        panic!("expected forwarded ControllerMessage::SurfaceActionRequest");
    };
    assert!(forwarded_request.encrypted_sensitive_params.is_none());

    let response = surfaces::SurfaceActionResponse {
        request_id: forwarded_request.request_id,
        success: true,
        result: Some(serde_json::json!({"ok": true})),
        error: None,
    };
    proxy.complete(forwarded_request.request_id, response.clone());

    let result = invoke_task
        .await
        .expect("invoke task should complete")
        .expect("provider-proxied request without sensitive payload should succeed");
    assert_eq!(result, response);
}

#[tokio::test(start_paused = true)]
async fn invoke_times_out_and_emits_cancellation() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = SurfaceProxy::new();
    let (_service_id, mut rx) = register_service_for_proxy(&registry, &service_connections).await;

    let invoke_task = tokio::spawn({
        let request = request_with_idem("idem-timeout");
        let proxy = proxy;
        async move {
            proxy
                .invoke(
                    &service_connections,
                    &registry,
                    request,
                    Some(Duration::from_secs(2)),
                )
                .await
        }
    });

    let first = rx
        .recv()
        .await
        .expect("first message should be action request");
    assert!(matches!(first, ControllerMessage::SurfaceActionRequest(_)));
    let second = rx.recv().await.expect("second message should be cancel");
    assert!(matches!(second, ControllerMessage::SurfaceActionCancel(_)));

    let result = invoke_task.await.expect("invoke task should finish");
    assert!(matches!(result, Err(SurfaceProxyError::Timeout)));
}

#[tokio::test(start_paused = true)]
async fn invoke_ignores_late_response_after_timeout() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = Arc::new(SurfaceProxy::new());
    let (_service_id, mut rx) = register_service_for_proxy(&registry, &service_connections).await;

    let proxy_clone = Arc::clone(&proxy);
    tokio::spawn(async move {
        let request = match rx.recv().await {
            Some(ControllerMessage::SurfaceActionRequest(request)) => request,
            other => panic!("expected surface action request, got {other:?}"),
        };
        tokio::time::advance(Duration::from_secs(3)).await;
        proxy_clone.complete(
            request.request_id,
            surfaces::SurfaceActionResponse {
                request_id: request.request_id,
                success: true,
                result: Some(serde_json::json!({"late": true})),
                error: None,
            },
        );
    });

    let result = proxy
        .invoke(
            &service_connections,
            &registry,
            request_with_idem("idem-late"),
            Some(Duration::from_secs(2)),
        )
        .await;
    assert!(matches!(result, Err(SurfaceProxyError::Timeout)));
}

#[tokio::test(start_paused = true)]
async fn invoke_validates_input_schema_before_dispatch() {
    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = SurfaceProxy::new();
    let service_id = Uuid::now_v7();
    let mut custom_registration = registration("provider-a", tenant_id());
    custom_registration.surfaces[0].interactions[0].input_schema =
        Some(surfaces::SchemaContract::Integer);
    registry
        .register_service(
            service_id,
            "uptrakit-agent-ssh",
            Some(tenant_id()),
            custom_registration,
        )
        .expect("registration should succeed");
    let (_rx, _cancel) = service_connections
        .register(
            service_id,
            BTreeSet::new(),
            None,
            None,
            Some("uptrakit-agent-ssh".to_string()),
        )
        .await;

    let mut request = request_with_idem("idem-schema");
    request
        .params
        .insert("value".to_string(), serde_json::json!("not-integer"));

    let result = proxy
        .invoke(
            &service_connections,
            &registry,
            request,
            Some(Duration::from_secs(5)),
        )
        .await;
    assert!(
        matches!(result, Err(SurfaceProxyError::SchemaValidationFailed(_))),
        "expected schema validation failure, got {result:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn invoke_targeted_surface_requires_explicit_target_provider() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = SurfaceProxy::new();
    let (_service_id, _rx) = register_service_for_proxy(&registry, &service_connections).await;

    let mut request = request_with_idem("idem-missing-target");
    request.target_provider_id = None;

    let err = proxy
        .invoke(
            &service_connections,
            &registry,
            request,
            Some(Duration::from_secs(5)),
        )
        .await
        .expect_err("targeted invocation must require explicit target provider");
    assert!(matches!(err, SurfaceProxyError::TargetProviderRequired));
}

#[tokio::test(start_paused = true)]
async fn invoke_provider_origin_can_route_to_another_provider() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = Arc::new(SurfaceProxy::new());

    let service_a = Uuid::now_v7();
    let mut reg_a = registration("provider-a", tenant_id());
    reg_a.surfaces[0].interactions[0].required_permission = None;
    registry
        .register_service(service_a, "uptrakit-agent-ssh", Some(tenant_id()), reg_a)
        .expect("provider-a registration should succeed");

    let service_b = Uuid::now_v7();
    let mut reg_b = registration("provider-b", tenant_id());
    reg_b.surfaces[0].interactions[0].required_permission = None;
    registry
        .register_service(service_b, "uptrakit-agent-ssh", Some(tenant_id()), reg_b)
        .expect("provider-b registration should succeed");

    let (_rx_a, _cancel_a) = service_connections
        .register(
            service_a,
            BTreeSet::new(),
            None,
            None,
            Some("uptrakit-agent-ssh".to_string()),
        )
        .await;
    let (mut rx_b, _cancel_b) = service_connections
        .register(
            service_b,
            BTreeSet::new(),
            None,
            None,
            Some("uptrakit-agent-ssh".to_string()),
        )
        .await;

    let proxy_clone = Arc::clone(&proxy);
    tokio::spawn(async move {
        if let Some(ControllerMessage::SurfaceActionRequest(request)) = rx_b.recv().await {
            proxy_clone.complete(
                request.request_id,
                surfaces::SurfaceActionResponse {
                    request_id: request.request_id,
                    success: true,
                    result: Some(serde_json::json!({"routed": true})),
                    error: None,
                },
            );
        }
    });

    let mut request = request_with_idem("idem-cross-provider");
    request.target_provider_id = Some("provider-b".to_string());
    request.caller_origin = SurfaceCallerOrigin::Provider {
        service_id: service_a,
    };

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            request,
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("controller-authorized cross-provider invoke should succeed");
    assert!(response.success);
}

#[tokio::test(start_paused = true)]
async fn invoke_returns_no_provider_for_yielded_service_provider() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = Arc::new(SurfaceProxy::new());

    let service_a = Uuid::now_v7();
    registry
        .register_service(
            service_a,
            "uptrakit-agent-ssh",
            Some(tenant_id()),
            registration("provider-a", tenant_id()),
        )
        .expect("provider-a registration should succeed");

    let (mut rx_a, _cancel_a) = service_connections
        .register(
            service_a,
            BTreeSet::new(),
            None,
            None,
            Some("uptrakit-agent-ssh".to_string()),
        )
        .await;
    assert!(service_connections.set_yielded(&service_a, true));

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            request_with_idem("idem-yielded-unavailable"),
            Some(Duration::from_secs(5)),
        )
        .await
        .expect_err("yielded provider should fail fast");

    assert!(matches!(response, SurfaceProxyError::NoProvider));
    assert!(
        rx_a.try_recv().is_err(),
        "yielded provider must not receive the proxied surface request"
    );
}

#[tokio::test(start_paused = true)]
async fn invoke_fails_immediately_when_provider_disconnects() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = Arc::new(SurfaceProxy::new());
    let (_service_id, mut rx) = register_service_for_proxy(&registry, &service_connections).await;

    let proxy_invoke = Arc::clone(&proxy);
    let invoke_task = tokio::spawn(async move {
        proxy_invoke
            .invoke(
                &service_connections,
                &registry,
                request_with_idem("idem-disconnect"),
                Some(Duration::from_secs(60)),
            )
            .await
    });

    let outbound = rx
        .recv()
        .await
        .expect("first message should be action request");
    assert!(matches!(
        outbound,
        ControllerMessage::SurfaceActionRequest(_)
    ));

    proxy.fail_in_flight_for_provider("provider-a");

    let result = invoke_task.await.expect("invoke task should finish");
    assert!(matches!(
        result,
        Err(SurfaceProxyError::ServiceDisconnected)
    ));
}
