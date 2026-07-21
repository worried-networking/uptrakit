use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use uptrakit_wire::{ControllerMessage, surfaces};
use uuid::Uuid;

use super::super::{
    PluginSurfaceLocalExecutor, SurfaceCallerOrigin, SurfaceInvokeRequest, SurfaceProxy,
    SurfaceProxyError,
};
use super::{TestPluginInvoker, tenant_id, user_id};
use crate::registry::{SurfaceRegistry, SurfaceRegistryConfig};
use uptrakit_service_connections::ServiceConnectionRegistry;

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
            descriptor: surfaces::SurfaceDescriptor::builder()
                .surface_id(surfaces::SurfaceId::new("ssh.guest.panel").unwrap())
                .label("SSH")
                .priority(100)
                .slot("software.tabs")
                .scope(surfaces::Scope::Tenant)
                .targeting(surfaces::Targeting::Targeted)
                .required_permission("view_software")
                .provider_kind(surfaces::ProviderKind::Service)
                .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::TextBlockNode,
                    surfaces::Capability::MutationAction,
                    surfaces::Capability::TargetedTargeting,
                ]))
                .root_node(surfaces::SurfaceNode::TextBlock {
                    text: "ok".to_string(),
                })
                .build(),
            interactions: vec![{
                let mut i = surfaces::InteractionDescriptor::new(
                    surfaces::InteractionId::new("refresh").unwrap(),
                    surfaces::InteractionKind::MutationAction,
                    "Action",
                    surfaces::InteractionTransport::ProviderProxied,
                );
                i.required_permission = Some("update_software".to_string());
                i.input_schema = Some(surfaces::SchemaContract::Object);
                i.result_schema = Some(surfaces::SchemaContract::Object);
                i.sensitive_fields = vec!["token".to_string()];
                i.timeout_seconds = Some(2);
                i
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

/// A `Plugin`-kind registration with a permissioned, `ControllerLocal`
/// interaction, used by the `provider_invocable` gate tests below.
///
/// This module's shared `registration()` fixture above registers a
/// `Service`-kind provider, which admission (task 1) forbids from setting
/// `provider_invocable` alongside `required_permission` — that combination
/// is currently allowed only for `Plugin`/`BuiltIn` providers. `ControllerLocal`
/// transport lets the interaction resolve without a connected service.
fn plugin_registration_with_permission(
    provider_id: &str,
    provider_invocable: bool,
) -> surfaces::SurfaceRegistration {
    surfaces::SurfaceRegistration {
        provider: surfaces::ProviderIdentity {
            provider_id: provider_id.to_string(),
            provider_kind: surfaces::ProviderKind::Plugin,
            provider_namespace: "plugin".to_string(),
        },
        framework_generation: surfaces::FrameworkGeneration::new(1, 0),
        capabilities: surfaces::CapabilitySet::from_capabilities([
            surfaces::Capability::TextBlockNode,
            surfaces::Capability::UniversalTargeting,
            surfaces::Capability::MutationAction,
        ]),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Tenant,
            tenant_id: Some(tenant_id().to_string()),
        },
        surfaces: vec![surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor::builder()
                .surface_id(surfaces::SurfaceId::new("proxmox.guest.invocable_panel").unwrap())
                .label("Guest Panel")
                .priority(100)
                .slot(surfaces::SLOT_SETTINGS_BELOW_GLOBAL)
                .scope(surfaces::Scope::Global)
                .targeting(surfaces::Targeting::Universal)
                .provider_kind(surfaces::ProviderKind::Plugin)
                .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::TextBlockNode,
                    surfaces::Capability::MutationAction,
                    surfaces::Capability::UniversalTargeting,
                ]))
                .root_node(surfaces::SurfaceNode::TextBlock {
                    text: "ok".to_string(),
                })
                .build(),
            interactions: vec![{
                let mut i = surfaces::InteractionDescriptor::new(
                    surfaces::InteractionId::new("refresh").unwrap(),
                    surfaces::InteractionKind::MutationAction,
                    "Action",
                    surfaces::InteractionTransport::ControllerLocal,
                );
                i.required_permission = Some("update_hosts".to_string());
                i.provider_invocable = provider_invocable;
                i.input_schema = Some(surfaces::SchemaContract::Object);
                i.result_schema = Some(surfaces::SchemaContract::Object);
                i.timeout_seconds = Some(30);
                i
            }],
            data_sources: vec![],
        }],
        encryption_metadata: None,
    }
}

