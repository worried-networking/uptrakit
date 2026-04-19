use std::sync::Arc;
use std::sync::Arc as StdArc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use uptrakit_internal_wire::surfaces;
use uptrakit_plugin_infrastructure_registry::SurfaceActionError;

use super::super::{
    PluginSurfaceActionInvoker, PluginSurfaceLocalExecutor, SurfaceCallerOrigin,
    SurfaceInvokeRequest, SurfaceProxy, SurfaceProxyError, map_surface_action_error,
};
use super::{TestPluginInvoker, tenant_id, user_id};
use crate::service_connections::ServiceConnectionRegistry;
use crate::surface_registry::{SurfaceRegistry, SurfaceRegistryConfig};

fn plugin_registration(provider_id: &str) -> surfaces::SurfaceRegistration {
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
                surface_id: surfaces::SurfaceId::new("notifications.email.global_smtp").unwrap(),
                label: "SMTP Defaults".to_string(),
                priority: 100,
                slot: surfaces::SLOT_SETTINGS_BELOW_GLOBAL.to_string(),
                scope: surfaces::Scope::Global,
                targeting: surfaces::Targeting::Universal,
                required_permission: None,
                provider_kind: surfaces::ProviderKind::Plugin,
                required_capabilities: surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::TextBlockNode,
                    surfaces::Capability::MutationAction,
                    surfaces::Capability::UniversalTargeting,
                ]),
                root_node: surfaces::SurfaceNode::TextBlock {
                    text: "ok".to_string(),
                },
            },
            interactions: vec![surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("save_global_smtp").unwrap(),
                kind: surfaces::InteractionKind::MutationAction,
                label: None,
                required_permission: None,
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

fn plugin_registration_with_local_sensitive(provider_id: &str) -> surfaces::SurfaceRegistration {
    let mut registration = plugin_registration(provider_id);
    registration
        .capabilities
        .0
        .insert(surfaces::Capability::SensitiveFields);
    registration.surfaces[0].interactions[0].sensitive_fields = vec!["smtp_password".to_string()];
    registration
}

struct BlockingPluginInvoker {
    started: StdArc<tokio::sync::Notify>,
    release: StdArc<tokio::sync::Notify>,
    calls: StdArc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl PluginSurfaceActionInvoker for BlockingPluginInvoker {
    async fn invoke(
        &self,
        _db: Option<&sea_orm::DatabaseConnection>,
        _tenant_id: Option<uuid::Uuid>,
        _caller_user_id: Option<uuid::Uuid>,
        _surface_id: &str,
        _interaction_id: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, SurfaceActionError> {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.started.notify_waiters();
        self.release.notified().await;
        Ok(serde_json::json!({"ok": true}))
    }
}

struct ErrorPluginInvoker {
    error: SurfaceActionError,
}

#[async_trait]
impl PluginSurfaceActionInvoker for ErrorPluginInvoker {
    async fn invoke(
        &self,
        _db: Option<&sea_orm::DatabaseConnection>,
        _tenant_id: Option<uuid::Uuid>,
        _caller_user_id: Option<uuid::Uuid>,
        _surface_id: &str,
        _interaction_id: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, SurfaceActionError> {
        Err(self.error.clone())
    }
}

#[tokio::test]
async fn map_surface_action_error_preserves_invalid_vs_internal_categories() {
    let invalid = map_surface_action_error(SurfaceActionError::InvalidInput(
        "missing config".to_string(),
    ));
    assert!(matches!(
        invalid,
        SurfaceProxyError::SchemaValidationFailed(_)
    ));

    let integration = map_surface_action_error(SurfaceActionError::ControllerIntegration(
        "db unavailable".to_string(),
    ));
    assert!(matches!(integration, SurfaceProxyError::SendFailed));

    let plugin = map_surface_action_error(SurfaceActionError::PluginInternal(
        "plugin panic".to_string(),
    ));
    assert!(matches!(plugin, SurfaceProxyError::SendFailed));
}

#[tokio::test(start_paused = true)]
async fn invoke_executes_plugin_controller_local_interaction() {
    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(plugin_registration("plugin.notifications_email"))
        .expect("plugin registration should succeed");

    let seen = StdArc::new(Mutex::new(Vec::new()));
    let invoker = TestPluginInvoker {
        response: serde_json::json!({"ok": true}),
        seen: StdArc::clone(&seen),
    };
    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new_without_database(Arc::new(invoker)),
    ));
    let service_connections = ServiceConnectionRegistry::new();

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "notifications.email.global_smtp".to_string(),
                interaction_id: "save_global_smtp".to_string(),
                idempotency_key: "idem-plugin-local".to_string(),
                target_provider_id: None,
                caller_origin: SurfaceCallerOrigin::UserSession {
                    user_id: user_id(),
                    session_id: "session-1".to_string(),
                },
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
            },
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("plugin-backed local interaction should succeed");

    assert!(response.success);
    assert_eq!(response.result, Some(serde_json::json!({"ok": true})));
    let seen = seen.lock();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "notifications.email.global_smtp");
    assert_eq!(seen[0].1, "save_global_smtp");
    assert_eq!(seen[0].2, Some(tenant_id()));
    assert_eq!(seen[0].3, Some(user_id()));
}

