use std::collections::{BTreeMap, BTreeSet};

use super::*;
use async_trait::async_trait;
use sea_orm::Database;
use serde_json::json;
use uptrakit_internal_wire::{
    ControllerMessage, TransportClosePolicy, TransportError,
    surfaces::{
        self, DataSourceKind, InteractionDescriptor, InteractionKind, RefreshPolicy, SurfaceNode,
        Targeting,
    },
};
use uptrakit_shared_types::Permission;

fn test_catalog() -> uptrakit_plugin_infrastructure_registry::PluginCatalog {
    let config = uptrakit_plugin_infrastructure_registry::CatalogConfig::default();
    uptrakit_plugin_infrastructure_registry::build_catalog(&config)
        .expect("plugin catalog must build for tests")
}

#[test]
fn surface_success_response_preserves_request_id_and_payload() {
    let request_id = uuid::Uuid::now_v7();
    let response = make_surface_success_response(request_id, json!({ "ok": true }));

    assert_eq!(response.request_id, request_id);
    assert!(response.success);
    assert_eq!(response.result, Some(json!({ "ok": true })));
    assert!(response.error.is_none());
}

#[test]
fn surface_error_response_preserves_request_id_and_structured_error() {
    let request_id = uuid::Uuid::now_v7();
    let response = make_surface_error_response(request_id, "boom");

    assert_eq!(response.request_id, request_id);
    assert!(!response.success);
    assert!(response.result.is_none());
    let error = response.error.expect("error payload should be present");
    assert_eq!(error.code, SurfaceActionErrorCode::InvalidRequest);
    assert_eq!(error.message, "boom");
}

#[test]
fn surface_registration_is_single_surface_and_tenant_bound() {
    let tenant_id = uuid::Uuid::now_v7();
    let registration = build_surface_registration(None, &test_catalog(), None, Some(tenant_id));

    assert_eq!(registration.surfaces.len(), 1);
    assert_eq!(
        registration.surfaces[0].descriptor.surface_id.as_str(),
        SSH_HOSTS_SURFACE_ID
    );
    assert_eq!(
        registration.effective_tenant_binding.scope,
        surfaces::Scope::Tenant
    );
    let tenant_id_str = tenant_id.to_string();
    assert_eq!(
        registration.effective_tenant_binding.tenant_id.as_deref(),
        Some(tenant_id_str.as_str())
    );
    assert!(
        registration
            .capabilities
            .0
            .contains(&surfaces::Capability::ProviderInitiatedActions),
        "provider-proxied ssh-agent surface actions must advertise provider_initiated_actions"
    );
}

#[test]
fn ssh_hosts_surface_descriptor_and_data_source_parity_is_preserved() {
    let registration = build_surface_registration(None, &test_catalog(), None, None);
    let surface = registration
        .surfaces
        .iter()
        .find(|surface| surface.descriptor.surface_id.as_str() == SSH_HOSTS_SURFACE_ID)
        .expect("ssh-agent.hosts surface is registered");

    assert_eq!(surface.descriptor.label, SSH_HOSTS_SURFACE_LABEL);
    assert_eq!(surface.descriptor.slot, surfaces::SLOT_SURFACE_PAGE);
    assert_eq!(surface.descriptor.priority, SSH_HOSTS_SURFACE_PRIORITY);
    assert_eq!(surface.descriptor.scope, surfaces::Scope::Tenant);
    assert_eq!(surface.descriptor.targeting, Targeting::Targeted);
    assert_eq!(
        surface.descriptor.required_permission.as_deref(),
        Some(Permission::UpdateHosts.as_str())
    );

    assert_eq!(surface.data_sources.len(), 1);
    let primary_data_source = &surface.data_sources[0];
    assert_eq!(primary_data_source.data_source_id.as_str(), "data.primary");
    assert_eq!(
        primary_data_source.kind,
        DataSourceKind::ProviderQuery {
            operation_id: SSH_HOSTS_DATA_ACTION_ID.to_string()
        }
    );
    assert_eq!(
        primary_data_source
            .pagination
            .as_ref()
            .map(|pagination| pagination.default_page_size),
        Some(SSH_HOSTS_DEFAULT_PER_PAGE as u16)
    );
    assert_eq!(
        primary_data_source
            .pagination
            .as_ref()
            .map(|pagination| pagination.max_page_size),
        Some(1000),
        "ssh-agent surface pagination should expose the full 1000-item limit"
    );
    assert_eq!(primary_data_source.refresh_policy, RefreshPolicy::Manual);

    let SurfaceNode::Section { children, .. } = &surface.descriptor.root_node else {
        panic!("root node should be a section");
    };
    let Some(SurfaceNode::Table {
        columns,
        row_actions,
        ..
    }) = children.first()
    else {
        panic!("first section child should be a table");
    };
    let actual_columns: Vec<(&str, &str)> = columns
        .iter()
        .map(|column| (column.key.as_str(), column.label.as_str()))
        .collect();
    assert_eq!(actual_columns, SSH_HOSTS_COLUMNS);
    let row_action_ids: Vec<&str> = row_actions
        .iter()
        .map(|action| action.interaction_id.as_str())
        .collect();
    assert_eq!(row_action_ids, SSH_HOSTS_ROW_ACTION_IDS);
}

