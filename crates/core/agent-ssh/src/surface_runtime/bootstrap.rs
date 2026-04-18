use serde_json::json;
use std::path::Path;

use uptrakit_internal_wire::{
    ServiceMessage,
    surfaces::{SurfaceActionRequest, SurfaceActionResponse},
};

use crate::operations::bootstrap as bootstrap_ops;

use super::{
    SurfaceRuntimeContext, make_surface_error_response, make_surface_success_response, params,
};

/// Spawn the bootstrap-connect (plan) step as a background task.
pub(super) fn spawn_bootstrap_connect(
    request: SurfaceActionRequest,
    ctx: &SurfaceRuntimeContext<'_>,
) {
    let state_dir = ctx.state_dir.to_path_buf();
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());
    let bg_tx = ctx.bg_tx.clone();
    let service_id = ctx.service_id;
    let tenant_id = ctx.tenant_id;
    let request_id = request.request_id;
    let params = serde_json::Value::Object(request.params);
    let sensitive_params_sealed = request
        .encrypted_sensitive_params
        .map(|value| value.ciphertext_b64);

    tokio::spawn(async move {
        let response = run_bootstrap_connect(
            request_id,
            &params,
            sensitive_params_sealed.as_deref(),
            private_key_der.as_deref(),
            service_id,
            tenant_id,
            &state_dir,
        )
        .await;
        let msg = ServiceMessage::SurfaceActionResponse(response);
        if bg_tx.send(msg).await.is_err() {
            tracing::error!("failed to send bootstrap-connect result via bg_tx");
        }
    });
}

/// Spawn the bootstrap-execute step as a background task.
pub(super) fn spawn_bootstrap_execute(
    request: SurfaceActionRequest,
    ctx: &SurfaceRuntimeContext<'_>,
) {
    let state_dir = ctx.state_dir.to_path_buf();
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());
    let bg_tx = ctx.bg_tx.clone();
    let service_id = ctx.service_id;
    let tenant_id = ctx.tenant_id;
    let request_id = request.request_id;
    let params = serde_json::Value::Object(request.params);
    let sensitive_params_sealed = request
        .encrypted_sensitive_params
        .map(|value| value.ciphertext_b64);

    tokio::spawn(async move {
        let response = run_bootstrap_execute(BootstrapExecuteArgs {
            request_id,
            params: &params,
            sensitive_params_sealed: sensitive_params_sealed.as_deref(),
            private_key_der: private_key_der.as_deref(),
            service_id,
            tenant_id,
            state_dir: &state_dir,
            bg_tx: &bg_tx,
        })
        .await;
        let msg = ServiceMessage::SurfaceActionResponse(response);
        if bg_tx.send(msg).await.is_err() {
            tracing::error!("failed to send bootstrap-execute result via bg_tx");
        }
    });
}

/// The bootstrap-connect handler: probe the host and return a plan.
#[tracing::instrument(skip_all, fields(request_id = %request_id))]
async fn run_bootstrap_connect(
    request_id: uuid::Uuid,
    params: &serde_json::Value,
    sensitive_params_sealed: Option<&str>,
    private_key_der: Option<&[u8]>,
    service_id: Option<uuid::Uuid>,
    tenant_id: Option<uuid::Uuid>,
    state_dir: &Path,
) -> SurfaceActionResponse {
    let sensitive =
        match params::decrypt_sensitive_auth_params(sensitive_params_sealed, private_key_der) {
            Ok(s) => s,
            Err(msg) => return make_surface_error_response(request_id, &msg),
        };

    let bootstrap_request =
        match params::parse_bootstrap_request(params, sensitive.as_ref(), service_id, tenant_id) {
            Ok(p) => p,
            Err(msg) => return make_surface_error_response(request_id, &msg),
        };

    match bootstrap_ops::bootstrap_connect(state_dir, &bootstrap_request.bootstrap_params).await {
        Ok(plan) => match serde_json::to_value(&plan) {
            Ok(data) => make_surface_success_response(request_id, data),
            Err(e) => {
                make_surface_error_response(request_id, &format!("failed to serialize plan: {e}"))
            }
        },
        Err(e) => {
            tracing::error!(error = %e, "bootstrap-connect failed");
            make_surface_error_response(request_id, &format!("bootstrap connect failed: {e}"))
        }
    }
}

/// Arguments for the bootstrap-execute handler, bundled to stay within the 7-arg clippy limit.
struct BootstrapExecuteArgs<'a> {
    request_id: uuid::Uuid,
    params: &'a serde_json::Value,
    sensitive_params_sealed: Option<&'a str>,
    private_key_der: Option<&'a [u8]>,
    service_id: Option<uuid::Uuid>,
    tenant_id: Option<uuid::Uuid>,
    state_dir: &'a Path,
    bg_tx: &'a tokio::sync::mpsc::Sender<ServiceMessage>,
}

