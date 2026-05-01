use std::sync::Arc;
use std::sync::Arc as StdArc;
use std::time::Duration;

use parking_lot::Mutex;
use uptrakit_wire::{ControllerMessage, surfaces};

use super::super::super::{
    PluginSurfaceLocalExecutor, SurfaceCallerOrigin, SurfaceProxy, SurfaceProxyError,
};
use super::super::{TestPluginInvoker, tenant_id};
use super::{register_service_for_proxy, registry, request_with_idem};
use uptrakit_service_connections::ServiceConnectionRegistry;

fn rollout(active: bool) -> crate::SurfaceRuntimeRolloutState {
    crate::SurfaceRuntimeRolloutState::phase0(active, Vec::new(), std::collections::BTreeMap::new())
}

fn plugin_registration_for_shared_surface(provider_id: &str) -> surfaces::SurfaceRegistration {
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
            descriptor: surfaces::SurfaceDescriptor {
                surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                label: "Plugin SSH".to_string(),
                priority: 100,
                slot: "software.tabs".to_string(),
                scope: surfaces::Scope::Tenant,
                targeting: surfaces::Targeting::Universal,
                required_permission: Some("view_software".to_string()),
                provider_kind: surfaces::ProviderKind::Plugin,
                required_capabilities: surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::TextBlockNode,
                    surfaces::Capability::MutationAction,
                    surfaces::Capability::UniversalTargeting,
                ]),
                root_node: surfaces::SurfaceNode::TextBlock {
                    text: "plugin".to_string(),
                },
            },
            interactions: vec![surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                kind: surfaces::InteractionKind::MutationAction,
                label: None,
                required_permission: Some("update_software".to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Object),
                sensitive_fields: vec![],
                timeout_seconds: Some(30),
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            }],
            data_sources: vec![],
        }],
        encryption_metadata: None,
    }
}

#[tokio::test(start_paused = true)]
async fn invoke_provider_proxied_interaction_requires_active_rollout() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = Arc::new(SurfaceProxy::new());

    let (_service_id, mut rx) = register_service_for_proxy(&registry, &service_connections).await;

    let inactive = proxy
        .invoke_with_rollout(
            &service_connections,
            &registry,
            &rollout(false),
            request_with_idem("idem-inactive"),
            Some(Duration::from_secs(5)),
        )
        .await;
    assert!(matches!(inactive, Err(SurfaceProxyError::RuntimeInactive)));
    assert!(
        rx.try_recv().is_err(),
        "inactive rollout must not send provider traffic"
    );

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

    let active = proxy
        .invoke_with_rollout(
            &service_connections,
            &registry,
            &rollout(true),
            request_with_idem("idem-active"),
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("active rollout should allow provider-proxied interactions");
    assert!(active.success);
}

#[tokio::test(start_paused = true)]
async fn inactive_rollout_rejects_cached_idempotent_response_replay() {
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

    let request = request_with_idem("idem-cached-when-inactive");
    proxy
        .invoke_with_rollout(
            &service_connections,
            &registry,
            &rollout(true),
            request.clone(),
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("active rollout should populate idempotency cache");

    let replay = proxy
        .invoke_with_rollout(
            &service_connections,
            &registry,
            &rollout(false),
            request,
            Some(Duration::from_secs(5)),
        )
        .await;
    assert!(matches!(replay, Err(SurfaceProxyError::RuntimeInactive)));
}

#[tokio::test(start_paused = true)]
async fn invoke_with_rollout_inactive_short_circuits_before_resolution_permission_and_validation() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let proxy = SurfaceProxy::new();
    let (service_id, _rx) = register_service_for_proxy(&registry, &service_connections).await;

    let mut invalid_provider = request_with_idem("idem-inactive-invalid-provider");
    invalid_provider.target_provider_id = Some("provider-missing".to_string());
    let invalid_provider_result = proxy
        .invoke_with_rollout(
            &service_connections,
            &registry,
            &rollout(false),
            invalid_provider,
            Some(Duration::from_secs(5)),
        )
        .await;
    assert!(matches!(
        invalid_provider_result,
        Err(SurfaceProxyError::RuntimeInactive)
    ));

    let mut permission_denied = request_with_idem("idem-inactive-permission");
    permission_denied.caller_origin = SurfaceCallerOrigin::Provider { service_id };
    let permission_result = proxy
        .invoke_with_rollout(
            &service_connections,
            &registry,
            &rollout(false),
            permission_denied,
            Some(Duration::from_secs(5)),
        )
        .await;
    assert!(matches!(
        permission_result,
        Err(SurfaceProxyError::RuntimeInactive)
    ));

    let validation_result = proxy
        .invoke_with_rollout(
            &service_connections,
            &registry,
            &rollout(false),
            request_with_idem("idem-inactive-validation"),
            Some(Duration::from_secs(0)),
        )
        .await;
    assert!(matches!(
        validation_result,
        Err(SurfaceProxyError::RuntimeInactive)
    ));
}

