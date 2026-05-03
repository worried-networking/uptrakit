use std::sync::Arc;
use std::time::Duration;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Map, json};
use uptrakit_plugin_infrastructure_registry::PluginOps;
use uptrakit_wire::surfaces;

use super::super::super::{
    PluginOpsSurfaceActionInvoker, PluginSurfaceLocalExecutor, ServiceConnectionRegistry,
    SurfaceCallerOrigin, SurfaceInvokeRequest, SurfaceProxy, SurfaceProxyError,
};
use super::super::{tenant_id, user_id};
use super::{ensure_master_key, setup_notification_db};
use crate::registry::{SurfaceRegistry, SurfaceRegistryConfig};

fn proxmox_hosts_registration(provider_id: &str) -> surfaces::SurfaceRegistration {
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
            descriptor: surfaces::SurfaceDescriptor {
                surface_id: surfaces::SurfaceId::new("proxmox.hosts").unwrap(),
                label: "Proxmox Hosts".to_string(),
                priority: 100,
                slot: surfaces::SLOT_SETTINGS_TABS.to_string(),
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
                interaction_id: surfaces::InteractionId::new("add-config").unwrap(),
                kind: surfaces::InteractionKind::FormSubmit,
                label: None,
                required_permission: Some("manage_commands".to_string()),
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
async fn invoke_proxmox_add_config_executes_controller_owned_create_path() {
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
        .bootstrap_plugin(proxmox_hosts_registration("plugin.infrastructure_proxmox"))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new(
        Arc::new(db.clone()),
        Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
    )));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = Map::new();
    params.insert("name".to_string(), json!("PVE Cluster"));
    params.insert("api_url".to_string(), json!("https://pve.local:8006"));
    params.insert(
        "api_token".to_string(),
        json!("root@pam!uptrakit=secret-token"),
    );
    params.insert("verify_tls".to_string(), json!(false));
    params.insert("node_filter".to_string(), json!(" node-a, , node-b "));

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "proxmox.hosts".to_string(),
                interaction_id: "add-config".to_string(),
                idempotency_key: "idem-proxmox-add-config".to_string(),
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
        .expect("proxmox add-config should execute on the controller-owned create path");

    assert!(response.success);
    let result = response
        .result
        .expect("proxmox add-config should return created plugin-config payload");
    assert_eq!(result["name"], "PVE Cluster");
    assert_eq!(result["plugin_type"], "infrastructure_proxmox");
    assert_eq!(result["enabled"], true);
    assert_eq!(result["config"]["api_url"], "https://pve.local:8006");
    assert_eq!(result["config"]["verify_tls"], false);
    assert_eq!(result["config"]["node_filter"], json!(["node-a", "node-b"]));

    let persisted = uptrakit_shared_db::entity::plugin_config::Entity::find()
        .filter(uptrakit_shared_db::entity::plugin_config::Column::TenantId.eq(tenant_id()))
        .one(&db)
        .await
        .expect("plugin config query should succeed")
        .expect("proxmox add-config should create a plugin config row");
    assert_eq!(persisted.name, "PVE Cluster");
    assert_eq!(persisted.plugin_type, "infrastructure_proxmox");
    assert!(persisted.enabled);
}

#[tokio::test]
async fn invoke_proxmox_add_config_accepts_legacy_string_verify_tls_values() {
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
        .bootstrap_plugin(proxmox_hosts_registration("plugin.infrastructure_proxmox"))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new(
        Arc::new(db),
        Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
    )));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = Map::new();
    params.insert("name".to_string(), json!("PVE Cluster"));
    params.insert("api_url".to_string(), json!("https://pve.local:8006"));
    params.insert(
        "api_token".to_string(),
        json!("root@pam!uptrakit=secret-token"),
    );
    params.insert("verify_tls".to_string(), json!("false"));

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "proxmox.hosts".to_string(),
                interaction_id: "add-config".to_string(),
                idempotency_key: "idem-proxmox-add-config-invalid-verify".to_string(),
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
        .expect("legacy string verify_tls should remain accepted");

    assert!(response.success);
    let result = response
        .result
        .expect("legacy string verify_tls should return created payload");
    assert_eq!(result["config"]["verify_tls"], false);
}

