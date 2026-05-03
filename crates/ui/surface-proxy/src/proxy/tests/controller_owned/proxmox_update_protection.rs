use std::sync::Arc;
use std::time::Duration;

use uptrakit_plugin_infrastructure_registry::PluginOps;
use uptrakit_wire::surfaces;
use uuid::Uuid;

use super::super::super::{
    PluginSurfaceLocalExecutor, ServiceConnectionRegistry, SurfaceCallerOrigin,
    SurfaceInvokeRequest, SurfaceProxy,
};
use super::super::{tenant_id, user_id};
use super::ensure_master_key;
use crate::registry::{SurfaceRegistry, SurfaceRegistryConfig};

fn proxmox_update_protection_registration(
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
                .label("Update Protection")
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

async fn setup_proxmox_db() -> sea_orm::DatabaseConnection {
    use sea_orm::{ConnectOptions, Database};

    let opt = ConnectOptions::new("sqlite::memory:".to_owned());
    let db = Database::connect(opt).await.expect("test db");
    uptrakit_shared_db::migration::run_migrations_with_plugins(
        &db,
        uptrakit_plugin_infrastructure_proxmox::ProxmoxPlugin::controller_migrations(),
    )
    .await
    .expect("shared + proxmox migrations should run");
    super::insert_tenant(&db, super::super::tenant_id()).await;
    db
}

async fn insert_active_proxmox_plugin_config(db: &sea_orm::DatabaseConnection) -> Uuid {
    use sea_orm::{ActiveModelTrait, Set};
    use uptrakit_shared_db::entity::plugin_config;

    let id = Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();
    plugin_config::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id()),
        name: Set("test-proxmox".to_string()),
        plugin_type: Set("infrastructure_proxmox".to_string()),
        config: Set(serde_json::json!({
            "api_url": "https://pve.test:8006",
            "api_token": "tok",
            "verify_tls": true,
            "node_filter": []
        })),
        enabled: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: sea_orm::ActiveValue::NotSet,
    }
    .insert(db)
    .await
    .expect("insert proxmox plugin_config");
    id
}

#[tokio::test]
async fn invoke_proxmox_save_global_defaults_emits_success_audit_row() {
    ensure_master_key();
    let db = setup_proxmox_db().await;
    let plugin_config_id = insert_active_proxmox_plugin_config(&db).await;
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(
        uptrakit_plugin_infrastructure_registry::build_catalog(
            &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
        )
        .expect("catalog should build"),
    );

    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(proxmox_update_protection_registration(
            "plugin.infrastructure_proxmox",
            "proxmox.settings.update-protection",
            "save-global-defaults",
        ))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new(Arc::new(db.clone()), Arc::clone(&plugin_ops))
            .with_audit_emitter(super::test_audit_emitter(db.clone())),
    ));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = serde_json::Map::new();
    params.insert(
        "plugin_config_id".to_string(),
        serde_json::json!(plugin_config_id.to_string()),
    );
    params.insert("mode".to_string(), serde_json::json!("do_nothing"));

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "proxmox.settings.update-protection".to_string(),
                interaction_id: "save-global-defaults".to_string(),
                idempotency_key: "idem-proxmox-save-global-defaults-success".to_string(),
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
        .expect("save-global-defaults should succeed");

    assert!(response.success);
    let row = super::latest_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::TENANT_SETTING_UPDATE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.actor_id, Some(user_id()));
    assert_eq!(row.target_type.as_deref(), Some("plugin_config"));
    assert_eq!(
        row.target_id.as_deref(),
        Some(plugin_config_id.to_string().as_str())
    );
    let details = row.details_json.expect("audit details");
    assert_eq!(
        details["mutation_source"],
        serde_json::json!("surface_proxy.proxmox_update_protection.save_global_defaults")
    );
    assert_eq!(
        details["plugin_type"],
        serde_json::json!("infrastructure_proxmox")
    );
}

#[tokio::test]
async fn invoke_proxmox_save_item_overrides_emits_software_item_update_audit_row() {
    ensure_master_key();
    let db = setup_proxmox_db().await;
    let plugin_config_id = insert_active_proxmox_plugin_config(&db).await;
    let software_item_id = Uuid::now_v7();
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(
        uptrakit_plugin_infrastructure_registry::build_catalog(
            &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
        )
        .expect("catalog should build"),
    );

    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(proxmox_update_protection_registration(
            "plugin.infrastructure_proxmox",
            "proxmox.software-item.update-protection",
            "save-item-overrides",
        ))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new(Arc::new(db.clone()), Arc::clone(&plugin_ops))
            .with_audit_emitter(super::test_audit_emitter(db.clone())),
    ));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = serde_json::Map::new();
    params.insert(
        "plugin_config_id".to_string(),
        serde_json::json!(plugin_config_id.to_string()),
    );
    params.insert(
        "software_item_id".to_string(),
        serde_json::json!(software_item_id.to_string()),
    );
    params.insert("mode".to_string(), serde_json::json!("do_nothing"));

    // Invoke — may succeed or fail depending on plugin state; audit must be emitted either way.
    let _ = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "proxmox.software-item.update-protection".to_string(),
                interaction_id: "save-item-overrides".to_string(),
                idempotency_key: "idem-proxmox-save-item-overrides-audit".to_string(),
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
        .await;

    let row = super::latest_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE,
    )
    .await;
    assert_eq!(row.actor_id, Some(user_id()));
    assert_eq!(row.target_type.as_deref(), Some("software_item"));
    let details = row.details_json.expect("audit details");
    assert_eq!(
        details["mutation_source"],
        serde_json::json!("surface_proxy.proxmox_update_protection.save_item_overrides")
    );
    assert_eq!(
        details["plugin_type"],
        serde_json::json!("infrastructure_proxmox")
    );
}
