use serde_json::Value;
use uptrakit_wire::{ServiceMessage, surfaces::SurfaceActionRequest};

use crate::operations::sync as operations_sync;

use super::{
    SurfaceRuntimeContext, make_surface_error_response, make_surface_success_response, params,
};

#[derive(Clone, Copy)]
enum SyncWorkflowStep {
    Connect,
    Execute,
}

/// Spawn the sync-connect (plan) step as a background task.
pub(super) fn spawn_sync_connect(request: SurfaceActionRequest, ctx: &SurfaceRuntimeContext<'_>) {
    spawn_sync_workflow(request, ctx, SyncWorkflowStep::Connect);
}

/// Spawn the sync-execute step as a background task.
pub(super) fn spawn_sync_execute(request: SurfaceActionRequest, ctx: &SurfaceRuntimeContext<'_>) {
    spawn_sync_workflow(request, ctx, SyncWorkflowStep::Execute);
}

fn spawn_sync_workflow(
    request: SurfaceActionRequest,
    ctx: &SurfaceRuntimeContext<'_>,
    step: SyncWorkflowStep,
) {
    let db_state_dir = ctx.state_dir.to_path_buf();
    let bg_tx = ctx.bg_tx.clone();
    let tenant_id = ctx.tenant_id;
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());
    let request_id = request.request_id;
    let params = Value::Object(request.params);
    let sensitive_params_sealed = request
        .encrypted_sensitive_params
        .map(|value| value.ciphertext_b64);

    tokio::spawn(async move {
        let Some(sync_request) = resolve_sync_auth(
            &params,
            sensitive_params_sealed.as_deref(),
            request_id,
            private_key_der.as_deref(),
            &bg_tx,
        )
        .await
        else {
            return;
        };

        let db = match crate::db::init_db(&db_state_dir).await {
            Ok(db) => db,
            Err(e) => {
                let resp = make_surface_error_response(
                    request_id,
                    &format!("failed to initialize database: {e}"),
                );
                let _ = bg_tx
                    .send(ServiceMessage::SurfaceActionResponse(resp))
                    .await;
                return;
            }
        };

        let response = match step {
            SyncWorkflowStep::Connect => match operations_sync::sync_connect(
                &sync_request.host_id,
                &db,
                tenant_id,
                sync_request.auth_override.as_ref(),
                sync_request.allow_all,
            )
            .await
            {
                Ok(plan) => match serde_json::to_value(&plan) {
                    Ok(data) => make_surface_success_response(request_id, data),
                    Err(e) => make_surface_error_response(
                        request_id,
                        &format!("failed to serialize plan: {e}"),
                    ),
                },
                Err(e) => make_surface_error_response(request_id, &e),
            },
            SyncWorkflowStep::Execute => match operations_sync::sync_execute(
                &sync_request.host_id,
                &db,
                tenant_id,
                sync_request.auth_override.as_ref(),
                sync_request.allow_all,
                &sync_request.skip_actions,
            )
            .await
            {
                Ok(summary) => make_surface_success_response(
                    request_id,
                    serde_json::json!({ "summary": summary }),
                ),
                Err(e) => make_surface_error_response(request_id, &e),
            },
        };

        let msg = ServiceMessage::SurfaceActionResponse(response);
        if matches!(step, SyncWorkflowStep::Execute) {
            if bg_tx.send(msg).await.is_err() {
                tracing::error!("failed to send sync-execute result via bg_tx");
            }
        } else {
            let _ = bg_tx.send(msg).await;
        }
    });
}

/// Resolve `host_id`, decrypt sensitive params, and build the auth override.
///
/// This is the common setup for both sync workflow steps. On any failure, a
/// `SurfaceActionResponse` error is sent via `bg_tx` and `None` is returned so
/// the caller can bail early.
async fn resolve_sync_auth(
    params: &Value,
    sensitive_params_sealed: Option<&str>,
    request_id: uuid::Uuid,
    private_key_der: Option<&[u8]>,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
) -> Option<params::SyncActionRequest> {
    let host_id = match params::parse_sync_host_id(params) {
        Ok(id) => id,
        Err(msg) => {
            let resp = make_surface_error_response(request_id, &msg);
            let _ = bg_tx
                .send(ServiceMessage::SurfaceActionResponse(resp))
                .await;
            return None;
        }
    };

    let sensitive =
        match params::decrypt_sensitive_auth_params(sensitive_params_sealed, private_key_der) {
            Ok(s) => s,
            Err(msg) => {
                let resp = make_surface_error_response(request_id, &msg);
                let _ = bg_tx
                    .send(ServiceMessage::SurfaceActionResponse(resp))
                    .await;
                return None;
            }
        };

    let sync_request = match params::parse_sync_request(params, host_id, sensitive.as_ref()) {
        Ok(request) => request,
        Err(msg) => {
            let resp = make_surface_error_response(request_id, &msg);
            let _ = bg_tx
                .send(ServiceMessage::SurfaceActionResponse(resp))
                .await;
            return None;
        }
    };

    Some(sync_request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn resolve_sync_auth_maps_missing_host_id_to_surface_error() {
        let (bg_tx, mut bg_rx) = tokio::sync::mpsc::channel(1);
        let request_id = uuid::Uuid::now_v7();

        let result = resolve_sync_auth(&json!({}), None, request_id, None, &bg_tx).await;

        assert!(result.is_none());
        let Some(ServiceMessage::SurfaceActionResponse(response)) = bg_rx.recv().await else {
            panic!("expected surface action response");
        };
        assert_eq!(response.request_id, request_id);
        assert!(!response.success);
        assert_eq!(
            response.error.as_ref().map(|error| error.message.as_str()),
            Some("missing required field 'id'")
        );
    }
}