fn registry() -> SurfaceRegistry {
    SurfaceRegistry::new(SurfaceRegistryConfig {
        allowed_controller_queries: HashSet::new(),
        allowed_sse_topics: HashSet::new(),
        ..SurfaceRegistryConfig::default()
    })
}

fn request_with_idem(idempotency_key: &str) -> SurfaceInvokeRequest {
    SurfaceInvokeRequest {
        method: None,
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
            method: None,
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
async fn invoke_stamps_effective_post_method_for_mutation() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = Arc::new(SurfaceProxy::new());
    let (_service_id, mut rx) = register_service_for_proxy(&registry, &service_connections).await;

    let invoke_task = tokio::spawn({
        let request = request_with_idem("idem-method-post");
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
    assert_eq!(
        forwarded_request.method,
        surfaces::InteractionHttpMethod::Post
    );

    proxy.complete(
        forwarded_request.request_id,
        surfaces::SurfaceActionResponse {
            request_id: forwarded_request.request_id,
            success: true,
            result: Some(serde_json::json!({"ok": true})),
            error: None,
        },
    );

    let result = invoke_task
        .await
        .expect("invoke task should complete")
        .expect("provider-proxied mutation should succeed");
    assert!(result.success);
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

/// Regression pin, not the RED: this denial is the gate's current behavior
/// both before and after the `provider_invocable` gate change — it stays
/// green throughout. It exists so the permission-gated deny path has a
/// direct test, independent of the `provider_invocable` opt-in exercised by
/// `invoke_provider_origin_allowed_when_provider_invocable` below.
///
/// Targets a `bootstrap_plugin`-registered interaction (see
/// `plugin_registration_with_permission`) rather than reusing this module's
/// shared `Service`-kind `registration()` fixture: admission (task 1) forbids
/// `provider_invocable` together with `required_permission` on
/// Service-registered interactions, so that combination cannot be expressed
/// via `register_service`.
#[tokio::test(start_paused = true)]
async fn invoke_provider_origin_denied_for_permission_gated_interaction() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new_without_database(Arc::new(TestPluginInvoker {
            response: serde_json::json!({"routed": true}),
            seen: Arc::new(Mutex::new(Vec::new())),
        })),
    ));

    let service_a = Uuid::now_v7();
    registry
        .register_service(
            service_a,
            "uptrakit-agent-ssh",
            Some(tenant_id()),
            registration("provider-a", tenant_id()),
        )
        .expect("provider-a registration should succeed");

    registry
        .bootstrap_plugin(plugin_registration_with_permission("provider-b", false))
        .expect("provider-b registration should succeed");

    let request = SurfaceInvokeRequest {
        method: None,
        tenant_id: tenant_id(),
        surface_id: "proxmox.guest.invocable_panel".to_string(),
        interaction_id: "refresh".to_string(),
        idempotency_key: "idem-cross-provider-denied".to_string(),
        target_provider_id: Some("provider-b".to_string()),
        caller_origin: SurfaceCallerOrigin::Provider {
            service_id: service_a,
        },
        params: serde_json::Map::new(),
        encrypted_sensitive_params: None,
    };

    let result = proxy
        .invoke(
            &service_connections,
            &registry,
            request,
            Some(Duration::from_secs(5)),
        )
        .await;
    assert!(matches!(
        result,
        Err(SurfaceProxyError::PermissionDenied(_))
    ));
}

/// Behavioral RED: fails with `PermissionDenied` until the provider-permission
/// gate honors `provider_invocable` (task 2's gate change). Same setup as
/// `invoke_provider_origin_denied_for_permission_gated_interaction` above,
/// plus the target interaction opting in via `provider_invocable = true`.
#[tokio::test(start_paused = true)]
async fn invoke_provider_origin_allowed_when_provider_invocable() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new_without_database(Arc::new(TestPluginInvoker {
            response: serde_json::json!({"routed": true}),
            seen: Arc::new(Mutex::new(Vec::new())),
        })),
    ));

    let service_a = Uuid::now_v7();
    registry
        .register_service(
            service_a,
            "uptrakit-agent-ssh",
            Some(tenant_id()),
            registration("provider-a", tenant_id()),
        )
        .expect("provider-a registration should succeed");

    registry
        .bootstrap_plugin(plugin_registration_with_permission("provider-b", true))
        .expect("provider-b registration should succeed");

    let request = SurfaceInvokeRequest {
        method: None,
        tenant_id: tenant_id(),
        surface_id: "proxmox.guest.invocable_panel".to_string(),
        interaction_id: "refresh".to_string(),
        idempotency_key: "idem-cross-provider-invocable".to_string(),
        target_provider_id: Some("provider-b".to_string()),
        caller_origin: SurfaceCallerOrigin::Provider {
            service_id: service_a,
        },
        params: serde_json::Map::new(),
        encrypted_sensitive_params: None,
    };

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            request,
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("provider_invocable interaction should allow provider-origin invoke");
    assert!(response.success);
}