#[tokio::test(start_paused = true)]
async fn inactive_rollout_untargeted_shared_surface_rejects_local_fallback() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let seen = StdArc::new(Mutex::new(Vec::new()));
    let proxy = Arc::new(SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new_without_database(Arc::new(TestPluginInvoker {
            response: serde_json::json!({"provider": "plugin"}),
            seen: StdArc::clone(&seen),
        })),
    )));

    let (_service_id, mut rx) = register_service_for_proxy(&registry, &service_connections).await;
    registry.register_provider_for_test(
        plugin_registration_for_shared_surface("plugin-a"),
        None,
        None,
    );

    let mut request = request_with_idem("idem-local-fallback");
    request.target_provider_id = None;

    let response = proxy
        .invoke_with_rollout(
            &service_connections,
            &registry,
            &rollout(false),
            request,
            Some(Duration::from_secs(5)),
        )
        .await
        .expect_err("inactive rollout should reject untargeted shared-surface requests");
    assert!(matches!(response, SurfaceProxyError::RuntimeInactive));
    assert!(
        rx.try_recv().is_err(),
        "inactive rollout fallback must not proxy to the service provider"
    );
    assert!(
        seen.lock().is_empty(),
        "inactive rollout must not execute a local provider fallback"
    );
}

#[tokio::test(start_paused = true)]
async fn inactive_rollout_keeps_explicit_provider_targets_blocked() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let seen = StdArc::new(Mutex::new(Vec::new()));
    let proxy = Arc::new(SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new_without_database(Arc::new(TestPluginInvoker {
            response: serde_json::json!({"provider": "plugin"}),
            seen: StdArc::clone(&seen),
        })),
    )));

    let (_service_id, mut rx) = register_service_for_proxy(&registry, &service_connections).await;
    registry.register_provider_for_test(
        plugin_registration_for_shared_surface("plugin-a"),
        None,
        None,
    );

    let result = proxy
        .invoke_with_rollout(
            &service_connections,
            &registry,
            &rollout(false),
            request_with_idem("idem-explicit-service"),
            Some(Duration::from_secs(5)),
        )
        .await;
    assert!(matches!(result, Err(SurfaceProxyError::RuntimeInactive)));
    assert!(
        rx.try_recv().is_err(),
        "inactive rollout must not send explicitly targeted provider traffic"
    );

    let mut plugin_target_request = request_with_idem("idem-explicit-plugin");
    plugin_target_request.target_provider_id = Some("plugin-a".to_string());
    let plugin_target_result = proxy
        .invoke_with_rollout(
            &service_connections,
            &registry,
            &rollout(false),
            plugin_target_request,
            Some(Duration::from_secs(5)),
        )
        .await;
    assert!(matches!(
        plugin_target_result,
        Err(SurfaceProxyError::RuntimeInactive)
    ));
    assert!(
        seen.lock().is_empty(),
        "inactive rollout must not execute explicitly targeted local provider actions"
    );
}

#[tokio::test(start_paused = true)]
async fn inactive_rollout_provider_origin_does_not_fall_through_to_local_provider() {
    let registry = registry();
    let service_connections = ServiceConnectionRegistry::new();
    let seen = StdArc::new(Mutex::new(Vec::new()));
    let proxy = Arc::new(SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new_without_database(Arc::new(TestPluginInvoker {
            response: serde_json::json!({"provider": "plugin"}),
            seen: StdArc::clone(&seen),
        })),
    )));

    let (service_id, mut rx) = register_service_for_proxy(&registry, &service_connections).await;
    registry.register_provider_for_test(
        plugin_registration_for_shared_surface("plugin-a"),
        None,
        None,
    );

    let mut request = request_with_idem("idem-provider-no-fallback");
    request.target_provider_id = None;
    request.caller_origin = SurfaceCallerOrigin::Provider { service_id };

    let result = proxy
        .invoke_with_rollout(
            &service_connections,
            &registry,
            &rollout(false),
            request,
            Some(Duration::from_secs(5)),
        )
        .await;

    assert!(matches!(result, Err(SurfaceProxyError::RuntimeInactive)));
    assert!(
        rx.try_recv().is_err(),
        "inactive rollout must not send provider traffic"
    );
    assert!(
        seen.lock().is_empty(),
        "provider-originated requests must not fall through to local providers"
    );
}
