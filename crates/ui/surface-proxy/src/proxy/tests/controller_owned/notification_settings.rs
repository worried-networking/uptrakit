use std::sync::Arc;
use std::sync::Arc as StdArc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use uptrakit_plugin_infrastructure_registry::{PluginOps, SurfaceActionError};
use uptrakit_wire::surfaces;

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
        _tenant_id: Option<uuid::Uuid>,
        _caller_user_id: Option<uuid::Uuid>,
        _surface_id: &str,
        _interaction_id: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, SurfaceActionError> {
        Err(SurfaceActionError::InvalidInput(self.error_message.clone()))
    }
}

fn notification_settings_registration(
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
        ]),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Global,
            tenant_id: None,
        },
        surfaces: vec![surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor {
                surface_id: surfaces::SurfaceId::new(surface_id).unwrap(),
                label: "Settings".to_string(),
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
                interaction_id: surfaces::InteractionId::new(interaction_id).unwrap(),
                kind: surfaces::InteractionKind::FormSubmit,
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

#[cfg(feature = "notifications-email")]
#[tokio::test]
async fn invoke_notifications_email_save_global_smtp_emits_global_setting_update_audit() {
    ensure_master_key();
    let db = setup_notification_db().await;
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
        .bootstrap_plugin(notification_settings_registration(
            "plugin.email",
            "notifications.email.global_smtp",
            "save_global_smtp",
        ))
        .expect("plugin registration should succeed");

    let mut params = serde_json::Map::new();
    params.insert("host".to_string(), serde_json::json!("smtp.global.example"));
    params.insert(
        "smtp_password".to_string(),
        serde_json::json!("secret-value"),
    );

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "notifications.email.global_smtp".to_string(),
                interaction_id: "save_global_smtp".to_string(),
                idempotency_key: "idem-global-smtp-audit".to_string(),
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
        .expect("save_global_smtp should succeed");
    assert!(response.success);

    {
        let seen = seen.lock();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "notifications.email.global_smtp");
        assert_eq!(seen[0].1, "save_global_smtp");
    }

    let row = super::latest_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::GLOBAL_SETTING_UPDATE,
    )
    .await;
    assert_eq!(row.tenant_id, tenant_id());
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.actor_id, Some(user_id()));
    assert_eq!(row.target_type.as_deref(), Some("global_setting"));
    assert_eq!(row.target_id.as_deref(), Some("global_smtp"));
    let details = row.details_json.expect("audit details");
    assert_eq!(
        details["mutation_source"],
        serde_json::json!("surface_proxy.notification_settings.save_global_smtp")
    );
    assert_eq!(details["setting_scope"], serde_json::json!("global"));
    assert_eq!(details["setting_area"], serde_json::json!("global_smtp"));
    assert!(
        !details.to_string().contains("secret-value"),
        "audit details must never include raw secret values"
    );
}

#[cfg(feature = "notifications-telegram")]
#[tokio::test]
async fn invoke_notifications_telegram_save_global_telegram_failure_emits_failed_audit() {
    ensure_master_key();
    let db = setup_notification_db().await;
    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new_without_database(Arc::new(ErrorPluginInvoker {
            error_message: "Internal server error".to_string(),
        }))
        .with_audit_emitter(super::test_audit_emitter(db.clone())),
    ));
    let service_connections = ServiceConnectionRegistry::new();
    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(notification_settings_registration(
            "plugin.telegram",
            "notifications.telegram.global_settings",
            "save_global_telegram",
        ))
        .expect("plugin registration should succeed");

    let mut params = serde_json::Map::new();
    params.insert(
        "bot_token".to_string(),
        serde_json::json!("123456:super-secret"),
    );

    let err = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "notifications.telegram.global_settings".to_string(),
                interaction_id: "save_global_telegram".to_string(),
                idempotency_key: "idem-global-telegram-audit-failure".to_string(),
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
        .expect_err("save_global_telegram should fail");
    assert!(matches!(err, SurfaceProxyError::SchemaValidationFailed(_)));

    let row = super::latest_tenant_audit_row_for_action_and_outcome(
        &db,
        uptrakit_audit_log::AuditActionType::GLOBAL_SETTING_UPDATE,
        uptrakit_audit_log::AuditOutcome::Failed,
    )
    .await;
    assert_eq!(row.tenant_id, tenant_id());
    assert_eq!(row.actor_id, Some(user_id()));
    assert_eq!(row.target_type.as_deref(), Some("global_setting"));
    assert_eq!(row.target_id.as_deref(), Some("global_telegram"));
    let details = row.details_json.expect("audit details");
    assert_eq!(
        details["mutation_source"],
        serde_json::json!("surface_proxy.notification_settings.save_global_telegram")
    );
    assert_eq!(details["reason_code"], serde_json::json!("storage_error"));
    assert!(
        !details.to_string().contains("123456:super-secret"),
        "audit details must never include raw secret values"
    );
}
