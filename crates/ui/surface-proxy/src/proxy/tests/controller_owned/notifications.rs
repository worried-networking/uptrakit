use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, json};
use uptrakit_plugin_infrastructure_registry::PluginOps;
use uptrakit_wire::surfaces;
use uuid::Uuid;

use super::super::super::controller_local::{
    build_notification_channel_create_request, build_notification_channel_update_request,
};
use super::super::super::{
    PluginSurfaceLocalExecutor, ServiceConnectionRegistry, SurfaceCallerOrigin,
    SurfaceInvokeRequest, SurfaceProxy, SurfaceProxyError,
};
use super::super::{tenant_id, user_id};
use super::{ensure_master_key, setup_notification_db};
use crate::registry::{SurfaceRegistry, SurfaceRegistryConfig};

fn notification_channel_registration(
    provider_id: &str,
    surface_id: &str,
    interaction_id: &str,
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
            surfaces::Capability::FormSubmit,
        ]),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Global,
            tenant_id: None,
        },
        surfaces: vec![surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor::builder()
                .surface_id(surfaces::SurfaceId::new(surface_id).unwrap())
                .label("Notification Channels")
                .priority(100)
                .slot(surfaces::SLOT_SETTINGS_TABS)
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
            interactions: vec![surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new(interaction_id).unwrap(),
                kind: surfaces::InteractionKind::FormSubmit,
                label: "Action".to_string(),
                required_permission: None,
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
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

#[tokio::test]
async fn invoke_allowlisted_notification_create_executes_controller_owned_path() {
    ensure_master_key();
    let db = setup_notification_db().await;
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(
        uptrakit_plugin_infrastructure_registry::build_catalog(
            &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
        )
        .expect("catalog should build"),
    );

    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(notification_channel_registration(
            "plugin.webhook",
            "notifications.webhook",
            "create",
        ))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new(
        Arc::new(db),
        Arc::clone(&plugin_ops),
    )));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = Map::new();
    params.insert("name".to_string(), json!("Ops Hook"));
    params.insert("channel_type".to_string(), json!("webhook"));
    params.insert(
        "config".to_string(),
        json!({
            "url": "https://example.invalid/hook"
        }),
    );
    params.insert("enabled".to_string(), json!(true));

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "notifications.webhook".to_string(),
                interaction_id: "create".to_string(),
                idempotency_key: "idem-notification-create".to_string(),
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
        .expect("allowlisted notification create should execute locally");

    assert!(response.success);
    let result = response
        .result
        .expect("notification create should return a payload");
    assert_eq!(result["channel_type"], "webhook");
    assert_eq!(result["name"], "Ops Hook");
}

#[cfg(feature = "notifications-email")]
#[tokio::test]
async fn invoke_notifications_email_configure_smtp_executes_controller_local_path() {
    ensure_master_key();
    let db = setup_notification_db().await;
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(
        uptrakit_plugin_infrastructure_registry::build_catalog(
            &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
        )
        .expect("catalog should build"),
    );

    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(notification_channel_registration(
            "plugin.notifications_email",
            "notifications.email",
            "configure_smtp",
        ))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new(
        Arc::new(db),
        Arc::clone(&plugin_ops),
    )));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = Map::new();
    params.insert("host".to_string(), json!("smtp.tenant.example"));
    params.insert("port".to_string(), json!(2525));
    params.insert("from_address".to_string(), json!("alerts@example.com"));
    params.insert("tls_mode".to_string(), json!("starttls"));

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "notifications.email".to_string(),
                interaction_id: "configure_smtp".to_string(),
                idempotency_key: "idem-email-configure-smtp".to_string(),
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
        .expect("configure_smtp should execute locally");

    assert!(response.success);
    let result = response
        .result
        .expect("configure_smtp should return a payload");
    assert_eq!(result["host"], "smtp.tenant.example");
    assert_eq!(result["port"], 2525);
    assert_eq!(result["from_address"], "alerts@example.com");
}

#[test]
fn build_notification_channel_create_request_rejects_non_boolean_enabled() {
    let params = json!({
        "name": "Ops Hook",
        "url": "https://example.invalid/hook",
        "enabled": { "bad": true }
    });
    let params = params.as_object().expect("params should be an object");

    let result = build_notification_channel_create_request("webhook", params);
    let err = result.expect_err("non-boolean enabled must be rejected");
    assert!(
        err.contains("enabled"),
        "expected enabled validation error, got: {err}"
    );
}

#[test]
fn build_notification_channel_update_request_rejects_non_boolean_enabled() {
    let params = json!({
        "url": "https://example.invalid/hook",
        "enabled": 1
    });
    let params = params.as_object().expect("params should be an object");

    let result = build_notification_channel_update_request("webhook", params);
    let err = result.expect_err("non-boolean enabled must be rejected");
    assert!(
        err.contains("enabled"),
        "expected enabled validation error, got: {err}"
    );
}

#[test]
fn build_notification_channel_requests_normalize_email_to_addresses_from_nested_config() {
    let create_params = json!({
        "name": "Email Alerts",
        "channel_type": "email",
        "config": {
            "to_addresses": "alice@example.com\nbob@example.com"
        },
        "enabled": true
    });
    let create_params = create_params
        .as_object()
        .expect("create params should be an object");
    let create_request = build_notification_channel_create_request("email", create_params)
        .expect("create request should build");
    let expected_create_config = json!({
        "to_addresses": ["alice@example.com", "bob@example.com"]
    });
    assert_eq!(
        create_request.config.as_value(),
        &expected_create_config,
        "nested email config textarea input must be normalized to array for create"
    );

    let update_params = json!({
        "id": Uuid::now_v7().to_string(),
        "config": {
            "to_addresses": "carol@example.com\ndave@example.com"
        }
    });
    let update_params = update_params
        .as_object()
        .expect("update params should be an object");
    let update_request = build_notification_channel_update_request("email", update_params)
        .expect("update request should build");
    let expected_update_config = json!({
        "to_addresses": ["carol@example.com", "dave@example.com"]
    });
    assert_eq!(
        update_request
            .config
            .as_ref()
            .map(|config| config.as_value()),
        Some(&expected_update_config),
        "nested email config textarea input must be normalized to array for update"
    );
}

