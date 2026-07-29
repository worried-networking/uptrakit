use std::sync::Arc;
use std::sync::Arc as StdArc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use uptrakit_plugin_infrastructure_registry::SurfaceActionError;
use uptrakit_wire::surfaces;

use super::super::{
    PluginSurfaceActionInvoker, PluginSurfaceLocalExecutor, SurfaceCallerOrigin,
    SurfaceInvokeRequest, SurfaceInvokerContext, SurfaceLocalActionExecutor, SurfaceProxy,
    SurfaceProxyError, map_surface_action_error,
};
use super::{TestPluginInvoker, tenant_id, user_id};
use crate::registry::AllProvidersVisible;
use crate::registry::{SurfaceRegistry, SurfaceRegistryConfig};
use uptrakit_service_connections::ServiceConnectionRegistry;

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
            descriptor: surfaces::SurfaceDescriptor::builder()
                .surface_id(surfaces::SurfaceId::new("notifications.email.global-smtp").unwrap())
                .label("SMTP Defaults")
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
                    surfaces::InteractionId::new("smtp").unwrap(),
                    surfaces::InteractionKind::MutationAction,
                    "Action",
                    surfaces::InteractionTransport::ControllerLocal,
                );
                i.http_method = surfaces::InteractionHttpMethod::Put;
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

fn plugin_registration_with_local_sensitive(provider_id: &str) -> surfaces::SurfaceRegistration {
    let mut registration = plugin_registration(provider_id);
    registration
        .capabilities
        .0
        .insert(surfaces::Capability::SensitiveFields);
    registration.surfaces[0].interactions[0].sensitive_fields = vec!["smtp_password".to_string()];
    registration
}

fn plugin_registration_with_declared_params(provider_id: &str) -> surfaces::SurfaceRegistration {
    let mut registration = plugin_registration(provider_id);
    registration.surfaces[0].interactions[0].params = vec![
        surfaces::ParamFieldDescriptor::new("email", surfaces::SchemaContract::String).required(),
    ];
    registration
}

fn plugin_registration_data_load(provider_id: &str) -> surfaces::SurfaceRegistration {
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
            surfaces::Capability::DataLoad,
        ]),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Tenant,
            tenant_id: Some(tenant_id().to_string()),
        },
        surfaces: vec![surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor::builder()
                .surface_id(
                    surfaces::SurfaceId::new("notifications.email.global_smtp_load").unwrap(),
                )
                .label("SMTP Defaults Load")
                .priority(100)
                .slot(surfaces::SLOT_SETTINGS_BELOW_GLOBAL)
                .scope(surfaces::Scope::Global)
                .targeting(surfaces::Targeting::Universal)
                .provider_kind(surfaces::ProviderKind::Plugin)
                .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::TextBlockNode,
                    surfaces::Capability::DataLoad,
                    surfaces::Capability::UniversalTargeting,
                ]))
                .root_node(surfaces::SurfaceNode::TextBlock {
                    text: "ok".to_string(),
                })
                .build(),
            interactions: vec![surfaces::InteractionDescriptor::new(
                surfaces::InteractionId::new("load_global_smtp").unwrap(),
                surfaces::InteractionKind::DataLoad,
                "Load",
                surfaces::InteractionTransport::ControllerLocal,
            )],
            data_sources: vec![],
        }],
        encryption_metadata: None,
    }
}

/// Captures the full `SurfaceActionRequest` handed to a `ControllerLocal`
/// executor so tests can assert the stamped `method` — `PluginSurfaceActionInvoker`
/// (used by `TestPluginInvoker` and friends above) never sees it.
struct CapturingLocalExecutor {
    captured: StdArc<Mutex<Option<surfaces::SurfaceActionRequest>>>,
}