#[test]
fn dynamic_primary_action_is_included_in_action_bar_when_available() {
    let actions = build_actions();
    assert!(
        actions
            .iter()
            .any(|action| action.action_id == "bootstrap-proxmox-guest"),
        "expected infra action bootstrap-proxmox-guest to be present in action library"
    );

    let registration = build_surface_registration(None, &test_catalog(), None, None);
    let surface = registration
        .surfaces
        .iter()
        .find(|surface| surface.descriptor.surface_id.as_str() == SSH_HOSTS_SURFACE_ID)
        .expect("ssh-agent.hosts surface is registered");

    let SurfaceNode::Section { children, .. } = &surface.descriptor.root_node else {
        panic!("root node should be a section");
    };
    let Some(SurfaceNode::ActionBar { action_ids }) = children.get(1) else {
        panic!("second section child should be an action bar");
    };
    let action_ids: BTreeSet<&str> = action_ids.iter().map(|id| id.as_str()).collect();
    assert!(action_ids.contains(SSH_HOSTS_PRIMARY_ACTION_ID));
    assert!(action_ids.contains("bootstrap-proxmox-guest"));
}

#[test]
fn workflow_interactions_are_registered_with_truthful_steps() {
    let registration = build_surface_registration(None, &test_catalog(), None, None);
    let surface = registration
        .surfaces
        .iter()
        .find(|surface| surface.descriptor.surface_id.as_str() == SSH_HOSTS_SURFACE_ID)
        .expect("ssh-agent.hosts surface is registered");

    let interactions: BTreeMap<&str, &InteractionDescriptor> = surface
        .interactions
        .iter()
        .map(|interaction| (interaction.interaction_id.as_str(), interaction))
        .collect();

    assert!(interactions.contains_key("list-hosts"));
    assert!(interactions.contains_key("remove-host"));

    let bootstrap = interactions
        .get("bootstrap")
        .copied()
        .expect("bootstrap workflow interaction is present");
    assert_eq!(bootstrap.kind, InteractionKind::Workflow);
    assert_eq!(bootstrap.workflow_steps.len(), 3);
    assert_eq!(bootstrap.workflow_steps[0].step_id, "connect");
    assert_eq!(
        bootstrap.workflow_steps[0]
            .submit_interaction_id
            .as_ref()
            .map(|id| id.as_str()),
        Some("bootstrap-connect")
    );
    assert!(bootstrap.workflow_steps[1].render_previous_response);
    assert_eq!(
        bootstrap.workflow_steps[2]
            .submit_interaction_id
            .as_ref()
            .map(|id| id.as_str()),
        Some("bootstrap-execute")
    );

    let sync_host = interactions
        .get("sync-host")
        .copied()
        .expect("sync-host workflow interaction is present");
    assert_eq!(sync_host.kind, InteractionKind::Workflow);
    assert_eq!(sync_host.workflow_steps.len(), 3);
    assert_eq!(sync_host.workflow_steps[0].step_id, "connect");
    assert_eq!(
        sync_host.workflow_steps[0]
            .submit_interaction_id
            .as_ref()
            .map(|id| id.as_str()),
        Some("sync-connect")
    );
    assert!(sync_host.workflow_steps[1].render_previous_response);
    assert_eq!(
        sync_host.workflow_steps[2]
            .submit_interaction_id
            .as_ref()
            .map(|id| id.as_str()),
        Some("sync-execute")
    );
}

#[test]
fn workflow_step_submit_interactions_are_registered_for_dispatch() {
    let registration = build_surface_registration(None, &test_catalog(), None, None);
    let surface = registration
        .surfaces
        .iter()
        .find(|registered| registered.descriptor.surface_id.as_str() == SSH_HOSTS_SURFACE_ID)
        .expect("ssh-agent.hosts surface is registered");
    let interaction_ids: BTreeSet<&str> = surface
        .interactions
        .iter()
        .map(|interaction| interaction.interaction_id.as_str())
        .collect();

    assert!(interaction_ids.contains("bootstrap-connect"));
    assert!(interaction_ids.contains("bootstrap-execute"));
    assert!(interaction_ids.contains("sync-connect"));
    assert!(interaction_ids.contains("sync-execute"));
}

#[derive(Default)]
struct RecordingTransport {
    sent: Vec<ServiceMessage>,
}