/// The bootstrap-execute handler: execute the bootstrap with optional skip set.
#[tracing::instrument(skip_all, fields(request_id = %args.request_id))]
async fn run_bootstrap_execute(args: BootstrapExecuteArgs<'_>) -> SurfaceActionResponse {
    let request_id = args.request_id;
    let bg_tx = args.bg_tx;
    let sensitive = match params::decrypt_sensitive_auth_params(
        args.sensitive_params_sealed,
        args.private_key_der,
    ) {
        Ok(s) => s,
        Err(msg) => return make_surface_error_response(request_id, &msg),
    };

    let bootstrap_request = match params::parse_bootstrap_request(
        args.params,
        sensitive.as_ref(),
        args.service_id,
        args.tenant_id,
    ) {
        Ok(p) => p,
        Err(msg) => return make_surface_error_response(request_id, &msg),
    };

    let host_id = bootstrap_request.bootstrap_params.host_id;

    match bootstrap_ops::bootstrap_execute(
        args.state_dir,
        bootstrap_request.bootstrap_params,
        &bootstrap_request.skip_actions,
    )
    .await
    {
        Ok(result) => {
            tracing::info!(%host_id, "bootstrap completed successfully");

            // For each infra plugin that detected infrastructure, send
            // ReportPluginConfig if new credentials were created.
            send_infra_plugin_reports(bg_tx, host_id, &result.infra_results).await;

            let any_infra = result.infra_results.iter().any(|r| r.detected);
            let mut data = json!({ "host_id": host_id.to_string() });
            if any_infra {
                data["has_infrastructure"] = json!(true);
            }
            make_surface_success_response(request_id, data)
        }
        Err(e) => {
            tracing::error!(error = %e, "bootstrap failed");
            make_surface_error_response(request_id, &format!("bootstrap failed: {e}"))
        }
    }
}

/// Send a `ReportPluginConfig` message for each infra result that produced one.
///
/// Iterates `infra_results` and, for any result that carries a
/// `report_plugin_config`, constructs the wire payload and sends it via
/// `bg_tx`.  Results that refer to an existing config are logged at `info`
/// level instead.  Send failures are logged at `error` level.
async fn send_infra_plugin_reports(
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    host_id: uuid::Uuid,
    infra_results: &[uptrakit_plugin_infrastructure_registry::agent_infra::BootstrapInfraResult],
) {
    for infra in infra_results {
        if let Some(report) = &infra.report_plugin_config {
            let payload: uptrakit_internal_wire::ReportPluginConfigPayload =
                serde_json::from_value(json!({
                    "request_id": uuid::Uuid::now_v7().to_string(),
                    "plugin_type": report.plugin_type,
                    "name": report.name,
                    "config": report.config,
                }))
                .expect("ReportPluginConfigPayload JSON is always valid");
            let msg = ServiceMessage::ReportPluginConfig(payload);
            if bg_tx.send(msg).await.is_err() {
                tracing::error!("failed to send ReportPluginConfig via bg_tx");
            }
        } else if let Some(config_id) = &infra.existing_plugin_config_id {
            tracing::info!(
                %host_id,
                %config_id,
                "reusing existing plugin config for cluster node"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::time::{Duration, timeout};

    fn bootstrap_execute_args<'a>(
        request_id: uuid::Uuid,
        params: &'a serde_json::Value,
        bg_tx: &'a tokio::sync::mpsc::Sender<ServiceMessage>,
    ) -> BootstrapExecuteArgs<'a> {
        BootstrapExecuteArgs {
            request_id,
            params,
            sensitive_params_sealed: None,
            private_key_der: None,
            service_id: None,
            tenant_id: None,
            state_dir: Path::new("/tmp"),
            bg_tx,
        }
    }

    #[tokio::test]
    async fn bootstrap_execute_maps_missing_target_to_surface_error() {
        let (bg_tx, _bg_rx) = tokio::sync::mpsc::channel(1);
        let request_id = uuid::Uuid::now_v7();
        let params = json!({});

        let response =
            run_bootstrap_execute(bootstrap_execute_args(request_id, &params, &bg_tx)).await;

        assert_eq!(response.request_id, request_id);
        assert!(!response.success);
        assert_eq!(
            response.error.as_ref().map(|error| error.message.as_str()),
            Some("missing required field 'target'")
        );
    }

    #[tokio::test]
    async fn bootstrap_connect_maps_missing_target_to_surface_error() {
        let request_id = uuid::Uuid::now_v7();
        let params = json!({});

        let response = run_bootstrap_connect(
            request_id,
            &params,
            None,
            None,
            None,
            None,
            Path::new("/tmp"),
        )
        .await;

        assert_eq!(response.request_id, request_id);
        assert!(!response.success);
        assert_eq!(
            response.error.as_ref().map(|error| error.message.as_str()),
            Some("missing required field 'target'")
        );
    }

    #[tokio::test]
    async fn send_infra_plugin_reports_emits_report_plugin_config_message() {
        let (bg_tx, mut bg_rx) = tokio::sync::mpsc::channel(1);
        let host_id = uuid::Uuid::now_v7();
        let report = uptrakit_plugin_infrastructure_registry::agent_infra::PluginConfigReport {
            plugin_type: "infrastructure_proxmox".to_string(),
            name: "pve.local".to_string(),
            config: json!({ "api_url": "https://pve.local:8006" }),
        };
        let infra_results = vec![
            uptrakit_plugin_infrastructure_registry::agent_infra::BootstrapInfraResult {
                report_plugin_config: Some(report),
                ..Default::default()
            },
        ];

        send_infra_plugin_reports(&bg_tx, host_id, &infra_results).await;

        let msg = timeout(Duration::from_secs(1), bg_rx.recv())
            .await
            .expect("report message should arrive")
            .expect("sender should remain open");
        let ServiceMessage::ReportPluginConfig(payload) = msg else {
            panic!("expected ReportPluginConfig message");
        };

        assert_eq!(payload.plugin_type, "infrastructure_proxmox");
        assert_eq!(payload.name, "pve.local");
        assert_eq!(
            payload.config,
            json!({ "api_url": "https://pve.local:8006" })
        );
        assert!(!payload.request_id.is_empty());
    }
}