#[async_trait]
impl SurfaceLocalActionExecutor for CapturingLocalExecutor {
    async fn execute(
        &self,
        _resolved: &crate::registry::ResolvedSurfaceAction,
        request: &surfaces::SurfaceActionRequest,
    ) -> Result<serde_json::Value, SurfaceProxyError> {
        *self.captured.lock() = Some(request.clone());
        Ok(serde_json::json!({"ok": true}))
    }
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
        _ctx: SurfaceInvokerContext<'_>,
        _surface_id: &str,
        _interaction_id: &str,
        _method: uptrakit_wire::surfaces::InteractionHttpMethod,
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
        _ctx: SurfaceInvokerContext<'_>,
        _surface_id: &str,
        _interaction_id: &str,
        _method: uptrakit_wire::surfaces::InteractionHttpMethod,
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
        .bootstrap_plugin(plugin_registration("plugin.notifications.email"))
        .expect("plugin registration should succeed");

    let seen = StdArc::new(Mutex::new(Vec::new()));
    let invoker = TestPluginInvoker {
        response: serde_json::json!({"ok": true}),
        seen: StdArc::clone(&seen),
    };
    let proxy = SurfaceProxy::new()
        .with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new_without_database(
            Arc::new(invoker),
        )))
        .with_provider_visibility(Arc::new(AllProvidersVisible));
    let service_connections = ServiceConnectionRegistry::new();

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                method: None,
                tenant_id: tenant_id(),
                surface_id: "notifications.email.global-smtp".to_string(),
                interaction_id: "smtp".to_string(),
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
    assert_eq!(seen[0].0, "notifications.email.global-smtp");
    assert_eq!(seen[0].1, "smtp");
    assert_eq!(seen[0].2, Some(tenant_id()));
    assert_eq!(seen[0].3, Some(user_id()));
}

#[tokio::test(start_paused = true)]
async fn invoke_controller_local_preserves_surface_action_error_categories() {
    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(plugin_registration("plugin.notifications.email"))
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
        let proxy = SurfaceProxy::new()
            .with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new_without_database(
                Arc::new(ErrorPluginInvoker {
                    error: plugin_error,
                }),
            )))
            .with_provider_visibility(Arc::new(AllProvidersVisible));
        let err = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    method: None,
                    tenant_id: tenant_id(),
                    surface_id: "notifications.email.global-smtp".to_string(),
                    interaction_id: "smtp".to_string(),
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
        .bootstrap_plugin(plugin_registration("plugin.notifications.email"))
        .expect("plugin registration should succeed");

    let started = StdArc::new(tokio::sync::Notify::new());
    let release = StdArc::new(tokio::sync::Notify::new());
    let calls = StdArc::new(std::sync::atomic::AtomicUsize::new(0));
    let proxy = Arc::new(
        SurfaceProxy::new()
            .with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new_without_database(
                Arc::new(BlockingPluginInvoker {
                    started: StdArc::clone(&started),
                    release: StdArc::clone(&release),
                    calls: StdArc::clone(&calls),
                }),
            )))
            .with_provider_visibility(Arc::new(AllProvidersVisible)),
    );
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
                    method: None,
                    tenant_id: tenant_id(),
                    surface_id: "notifications.email.global-smtp".to_string(),
                    interaction_id: "smtp".to_string(),
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
                method: None,
                tenant_id: tenant_id(),
                surface_id: "notifications.email.global-smtp".to_string(),
                interaction_id: "smtp".to_string(),
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

