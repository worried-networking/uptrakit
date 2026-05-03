use std::sync::Arc;
use std::sync::Arc as StdArc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use uptrakit_plugin_infrastructure_registry::SurfaceActionError;
use uptrakit_wire::surfaces;
use uuid::Uuid;

use super::super::super::{
    PluginSurfaceActionInvoker, PluginSurfaceLocalExecutor, ServiceConnectionRegistry,
    SurfaceCallerOrigin, SurfaceInvokeRequest, SurfaceProxy, SurfaceProxyError,
};
use super::super::{TestPluginInvoker, tenant_id, user_id};
use super::{ensure_master_key, setup_notification_db};
use crate::registry::{SurfaceRegistry, SurfaceRegistryConfig};

struct ErrorPluginInvoker {
    error_message: String,
}

#[async_trait]
impl PluginSurfaceActionInvoker for ErrorPluginInvoker {
    async fn invoke(
        &self,
        _db: Option<&sea_orm::DatabaseConnection>,
        _tenant_id: Option<Uuid>,
        _caller_user_id: Option<Uuid>,
        _surface_id: &str,
        _interaction_id: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, SurfaceActionError> {
        Err(SurfaceActionError::InvalidInput(self.error_message.clone()))
    }
}

fn docker_switch_tag_registration(provider_id: &str) -> surfaces::SurfaceRegistration {
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
            scope: surfaces::Scope::Global,
            tenant_id: None,
        },
        surfaces: vec![surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor {
                surface_id: surfaces::SurfaceId::new("docker.item-host-actions").unwrap(),
                label: "Docker Actions".to_string(),
                priority: 100,
                slot: "software.actions".to_string(),
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
                interaction_id: surfaces::InteractionId::new("switch-tag").unwrap(),
                kind: surfaces::InteractionKind::MutationAction,
                label: None,
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
async fn invoke_docker_switch_tag_success_emits_software_item_update_audit_row() {
    ensure_master_key();
    let db = setup_notification_db().await;
    let software_item_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();
    let seen = StdArc::new(Mutex::new(Vec::new()));
    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new_without_database(Arc::new(TestPluginInvoker {
            response: serde_json::json!({"ok": true}),
            seen: StdArc::clone(&seen),
        }))
        .with_audit_emitter(super::test_audit_emitter(db.clone())),
    ));
    let service_connections = ServiceConnectionRegistry::new();
    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(docker_switch_tag_registration("plugin.releases_docker"))
        .expect("plugin registration should succeed");

    let mut params = serde_json::Map::new();
    params.insert(
        "software_item_id".to_string(),
        serde_json::json!(software_item_id.to_string()),
    );
    params.insert(
        "host_id".to_string(),
        serde_json::json!(host_id.to_string()),
    );
    params.insert(
        "new_image_ref".to_string(),
        serde_json::json!("ghcr.io/example/app:26.2.6"),
    );

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "docker.item-host-actions".to_string(),
                interaction_id: "switch-tag".to_string(),
                idempotency_key: "idem-docker-switch-tag-success".to_string(),
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
        .expect("switch-tag should succeed");

    assert!(response.success);
    {
        let seen = seen.lock();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "docker.item-host-actions");
        assert_eq!(seen[0].1, "switch-tag");
    }

    let row = super::latest_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.actor_id, Some(user_id()));
    assert_eq!(row.target_type.as_deref(), Some("software_item"));
    assert_eq!(
        row.target_id.as_deref(),
        Some(software_item_id.to_string().as_str())
    );
    let details = row.details_json.expect("audit details");
    assert_eq!(
        details["mutation_source"],
        serde_json::json!("surface_proxy.docker_switch_tag")
    );
    assert_eq!(details["host_id"], serde_json::json!(host_id.to_string()));
    assert_eq!(
        details["new_image_ref"],
        serde_json::json!("ghcr.io/example/app:26.2.6")
    );
}

#[tokio::test]
async fn invoke_docker_switch_tag_invalid_image_emits_validation_failed_audit_row() {
    ensure_master_key();
    let db = setup_notification_db().await;
    let software_item_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();
    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new_without_database(Arc::new(ErrorPluginInvoker {
            error_message: "invalid image reference: bad tag".to_string(),
        }))
        .with_audit_emitter(super::test_audit_emitter(db.clone())),
    ));
    let service_connections = ServiceConnectionRegistry::new();
    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(docker_switch_tag_registration("plugin.releases_docker"))
        .expect("plugin registration should succeed");

    let mut params = serde_json::Map::new();
    params.insert(
        "software_item_id".to_string(),
        serde_json::json!(software_item_id.to_string()),
    );
    params.insert(
        "host_id".to_string(),
        serde_json::json!(host_id.to_string()),
    );
    params.insert("new_image_ref".to_string(), serde_json::json!("bad ref"));

    let err = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "docker.item-host-actions".to_string(),
                interaction_id: "switch-tag".to_string(),
                idempotency_key: "idem-docker-switch-tag-invalid".to_string(),
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
        .expect_err("switch-tag should fail");
    assert!(matches!(err, SurfaceProxyError::SchemaValidationFailed(_)));

    let row = super::latest_tenant_audit_row_for_action_and_outcome(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE,
        uptrakit_audit_log::AuditOutcome::ValidationFailed,
    )
    .await;
    assert_eq!(row.target_type.as_deref(), Some("software_item"));
    assert_eq!(
        row.target_id.as_deref(),
        Some(software_item_id.to_string().as_str())
    );
    let details = row.details_json.expect("audit details");
    assert_eq!(
        details["mutation_source"],
        serde_json::json!("surface_proxy.docker_switch_tag")
    );
    assert_eq!(details["host_id"], serde_json::json!(host_id.to_string()));
    assert_eq!(details["reason_code"], serde_json::json!("invalid_request"));
    assert_eq!(details["new_image_ref"], serde_json::json!("bad ref"));
}