#[async_trait]
impl ServiceTransport for RecordingTransport {
    async fn transport_send(&mut self, msg: ServiceMessage) -> Result<(), TransportError> {
        self.sent.push(msg);
        Ok(())
    }

    async fn transport_send_best_effort(&mut self, msg: ServiceMessage) {
        self.sent.push(msg);
    }

    async fn transport_send_auto_paginate(
        &mut self,
        msg: ServiceMessage,
    ) -> Result<(), TransportError> {
        self.sent.push(msg);
        Ok(())
    }

    async fn transport_recv(&mut self) -> Option<ControllerMessage> {
        None
    }

    fn close_policy(&self) -> TransportClosePolicy {
        TransportClosePolicy::Reconnect { reason: None }
    }
}

#[tokio::test]
async fn unregistered_interaction_is_rejected_before_infra_fallback() {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("in-memory db");
    let state_dir = tempfile::tempdir().expect("tempdir");
    let (bg_tx, _bg_rx) = tokio::sync::mpsc::channel(4);
    let surface_proxy = Arc::new(uptrakit_service_sdk::ServiceSurfaceProxy::new());
    let infra_bundles = Arc::new(Vec::new());
    let ctx = SurfaceRuntimeContext {
        db: &db,
        state_dir: state_dir.path(),
        private_key_der: None,
        service_id: None,
        tenant_id: None,
        bg_tx: &bg_tx,
        surface_proxy: &surface_proxy,
        infra_bundles,
    };
    let request = SurfaceActionRequest {
        request_id: uuid::Uuid::now_v7(),
        tenant_id: uuid::Uuid::now_v7().to_string(),
        surface_id: surfaces::SurfaceId::new(SSH_HOSTS_SURFACE_ID.to_string())
            .expect("surface id should be valid"),
        interaction_id: surfaces::InteractionId::new("non-registered-action".to_string())
            .expect("interaction id should be valid"),
        idempotency_key: uuid::Uuid::now_v7().to_string(),
        target_provider_id: None,
        caller_origin: surfaces::CallerOrigin::BuiltInSystem {
            principal: "test".to_string(),
        },
        params: serde_json::Map::new(),
        encrypted_sensitive_params: None,
    };
    let mut conn = RecordingTransport::default();

    handle_surface_action_request(request, &ctx, &mut conn).await;

    assert_eq!(conn.sent.len(), 1);
    let ServiceMessage::SurfaceActionResponse(response) = &conn.sent[0] else {
        panic!("expected surface action response");
    };
    assert!(!response.success);
    assert_eq!(
        response.error.as_ref().map(|error| error.message.as_str()),
        Some("unknown action")
    );
}

#[tokio::test]
async fn bootstrap_execute_dispatches_async_response_via_bg_tx() {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("in-memory db");
    let state_dir = tempfile::tempdir().expect("tempdir");
    let (bg_tx, mut bg_rx) = tokio::sync::mpsc::channel(4);
    let surface_proxy = Arc::new(uptrakit_service_sdk::ServiceSurfaceProxy::new());
    let infra_bundles = Arc::new(Vec::new());
    let ctx = SurfaceRuntimeContext {
        db: &db,
        state_dir: state_dir.path(),
        private_key_der: None,
        service_id: None,
        tenant_id: None,
        bg_tx: &bg_tx,
        surface_proxy: &surface_proxy,
        infra_bundles,
    };
    let request = SurfaceActionRequest {
        request_id: uuid::Uuid::now_v7(),
        tenant_id: uuid::Uuid::now_v7().to_string(),
        surface_id: surfaces::SurfaceId::new(SSH_HOSTS_SURFACE_ID.to_string())
            .expect("surface id should be valid"),
        interaction_id: surfaces::InteractionId::new("bootstrap-execute".to_string())
            .expect("interaction id should be valid"),
        idempotency_key: uuid::Uuid::now_v7().to_string(),
        target_provider_id: None,
        caller_origin: surfaces::CallerOrigin::BuiltInSystem {
            principal: "test".to_string(),
        },
        params: serde_json::Map::new(),
        encrypted_sensitive_params: None,
    };
    let mut conn = RecordingTransport::default();

    handle_surface_action_request(request, &ctx, &mut conn).await;

    assert!(
        conn.sent.is_empty(),
        "bootstrap-execute should respond via bg_tx"
    );

    let msg = tokio::time::timeout(std::time::Duration::from_secs(1), bg_rx.recv())
        .await
        .expect("bootstrap response should arrive")
        .expect("sender should remain open");
    let ServiceMessage::SurfaceActionResponse(response) = msg else {
        panic!("expected surface action response");
    };
    assert!(!response.success);
    assert_eq!(
        response.error.as_ref().map(|error| error.message.as_str()),
        Some("missing required field 'target'")
    );
}
