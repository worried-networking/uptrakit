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
use crate::registry::{AllProvidersVisible, SurfaceRegistry, SurfaceRegistryConfig};

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
            interactions: vec![{
                let mut i = surfaces::InteractionDescriptor::new(
                    surfaces::InteractionId::new(interaction_id).unwrap(),
                    surfaces::InteractionKind::FormSubmit,
                    "Action",
                    surfaces::InteractionTransport::ControllerLocal,
                );
                i.http_method = surfaces::InteractionHttpMethod::Put;
                i.input_schema = Some(surfaces::SchemaContract::Object);
                i.result_schema = Some(surfaces::SchemaContract::Any);
                i.timeout_seconds = Some(30);
                i
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
        uptrakit_plugin_infrastructure_proxmox::ProxmoxPlugin::controller_migrations,
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
        plugin_type: Set("infrastructure.proxmox".to_string()),
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
    let catalog = uptrakit_plugin_infrastructure_registry::build_catalog(
        &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
        uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(),
    )
    .expect("catalog should build");
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(catalog);

    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(proxmox_update_protection_registration(
            "plugin.infrastructure.proxmox",
            "proxmox.settings.update-hooks",
            "global-defaults",
        ))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new()
        .with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(Arc::new(db.clone()), Arc::clone(&plugin_ops))
                .with_audit_emitter(super::test_audit_emitter(db.clone())),
        ))
        .with_provider_visibility(Arc::new(AllProvidersVisible));
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
                method: Some(surfaces::InteractionHttpMethod::Put),
                tenant_id: tenant_id(),
                surface_id: "proxmox.settings.update-hooks".to_string(),
                interaction_id: "global-defaults".to_string(),
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
        serde_json::json!("infrastructure.proxmox")
    );
}

#[tokio::test]
async fn invoke_proxmox_save_item_overrides_emits_software_item_update_audit_row() {
    ensure_master_key();
    let db = setup_proxmox_db().await;
    let plugin_config_id = insert_active_proxmox_plugin_config(&db).await;
    let software_item_id = Uuid::now_v7();
    let catalog = uptrakit_plugin_infrastructure_registry::build_catalog(
        &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
        uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(),
    )
    .expect("catalog should build");
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(catalog);

    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(proxmox_update_protection_registration(
            "plugin.infrastructure.proxmox",
            "proxmox.software-item.update-hooks",
            "overrides",
        ))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new()
        .with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(Arc::new(db.clone()), Arc::clone(&plugin_ops))
                .with_audit_emitter(super::test_audit_emitter(db.clone())),
        ))
        .with_provider_visibility(Arc::new(AllProvidersVisible));
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
                method: Some(surfaces::InteractionHttpMethod::Put),
                tenant_id: tenant_id(),
                surface_id: "proxmox.software-item.update-hooks".to_string(),
                interaction_id: "overrides".to_string(),
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
        serde_json::json!("infrastructure.proxmox")
    );
}

#[tokio::test]
async fn invoke_proxmox_save_scaling_global_defaults_emits_tenant_setting_update_audit_row() {
    ensure_master_key();
    let db = setup_proxmox_db().await;
    let plugin_config_id = insert_active_proxmox_plugin_config(&db).await;
    let catalog = uptrakit_plugin_infrastructure_registry::build_catalog(
        &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
        uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(),
    )
    .expect("catalog should build");
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(catalog);

    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(proxmox_update_protection_registration(
            "plugin.infrastructure.proxmox",
            "proxmox.settings.resource-scaling",
            "global-defaults",
        ))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new()
        .with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(Arc::new(db.clone()), Arc::clone(&plugin_ops))
                .with_audit_emitter(super::test_audit_emitter(db.clone())),
        ))
        .with_provider_visibility(Arc::new(AllProvidersVisible));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = serde_json::Map::new();
    params.insert(
        "plugin_config_id".to_string(),
        serde_json::json!(plugin_config_id.to_string()),
    );
    params.insert("scaling_mode".to_string(), serde_json::json!("none"));

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                method: Some(surfaces::InteractionHttpMethod::Put),
                tenant_id: tenant_id(),
                surface_id: "proxmox.settings.resource-scaling".to_string(),
                interaction_id: "global-defaults".to_string(),
                idempotency_key: "idem-proxmox-save-scaling-global-success".to_string(),
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
        .expect("save-scaling-global-defaults must succeed");

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
    let details = row.details_json.expect("audit details");
    assert_eq!(
        details["mutation_source"],
        serde_json::json!("surface_proxy.proxmox_resource_scaling.save_scaling_global_defaults")
    );
    assert_eq!(
        details["plugin_type"],
        serde_json::json!("infrastructure.proxmox")
    );
}

#[tokio::test]
async fn invoke_proxmox_save_scaling_item_overrides_emits_software_item_update_audit_row() {
    ensure_master_key();
    let db = setup_proxmox_db().await;
    let plugin_config_id = insert_active_proxmox_plugin_config(&db).await;
    let software_item_id = Uuid::now_v7();
    let catalog = uptrakit_plugin_infrastructure_registry::build_catalog(
        &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
        uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(),
    )
    .expect("catalog should build");
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(catalog);

    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(proxmox_update_protection_registration(
            "plugin.infrastructure.proxmox",
            "proxmox.software-item.resource-scaling",
            "overrides",
        ))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new()
        .with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(Arc::new(db.clone()), Arc::clone(&plugin_ops))
                .with_audit_emitter(super::test_audit_emitter(db.clone())),
        ))
        .with_provider_visibility(Arc::new(AllProvidersVisible));
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
    params.insert("scaling_mode".to_string(), serde_json::json!("inherit"));

    // Invoke — may succeed or fail; audit must be emitted either way.
    let _ = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                method: Some(surfaces::InteractionHttpMethod::Put),
                tenant_id: tenant_id(),
                surface_id: "proxmox.software-item.resource-scaling".to_string(),
                interaction_id: "overrides".to_string(),
                idempotency_key: "idem-proxmox-save-scaling-item-audit".to_string(),
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
        serde_json::json!("surface_proxy.proxmox_resource_scaling.save_scaling_item_overrides")
    );
    assert_eq!(
        details["plugin_type"],
        serde_json::json!("infrastructure.proxmox")
    );
}