#[tokio::test]
async fn controller_local_client_disconnect_releases_idempotency() {
    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(plugin_registration("plugin.notifications.email"))
        .expect("plugin registration should succeed");

    let started = StdArc::new(tokio::sync::Notify::new());
    let release = StdArc::new(tokio::sync::Notify::new());
    let calls = StdArc::new(std::sync::atomic::AtomicUsize::new(0));
    let invoker = BlockingPluginInvoker {
        started: StdArc::clone(&started),
        release: StdArc::clone(&release),
        calls: StdArc::clone(&calls),
    };
    let proxy = Arc::new(
        SurfaceProxy::new()
            .with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new_without_database(
                Arc::new(invoker),
            )))
            .with_provider_visibility(Arc::new(AllProvidersVisible)),
    );
    let service_connections = ServiceConnectionRegistry::new();

    // Register the `started` waiter BEFORE spawning. `BlockingPluginInvoker::invoke`
    // signals via `notify_waiters()` (controller_local.rs:100), which — unlike
    // `notify_one()` — stores NO permit and only wakes waiters already registered at
    // the call instant. If the spawned task reached `notify_waiters()` before the main
    // task polled `started.notified()`, the edge would be lost and this test would HANG
    // (not fail). `Notified::enable()` registers the waiter now, without awaiting,
    // closing the lost-wakeup race.
    let started_fut = started.notified();
    tokio::pin!(started_fut);
    started_fut.as_mut().enable();

    let proxy_clone = Arc::clone(&proxy);
    let handle = tokio::spawn(async move {
        proxy_clone
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    method: None,
                    tenant_id: tenant_id(),
                    surface_id: "notifications.email.global-smtp".to_string(),
                    interaction_id: "smtp".to_string(),
                    idempotency_key: "idem-local-cancel".to_string(),
                    target_provider_id: None,
                    caller_origin: SurfaceCallerOrigin::UserSession {
                        user_id: user_id(),
                        session_id: "session-1".to_string(),
                    },
                    params: serde_json::Map::new(),
                    encrypted_sensitive_params: None,
                },
                Some(Duration::from_secs(30)),
            )
            .await
    });

    // Wait until execute() has entered — reservation is live (non-vacuous baseline).
    started_fut.await;
    assert_eq!(
        proxy.pending.lock().in_flight_idempotency.len(),
        1,
        "idempotency reservation must be live while the plugin is executing"
    );

    // Client disconnects: drop the invoke future mid-execute().
    handle.abort();
    let _ = handle.await;

    assert!(
        proxy.pending.lock().in_flight_idempotency.is_empty(),
        "idempotency reservation leaked after ControllerLocal cancel"
    );
}

#[tokio::test(start_paused = true)]
async fn invoke_controller_local_allows_cleartext_sensitive_fields() {
    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(plugin_registration_with_local_sensitive(
            "plugin.notifications.email",
        ))
        .expect("plugin registration should succeed");

    let invoker = TestPluginInvoker {
        response: serde_json::json!({"ok": true}),
        seen: StdArc::new(Mutex::new(Vec::new())),
    };
    let proxy = SurfaceProxy::new()
        .with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new_without_database(
            Arc::new(invoker),
        )))
        .with_provider_visibility(Arc::new(AllProvidersVisible));
    let service_connections = ServiceConnectionRegistry::new();
    let mut params = serde_json::Map::new();
    params.insert("smtp_password".to_string(), serde_json::json!("clear"));

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                method: None,
                tenant_id: tenant_id(),
                surface_id: "notifications.email.global-smtp".to_string(),
                interaction_id: "smtp".to_string(),
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

#[tokio::test(start_paused = true)]
async fn invoke_stamps_effective_get_method_for_data_load_controller_local() {
    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(plugin_registration_data_load(
            "plugin.notifications.email-load",
        ))
        .expect("plugin registration should succeed");

    let captured = StdArc::new(Mutex::new(None));
    let proxy = SurfaceProxy::new()
        .with_local_executor(Arc::new(CapturingLocalExecutor {
            captured: StdArc::clone(&captured),
        }))
        .with_provider_visibility(Arc::new(AllProvidersVisible));
    let service_connections = ServiceConnectionRegistry::new();

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                method: None,
                tenant_id: tenant_id(),
                surface_id: "notifications.email.global_smtp_load".to_string(),
                interaction_id: "load_global_smtp".to_string(),
                idempotency_key: "idem-data-load-method".to_string(),
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
        .expect("data-load controller-local interaction should succeed");

    assert!(response.success);
    let captured_request = captured
        .lock()
        .clone()
        .expect("executor should capture the dispatched request");
    assert_eq!(
        captured_request.method,
        surfaces::InteractionHttpMethod::Get
    );
}

