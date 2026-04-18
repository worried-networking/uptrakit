use std::sync::Arc;

use uptrakit_internal_wire::{ServiceMessage, surfaces::SurfaceActionRequest};
use uptrakit_plugin_infrastructure_registry::agent_infra::InfraPluginContext;

use crate::operations::bootstrap_proxmox::AgentGuestBootstrapExecutor;

use super::super::proxy::InfraActionInvokerImpl;
use super::super::{SurfaceRuntimeContext, make_surface_error_response};

/// Spawn an infrastructure plugin action as a background task.
///
/// Iterates all registered infra plugins; the first one to return `Some`
/// wins. If no plugin handles the action, an error response is sent.
pub(super) fn spawn_infra_plugin_action(
    request: SurfaceActionRequest,
    ctx: &SurfaceRuntimeContext<'_>,
) {
    let state_dir = ctx.state_dir.to_path_buf();
    let bg_tx = ctx.bg_tx.clone();
    let proxy = Arc::clone(ctx.surface_proxy);
    let infra_bundles = Arc::clone(&ctx.infra_bundles);
    let service_id = ctx.service_id;
    let tenant_id = ctx.tenant_id;
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());

    tokio::spawn(async move {
        let db = match crate::db::init_db(&state_dir).await {
            Ok(db) => db,
            Err(e) => {
                let resp = make_surface_error_response(
                    request.request_id,
                    &format!("failed to initialize database: {e}"),
                );
                let _ = bg_tx
                    .send(ServiceMessage::SurfaceActionResponse(resp))
                    .await;
                return;
            }
        };

        let tenant_id_str = tenant_id.map(|t| t.to_string());
        let action_invoker = InfraActionInvokerImpl::new(&proxy, &bg_tx, tenant_id);
        let guest_bootstrap = AgentGuestBootstrapExecutor {
            state_dir: state_dir.clone(),
            service_id,
        };
        let plugin_ctx = InfraPluginContext {
            db: &db,
            tenant_id: tenant_id_str.as_deref(),
            service_id,
            state_dir: &state_dir,
            private_key_der: private_key_der.as_deref(),
            action_invoker: &action_invoker,
            guest_bootstrap: &guest_bootstrap,
        };

        let mut response = None;
        for bundle in infra_bundles.iter() {
            if let Some(guest_exec) = bundle.guest_exec.as_ref()
                && let Some(resp) = guest_exec
                    .handle_service_extension_action(&plugin_ctx, &request)
                    .await
            {
                response = Some(resp);
                break;
            }
        }

        let resp = response.unwrap_or_else(|| {
            tracing::warn!(
                action_id = %request.interaction_id,
                surface_id = %request.surface_id,
                "no infrastructure plugin handled this action"
            );
            make_surface_error_response(request.request_id, "unknown action")
        });

        if bg_tx
            .send(ServiceMessage::SurfaceActionResponse(resp))
            .await
            .is_err()
        {
            tracing::error!("failed to send infra plugin action result via bg_tx");
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::spawn_infra_plugin_action;
    use crate::surface_runtime::SurfaceRuntimeContext;
    use uptrakit_internal_wire::{
        ServiceMessage,
        surfaces::{self, SurfaceActionRequest},
    };

    #[tokio::test]
    async fn unhandled_infra_action_sends_unknown_action_error_via_bg_tx() {
        let db = sea_orm::Database::connect("sqlite::memory:")
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
            surface_id: surfaces::SurfaceId::new("ssh-agent.hosts".to_string())
                .expect("surface id should be valid"),
            interaction_id: surfaces::InteractionId::new("infra-test-action".to_string())
                .expect("interaction id should be valid"),
            idempotency_key: uuid::Uuid::now_v7().to_string(),
            target_provider_id: None,
            caller_origin: surfaces::CallerOrigin::BuiltInSystem {
                principal: "test".to_string(),
            },
            params: serde_json::Map::new(),
            encrypted_sensitive_params: None,
        };

        spawn_infra_plugin_action(request, &ctx);

        let msg = tokio::time::timeout(std::time::Duration::from_secs(1), bg_rx.recv())
            .await
            .expect("infra action response should arrive")
            .expect("sender should remain open");
        let ServiceMessage::SurfaceActionResponse(response) = msg else {
            panic!("expected surface action response");
        };
        assert!(!response.success);
        assert_eq!(
            response.error.as_ref().map(|error| error.message.as_str()),
            Some("unknown action")
        );
    }
}