#[tokio::test]
async fn invoke_allowlisted_notification_row_actions_use_controller_owned_path() {
    ensure_master_key();
    let db = setup_notification_db().await;
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(
        uptrakit_plugin_infrastructure_registry::build_catalog(
            &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
        )
        .expect("catalog should build"),
    );

    let service_connections = ServiceConnectionRegistry::new();
    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new(
        Arc::new(db),
        Arc::clone(&plugin_ops),
    )));

    for interaction_id in ["edit", "test", "delete"] {
        let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
        registry
            .bootstrap_plugin(notification_channel_registration(
                "plugin.webhook",
                "notifications.webhook",
                interaction_id,
            ))
            .expect("plugin registration should succeed");

        let mut params = Map::new();
        params.insert("id".to_string(), json!(Uuid::now_v7().to_string()));
        params.insert("name".to_string(), json!("Updated Hook"));
        params.insert("url".to_string(), json!("https://example.invalid/updated"));
        params.insert("enabled".to_string(), json!(true));

        let err = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "notifications.webhook".to_string(),
                    interaction_id: interaction_id.to_string(),
                    idempotency_key: format!("idem-notification-{interaction_id}"),
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
            .expect_err("row action on missing channel should fail");

        let SurfaceProxyError::SchemaValidationFailed(message) = err else {
            panic!("unexpected error type for {interaction_id}: {err:?}");
        };
        assert!(
            message.contains("Channel not found"),
            "expected controller-owned not-found for {interaction_id}, got: {message}"
        );
    }
}

#[tokio::test]
async fn invoke_allowlisted_notification_create_emits_audit_row() {
    ensure_master_key();
    let db = setup_notification_db().await;
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(
        uptrakit_plugin_infrastructure_registry::build_catalog(
            &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
        )
        .expect("catalog should build"),
    );

    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(notification_channel_registration(
            "plugin.webhook",
            "notifications.webhook",
            "create",
        ))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new(Arc::new(db.clone()), Arc::clone(&plugin_ops))
            .with_audit_emitter(super::test_audit_emitter(db.clone())),
    ));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = serde_json::Map::new();
    params.insert("name".to_string(), serde_json::json!("Ops Hook"));
    params.insert("channel_type".to_string(), serde_json::json!("webhook"));
    params.insert(
        "config".to_string(),
        serde_json::json!({"url": "https://example.invalid/hook"}),
    );
    params.insert("enabled".to_string(), serde_json::json!(true));

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "notifications.webhook".to_string(),
                interaction_id: "create".to_string(),
                idempotency_key: "idem-notification-create-audit".to_string(),
                target_provider_id: None,
                caller_origin: SurfaceCallerOrigin::UserSession {
                    user_id: user_id(),
                    session_id: "session-1".to_string(),
                },
                params,
                encrypted_sensitive_params: None,
            },
            Some(std::time::Duration::from_secs(5)),
        )
        .await
        .expect("notification create should succeed");

    assert!(response.success);

    let row = super::latest_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_CREATE,
    )
    .await;
    assert_eq!(row.tenant_id, tenant_id());
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.actor_id, Some(user_id()));
    assert_eq!(row.target_type.as_deref(), Some("notification_channel"));
    let details = row.details_json.expect("audit details");
    assert_eq!(details["channel_type"], serde_json::json!("webhook"));
    assert_eq!(
        details["action_source"],
        serde_json::json!("surface_proxy.notification_channel.create")
    );
}

#[test]
fn build_notification_channel_requests_pass_config_through() {
    let create_params = serde_json::json!({
        "name": "Email Alerts",
        "channel_type": "email",
        "config": {
            "to_addresses": ["alice@example.com", "bob@example.com"]
        },
        "enabled": true
    });
    let create_params = create_params
        .as_object()
        .expect("create params should be an object");
    let create_request = build_notification_channel_create_request("email", create_params)
        .expect("create request should build");
    assert_eq!(
        create_request.config,
        serde_json::from_value::<uptrakit_web_api_types::notifications::channels::JsonObjectInput>(
            serde_json::json!({
                "to_addresses": ["alice@example.com", "bob@example.com"]
            })
        )
        .expect("valid JsonObjectInput"),
        "config JSON object must be passed through unchanged for create"
    );

    let update_params = serde_json::json!({
        "id": uuid::Uuid::now_v7().to_string(),
        "config": {
            "to_addresses": ["carol@example.com", "dave@example.com"]
        }
    });
    let update_params = update_params
        .as_object()
        .expect("update params should be an object");
    let update_request = build_notification_channel_update_request("email", update_params)
        .expect("update request should build");
    assert_eq!(
        update_request.config,
        Some(
            serde_json::from_value::<
                uptrakit_web_api_types::notifications::channels::JsonObjectInput,
            >(serde_json::json!({
                "to_addresses": ["carol@example.com", "dave@example.com"]
            }))
            .expect("valid JsonObjectInput")
        ),
        "config JSON object must be passed through unchanged for update"
    );
}