/// Regression for the production break: the agent sends nested provider→plugin
/// calls with `target_provider_id: None` (it cannot know the controller-side
/// plugin's provider id). Before the resolution fix, implicit resolution forced
/// the caller's own provider (`provider-a`) as the target and failed with
/// `InvalidProvider("provider-a")` — the `provider_invocable` gate was never
/// reached. Same setup as `invoke_provider_origin_allowed_when_provider_invocable`
/// but with `target_provider_id: None`, exercising the real agent path.
#[tokio::test(start_paused = true)]
async fn invoke_provider_origin_resolves_target_from_surface_when_target_none() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new_without_database(Arc::new(TestPluginInvoker {
            response: serde_json::json!({"routed": true}),
            seen: Arc::new(Mutex::new(Vec::new())),
        })),
    ));

    let service_a = Uuid::now_v7();
    registry
        .register_service(
            service_a,
            "uptrakit-agent-ssh",
            Some(tenant_id()),
            registration("provider-a", tenant_id()),
        )
        .expect("provider-a registration should succeed");

    registry
        .bootstrap_plugin(plugin_registration_with_permission("provider-b", true))
        .expect("provider-b registration should succeed");

    let request = SurfaceInvokeRequest {
        method: None,
        tenant_id: tenant_id(),
        surface_id: "proxmox.guest.invocable_panel".to_string(),
        interaction_id: "refresh".to_string(),
        idempotency_key: "idem-provider-origin-target-none".to_string(),
        target_provider_id: None,
        caller_origin: SurfaceCallerOrigin::Provider {
            service_id: service_a,
        },
        params: serde_json::Map::new(),
        encrypted_sensitive_params: None,
    };

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            request,
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("target=None provider-origin invoke should resolve to the surface's plugin");
    assert!(response.success);
}

/// A service invoking its *own* surface with `target_provider_id: None` still
/// self-resolves: the caller provides the requested surface, so it names the
/// provider unambiguously even for a `Targeted` surface (no `TargetProviderRequired`).
#[tokio::test(start_paused = true)]
async fn invoke_provider_origin_self_target_when_target_none() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = Arc::new(SurfaceProxy::new());

    let service_a = Uuid::now_v7();
    let mut reg_a = registration("provider-a", tenant_id());
    reg_a.surfaces[0].interactions[0].required_permission = None;
    registry
        .register_service(service_a, "uptrakit-agent-ssh", Some(tenant_id()), reg_a)
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

    let proxy_clone = Arc::clone(&proxy);
    tokio::spawn(async move {
        if let Some(ControllerMessage::SurfaceActionRequest(request)) = rx_a.recv().await {
            proxy_clone.complete(
                request.request_id,
                surfaces::SurfaceActionResponse {
                    request_id: request.request_id,
                    success: true,
                    result: Some(serde_json::json!({"self": true})),
                    error: None,
                },
            );
        }
    });

    let mut request = request_with_idem("idem-provider-origin-self-target");
    request.target_provider_id = None;
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
        .expect("service self-invoking its own surface should route to itself");
    assert!(response.success);
}