#[tokio::test]
async fn invoke_proxmox_add_config_rejects_invalid_verify_tls_type() {
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
        .bootstrap_plugin(proxmox_hosts_registration("plugin.infrastructure_proxmox"))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new(
        Arc::new(db),
        Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
    )));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = Map::new();
    params.insert("name".to_string(), json!("PVE Cluster"));
    params.insert("api_url".to_string(), json!("https://pve.local:8006"));
    params.insert(
        "api_token".to_string(),
        json!("root@pam!uptrakit=secret-token"),
    );
    params.insert("verify_tls".to_string(), json!("definitely-not-bool"));

    let err = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "proxmox.hosts".to_string(),
                interaction_id: "add-config".to_string(),
                idempotency_key: "idem-proxmox-add-config-invalid-verify".to_string(),
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
        .expect_err("invalid verify_tls string should be rejected");

    let SurfaceProxyError::SchemaValidationFailed(message) = err else {
        panic!("unexpected error variant: {err:?}");
    };
    assert!(
        message.contains("verify_tls"),
        "expected verify_tls validation error, got: {message}"
    );
}

#[tokio::test]
async fn invoke_proxmox_add_config_rejects_invalid_node_filter_type() {
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
        .bootstrap_plugin(proxmox_hosts_registration("plugin.infrastructure_proxmox"))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new(
        Arc::new(db),
        Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
    )));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = Map::new();
    params.insert("name".to_string(), json!("PVE Cluster"));
    params.insert("api_url".to_string(), json!("https://pve.local:8006"));
    params.insert(
        "api_token".to_string(),
        json!("root@pam!uptrakit=secret-token"),
    );
    params.insert("verify_tls".to_string(), json!(true));
    params.insert("node_filter".to_string(), json!(123));

    let err = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "proxmox.hosts".to_string(),
                interaction_id: "add-config".to_string(),
                idempotency_key: "idem-proxmox-add-config-invalid-node-filter".to_string(),
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
        .expect_err("non-string/array node_filter should be rejected");

    let SurfaceProxyError::SchemaValidationFailed(message) = err else {
        panic!("unexpected error variant: {err:?}");
    };
    assert!(
        message.contains("node_filter"),
        "expected node_filter validation error, got: {message}"
    );
}

#[tokio::test]
async fn invoke_proxmox_add_config_preserves_duplicate_name_conflict() {
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
        .bootstrap_plugin(proxmox_hosts_registration("plugin.infrastructure_proxmox"))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new(
        Arc::new(db),
        Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
    )));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = Map::new();
    params.insert("name".to_string(), json!("PVE Cluster"));
    params.insert("api_url".to_string(), json!("https://pve.local:8006"));
    params.insert(
        "api_token".to_string(),
        json!("root@pam!uptrakit=secret-token"),
    );

    proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "proxmox.hosts".to_string(),
                interaction_id: "add-config".to_string(),
                idempotency_key: "idem-proxmox-add-config-1".to_string(),
                target_provider_id: None,
                caller_origin: SurfaceCallerOrigin::UserSession {
                    user_id: user_id(),
                    session_id: "session-1".to_string(),
                },
                params: params.clone(),
                encrypted_sensitive_params: None,
            },
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("first proxmox add-config create should succeed");

    let err = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "proxmox.hosts".to_string(),
                interaction_id: "add-config".to_string(),
                idempotency_key: "idem-proxmox-add-config-2".to_string(),
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
        .expect_err("duplicate proxmox add-config create should fail");

    let SurfaceProxyError::Conflict { code, message } = err else {
        panic!("unexpected error variant: {err:?}");
    };
    assert_eq!(code, "duplicate_name");
    assert!(
        message.contains("already exists"),
        "expected duplicate-name conflict message, got: {message}"
    );
}