#[tokio::test(start_paused = true)]
async fn invoke_rejects_body_missing_required_declared_param() {
    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(plugin_registration_with_declared_params(
            "plugin.notifications.email-params",
        ))
        .expect("plugin registration should succeed");

    let invoker = TestPluginInvoker {
        response: serde_json::json!({"ok": true}),
        seen: StdArc::new(Mutex::new(Vec::new())),
    };
    let proxy = SurfaceProxy::new()
        .with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new_without_database(
            Arc::new(invoker),
        )))
        .with_provider_visibility(Arc::new(AllProvidersVisible));
    let service_connections = ServiceConnectionRegistry::new();

    let err = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                method: None,
                tenant_id: tenant_id(),
                surface_id: "notifications.email.global-smtp".to_string(),
                interaction_id: "smtp".to_string(),
                idempotency_key: "idem-missing-required-param".to_string(),
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
        .expect_err("missing required declared param must be rejected");

    assert!(matches!(err, SurfaceProxyError::SchemaValidationFailed(_)));
}

#[tokio::test(start_paused = true)]
async fn invoke_allows_undeclared_body_key_to_pass_through() {
    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(plugin_registration_with_declared_params(
            "plugin.notifications.email-params",
        ))
        .expect("plugin registration should succeed");

    let seen = StdArc::new(Mutex::new(Vec::new()));
    let invoker = TestPluginInvoker {
        response: serde_json::json!({"ok": true}),
        seen: StdArc::clone(&seen),
    };
    let proxy = SurfaceProxy::new()
        .with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new_without_database(
            Arc::new(invoker),
        )))
        .with_provider_visibility(Arc::new(AllProvidersVisible));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = serde_json::Map::new();
    params.insert("email".to_string(), serde_json::json!("admin@example.test"));
    params.insert("undeclared_key".to_string(), serde_json::json!("passes"));

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                method: None,
                tenant_id: tenant_id(),
                surface_id: "notifications.email.global-smtp".to_string(),
                interaction_id: "smtp".to_string(),
                idempotency_key: "idem-undeclared-key".to_string(),
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
        .expect("undeclared body keys must pass through untyped");

    assert!(response.success);
}

#[tokio::test]
async fn invoke_denies_plugin_controller_local_interaction_without_provider_visibility() {
    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(plugin_registration("plugin.notifications.email"))
        .expect("plugin registration should succeed");

    let seen = StdArc::new(Mutex::new(Vec::new()));
    let invoker = TestPluginInvoker {
        response: serde_json::json!({"ok": true}),
        seen: StdArc::clone(&seen),
    };
    // Deliberately constructed WITHOUT `.with_provider_visibility(...)` — the proxy's
    // fail-closed default (`DenyAllPluginProviders`) must hide the Plugin-kind provider
    // registered above, even though a local executor is wired and would otherwise
    // happily service the request.
    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new_without_database(Arc::new(invoker)),
    ));
    let service_connections = ServiceConnectionRegistry::new();

    let err = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                method: None,
                tenant_id: tenant_id(),
                surface_id: "notifications.email.global-smtp".to_string(),
                interaction_id: "smtp".to_string(),
                idempotency_key: "idem-plugin-local-deny-default".to_string(),
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
        .expect_err("a Plugin-kind provider must be denied without explicit visibility");

    assert!(
        matches!(err, SurfaceProxyError::NoProvider),
        "fail-closed default must surface NoProvider, got {err:?}"
    );
    assert!(
        seen.lock().is_empty(),
        "the local executor must never be reached when the provider is hidden by the deny-all default"
    );
}