/// An explicit but unknown `target_provider_id` still errors `InvalidProvider`;
/// the message now names the provider id instead of surfacing it bare.
#[tokio::test(start_paused = true)]
async fn invoke_explicit_bogus_target_errors_with_named_provider() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = SurfaceProxy::new();

    registry
        .bootstrap_plugin(plugin_registration_with_permission("provider-b", true))
        .expect("provider-b registration should succeed");

    let request = SurfaceInvokeRequest {
        method: None,
        tenant_id: tenant_id(),
        surface_id: "proxmox.guest.invocable_panel".to_string(),
        interaction_id: "refresh".to_string(),
        idempotency_key: "idem-bogus-target".to_string(),
        target_provider_id: Some("provider-ghost".to_string()),
        caller_origin: SurfaceCallerOrigin::UserSession {
            user_id: user_id(),
            session_id: "session-1".to_string(),
        },
        params: serde_json::Map::new(),
        encrypted_sensitive_params: None,
    };

    let err = proxy
        .invoke(
            &service_connections,
            &registry,
            request,
            Some(Duration::from_secs(5)),
        )
        .await
        .expect_err("unknown explicit target provider must error");
    match err {
        SurfaceProxyError::InvalidProvider(message) => {
            assert!(
                message.contains("provider-ghost"),
                "message should name the provider id: {message}"
            );
        }
        other => panic!("expected InvalidProvider, got {other:?}"),
    }
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

#[tokio::test]
async fn provider_proxied_client_disconnect_releases_budget_and_idempotency() {
    let registry = Arc::new(registry());
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = Arc::new(SurfaceProxy::new());
    let (_service_id, mut rx) = register_service_for_proxy(&registry, &service_connections).await;

    // Drive invoke in a task and DON'T respond — park it on `timeout(rx)`.
    let proxy_clone = Arc::clone(&proxy);
    let sc = service_connections.clone();
    let reg = Arc::clone(&registry);
    let handle = tokio::spawn(async move {
        proxy_clone
            .invoke(
                &sc,
                &reg,
                request_with_idem("idem-cancel"),
                Some(Duration::from_secs(120)),
            )
            .await
    });

    // Wait for the outbound request — proves register_pending ran and counters incremented.
    let outbound = rx.recv().await.expect("outbound surface request");
    assert!(matches!(
        outbound,
        ControllerMessage::SurfaceActionRequest(_)
    ));
    assert_eq!(
        proxy
            .pending
            .lock()
            .in_flight_per_provider
            .get("provider-a")
            .copied(),
        Some(1),
        "provider budget must be reserved mid-flight (non-vacuous baseline)"
    );

    // Simulate the client disconnecting: drop the invoke future.
    handle.abort();
    let _ = handle.await;

    let state = proxy.pending.lock();
    assert!(
        state.in_flight_per_provider.is_empty(),
        "provider budget leaked after cancel"
    );
    assert!(
        state.in_flight_per_tenant.is_empty(),
        "tenant budget leaked after cancel"
    );
    assert!(
        state.in_flight_idempotency.is_empty(),
        "idempotency reservation leaked after cancel"
    );
}

#[tokio::test]
async fn provider_proxied_success_releases_exactly_once() {
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
            request_with_idem("idem-ok"),
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("invoke should succeed");
    assert!(response.success);

    let state = proxy.pending.lock();
    assert!(state.in_flight_per_provider.is_empty());
    assert!(state.in_flight_per_tenant.is_empty());
    assert!(state.in_flight_idempotency.is_empty());
}

#[tokio::test]
async fn provider_proxied_repeated_cancellation_does_not_accumulate_budget() {
    let registry = Arc::new(registry());
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = Arc::new(SurfaceProxy::new());
    let (_service_id, mut rx) = register_service_for_proxy(&registry, &service_connections).await;

    // Far more cancellations than the 32-per-provider cap — a leak would trip RateLimited well before 40.
    for i in 0..40 {
        let proxy_clone = Arc::clone(&proxy);
        let sc = service_connections.clone();
        let reg = Arc::clone(&registry);
        let handle = tokio::spawn(async move {
            proxy_clone
                .invoke(
                    &sc,
                    &reg,
                    request_with_idem(&format!("idem-loop-{i}")),
                    Some(Duration::from_secs(120)),
                )
                .await
        });
        let outbound = rx.recv().await.expect("outbound request");
        assert!(
            matches!(outbound, ControllerMessage::SurfaceActionRequest(_)),
            "iteration {i} must register and send (not RateLimited)"
        );
        handle.abort();
        let _ = handle.await;
    }

    // Non-vacuous: a leak would leave up to 40 stuck entries here; the guard drives it to empty.
    assert!(
        proxy.pending.lock().in_flight_per_provider.is_empty(),
        "cancelled requests must not accumulate provider budget"
    );
}