#[tokio::test]
async fn invoke_proxmox_add_config_emits_audit_row_when_emitter_is_configured() {
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
        .bootstrap_plugin(proxmox_hosts_registration("plugin.infrastructure_proxmox"))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new(Arc::new(db.clone()), Arc::clone(&plugin_ops))
            .with_audit_emitter(super::test_audit_emitter(db.clone())),
    ));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = Map::new();
    params.insert("name".to_string(), json!("PVE Cluster"));
    params.insert("api_url".to_string(), json!("https://pve.local:8006"));
    params.insert(
        "api_token".to_string(),
        json!("root@pam!uptrakit=secret-token"),
    );
    params.insert("verify_tls".to_string(), json!(false));

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "proxmox.hosts".to_string(),
                interaction_id: "add-config".to_string(),
                idempotency_key: "idem-proxmox-add-config-audit".to_string(),
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
        .expect("proxmox add-config should succeed");

    assert!(response.success);
    let row = super::latest_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.actor_id, Some(user_id()));
    assert_eq!(row.target_type.as_deref(), Some("plugin_config"));
    let details = row.details_json.expect("audit details");
    assert_eq!(
        details["create_source"],
        json!("surface_proxy.proxmox_add_config")
    );
    assert_eq!(details["plugin_type"], json!("infrastructure_proxmox"));
}

#[tokio::test]
async fn invoke_proxmox_add_config_validation_failure_emits_validation_failed_audit_row() {
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
        .bootstrap_plugin(proxmox_hosts_registration("plugin.infrastructure_proxmox"))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new(Arc::new(db.clone()), Arc::clone(&plugin_ops))
            .with_audit_emitter(super::test_audit_emitter(db.clone())),
    ));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = Map::new();
    params.insert("name".to_string(), json!("PVE Cluster"));
    params.insert("api_url".to_string(), json!("https://pve.local:8006"));
    params.insert(
        "api_token".to_string(),
        json!("root@pam!uptrakit=secret-token"),
    );
    params.insert("verify_tls".to_string(), json!("definitely-not-bool"));

    let err = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "proxmox.hosts".to_string(),
                interaction_id: "add-config".to_string(),
                idempotency_key: "idem-proxmox-add-config-audit-validation-failed".to_string(),
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
        .expect_err("invalid verify_tls should be rejected");
    assert!(matches!(err, SurfaceProxyError::SchemaValidationFailed(_)));

    let row = super::latest_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
    );
    let details = row.details_json.expect("audit details");
    assert_eq!(
        details["create_source"],
        json!("surface_proxy.proxmox_add_config")
    );
    assert_eq!(details["reason_code"], json!("validation_failed"));
}

#[tokio::test]
async fn invoke_proxmox_add_config_duplicate_conflict_emits_failed_audit_row() {
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
        .bootstrap_plugin(proxmox_hosts_registration("plugin.infrastructure_proxmox"))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new(Arc::new(db.clone()), Arc::clone(&plugin_ops))
            .with_audit_emitter(super::test_audit_emitter(db.clone())),
    ));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = Map::new();
    params.insert("name".to_string(), json!("PVE Cluster"));
    params.insert("api_url".to_string(), json!("https://pve.local:8006"));
    params.insert(
        "api_token".to_string(),
        json!("root@pam!uptrakit=secret-token"),
    );

    proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "proxmox.hosts".to_string(),
                interaction_id: "add-config".to_string(),
                idempotency_key: "idem-proxmox-add-config-audit-conflict-first".to_string(),
                target_provider_id: None,
                caller_origin: SurfaceCallerOrigin::UserSession {
                    user_id: user_id(),
                    session_id: "session-1".to_string(),
                },
                params: params.clone(),
                encrypted_sensitive_params: None,
            },
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("initial create should succeed");

    let err = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "proxmox.hosts".to_string(),
                interaction_id: "add-config".to_string(),
                idempotency_key: "idem-proxmox-add-config-audit-conflict-second".to_string(),
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
        .expect_err("duplicate create should fail");
    assert!(matches!(err, SurfaceProxyError::Conflict { .. }));

    let row = super::latest_tenant_audit_row_for_action_and_outcome(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
        uptrakit_audit_log::AuditOutcome::Failed,
    )
    .await;
    let details = row.details_json.expect("audit details");
    assert_eq!(
        details["create_source"],
        json!("surface_proxy.proxmox_add_config")
    );
    assert_eq!(details["reason_code"], json!("duplicate_name"));
    assert_eq!(details["error_kind"], json!("conflict"));
}