#[tokio::test(start_paused = true)]
async fn invoke_controller_local_preserves_surface_action_error_categories() {
    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(plugin_registration("plugin.notifications_email"))
        .expect("plugin registration should succeed");
    let service_connections = ServiceConnectionRegistry::new();

    let cases = [
        (
            "invalid-input",
            SurfaceActionError::InvalidInput("missing field".to_string()),
            SurfaceProxyError::SchemaValidationFailed("missing field".to_string()),
        ),
        (
            "controller-integration",
            SurfaceActionError::ControllerIntegration("db down".to_string()),
            SurfaceProxyError::SendFailed,
        ),
        (
            "plugin-internal",
            SurfaceActionError::PluginInternal("panic".to_string()),
            SurfaceProxyError::SendFailed,
        ),
    ];

    for (suffix, plugin_error, expected_proxy_error) in cases {
        let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new_without_database(Arc::new(ErrorPluginInvoker {
                error: plugin_error,
            })),
        ));
        let err = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "notifications.email.global_smtp".to_string(),
                    interaction_id: "save_global_smtp".to_string(),
                    idempotency_key: format!("idem-plugin-local-error-{suffix}"),
                    target_provider_id: None,
                    caller_origin: SurfaceCallerOrigin::UserSession {
                        user_id: user_id(),
                        session_id: "session-1".to_string(),
                    },
                    params: serde_json::Map::new(),
                    encrypted_sensitive_params: None,
                },
                Some(Duration::from_secs(5)),
            )
            .await
            .expect_err("controller-local plugin error should map to proxy error");
        assert_eq!(err, expected_proxy_error);
    }
}

#[tokio::test(start_paused = true)]
async fn invoke_controller_local_rejects_concurrent_duplicate_idempotency() {
    let registry = Arc::new(SurfaceRegistry::new(SurfaceRegistryConfig::default()));
    registry
        .bootstrap_plugin(plugin_registration("plugin.notifications_email"))
        .expect("plugin registration should succeed");

    let started = StdArc::new(tokio::sync::Notify::new());
    let release = StdArc::new(tokio::sync::Notify::new());
    let calls = StdArc::new(std::sync::atomic::AtomicUsize::new(0));
    let proxy = Arc::new(SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new_without_database(Arc::new(BlockingPluginInvoker {
            started: StdArc::clone(&started),
            release: StdArc::clone(&release),
            calls: StdArc::clone(&calls),
        })),
    )));
    let service_connections = Arc::new(ServiceConnectionRegistry::new());

    let proxy_first = Arc::clone(&proxy);
    let registry_first = Arc::clone(&registry);
    let service_connections_first = Arc::clone(&service_connections);
    let first_invoke = tokio::spawn(async move {
        proxy_first
            .invoke(
                &service_connections_first,
                &registry_first,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "notifications.email.global_smtp".to_string(),
                    interaction_id: "save_global_smtp".to_string(),
                    idempotency_key: "idem-local-dup".to_string(),
                    target_provider_id: None,
                    caller_origin: SurfaceCallerOrigin::UserSession {
                        user_id: user_id(),
                        session_id: "session-1".to_string(),
                    },
                    params: serde_json::Map::new(),
                    encrypted_sensitive_params: None,
                },
                Some(Duration::from_secs(5)),
            )
            .await
    });

    started.notified().await;

    let second = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "notifications.email.global_smtp".to_string(),
                interaction_id: "save_global_smtp".to_string(),
                idempotency_key: "idem-local-dup".to_string(),
                target_provider_id: None,
                caller_origin: SurfaceCallerOrigin::UserSession {
                    user_id: user_id(),
                    session_id: "session-1".to_string(),
                },
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
            },
            Some(Duration::from_secs(5)),
        )
        .await;

    assert!(
        matches!(second, Err(SurfaceProxyError::DuplicateRequest)),
        "concurrent duplicate local invocation must be rejected"
    );

    release.notify_waiters();
    let first = first_invoke
        .await
        .expect("first invoke task should complete")
        .expect("first local invocation should succeed");
    assert!(first.success);
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
}

#[tokio::test(start_paused = true)]
async fn invoke_controller_local_allows_cleartext_sensitive_fields() {
    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(plugin_registration_with_local_sensitive(
            "plugin.notifications_email",
        ))
        .expect("plugin registration should succeed");

    let invoker = TestPluginInvoker {
        response: serde_json::json!({"ok": true}),
        seen: StdArc::new(Mutex::new(Vec::new())),
    };
    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new_without_database(Arc::new(invoker)),
    ));
    let service_connections = ServiceConnectionRegistry::new();
    let mut params = serde_json::Map::new();
    params.insert("smtp_password".to_string(), serde_json::json!("clear"));

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "notifications.email.global_smtp".to_string(),
                interaction_id: "save_global_smtp".to_string(),
                idempotency_key: "idem-plugin-local-sensitive".to_string(),
                target_provider_id: None,
                caller_origin: SurfaceCallerOrigin::UserSession {
                    user_id: user_id(),
                    session_id: "session-1".to_string(),
                },
                params,
                encrypted_sensitive_params: None,
            },
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("controller-local sensitive fields should be accepted in cleartext");

    assert!(response.success);
    assert_eq!(response.result, Some(serde_json::json!({"ok": true})));
}
