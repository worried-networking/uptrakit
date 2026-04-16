//! Surface action handlers for the Proxmox agent infrastructure plugin.
//!
//! Handles: `list-discovered-guests`, `bootstrap-proxmox-guest`.

use serde_json::json;
use uptrakit_plugin_infrastructure_core::agent_infra::{GuestBootstrapParams, InfraPluginContext};
use uptrakit_plugin_infrastructure_core::surfaces::{
    SurfaceActionError, SurfaceActionErrorCode, SurfaceActionRequest, SurfaceActionResponse,
};

use super::db_ops;

/// Dispatch a surface action to the appropriate handler.
///
/// Returns `Some(response)` if this plugin handles the action, `None` otherwise.
pub async fn handle_surface_action(
    ctx: &InfraPluginContext<'_>,
    request: &SurfaceActionRequest,
) -> Option<SurfaceActionResponse> {
    match request.interaction_id.as_str() {
        "list-discovered-guests" => {
            Some(handle_list_discovered_guests(request.request_id, ctx).await)
        }
        "bootstrap-proxmox-guest" => {
            Some(handle_bootstrap_proxmox_guest(request.request_id, &request.params, ctx).await)
        }
        _ => None,
    }
}

// ── list-discovered-guests ───────────────────────────────────────────────────

/// List discovered Proxmox guests via the controller's Proxmox plugin.
async fn handle_list_discovered_guests(
    request_id: uuid::Uuid,
    ctx: &InfraPluginContext<'_>,
) -> SurfaceActionResponse {
    let response = ctx
        .action_invoker
        .invoke(
            "proxmox.hosts",
            "list-all-unmatched",
            json!({ "per_page": 1000 }),
        )
        .await;

    match response {
        Ok(proxy_resp) if proxy_resp.success => {
            let options = surface_action_result_or_null(&proxy_resp)["items"].clone();
            make_success_response(request_id, json!({ "options": options }))
        }
        Ok(proxy_resp) => {
            tracing::debug!(
                error = ?proxy_resp.error,
                "list-all-unmatched returned error, returning empty options"
            );
            make_success_response(request_id, json!({ "options": [] }))
        }
        Err(e) => {
            tracing::debug!(
                error = %e,
                "list-all-unmatched proxy call failed, returning empty options"
            );
            make_success_response(request_id, json!({ "options": [] }))
        }
    }
}

// ── bootstrap-proxmox-guest ──────────────────────────────────────────────────

/// Bootstrap one or more discovered Proxmox guests with bounded concurrency.
async fn handle_bootstrap_proxmox_guest(
    request_id: uuid::Uuid,
    params: &serde_json::Map<String, serde_json::Value>,
    ctx: &InfraPluginContext<'_>,
) -> SurfaceActionResponse {
    const MAX_CONCURRENT_BOOTSTRAPS: usize = 4;

    // 1. Parse selected guest IDs from the request params.
    let discovered_guests = parse_discovered_guests(params);
    if discovered_guests.is_empty() {
        return make_error_response(request_id, "no guests selected");
    }

    // 2. Fetch all unmatched guests (one proxy call).
    //    Pass per_page=1000 to avoid pagination truncation (same as list-discovered-guests).
    let guests_data = match ctx
        .action_invoker
        .invoke(
            "proxmox.hosts",
            "list-all-unmatched",
            json!({ "per_page": 1000 }),
        )
        .await
    {
        Ok(resp) if resp.success => surface_action_result_or_null(&resp),
        Ok(resp) => {
            let err = surface_action_error_message(resp)
                .unwrap_or_else(|| "Proxmox plugin returned error".to_string());
            return make_error_response(request_id, &err);
        }
        Err(e) => {
            return make_error_response(
                request_id,
                &format!("failed to query Proxmox plugin (is it installed?): {e}"),
            );
        }
    };

    let options = guests_data
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // 3. Load PVE hosts and build (node, config_id) -> host_id map.
    let pve_host_map: std::collections::HashMap<(String, String), String> =
        match db_ops::find_pve_hosts(ctx.db).await {
            Ok(hosts) => hosts
                .into_iter()
                .filter_map(|h| {
                    let node = h.pve_node_name?;
                    let config = h.pve_plugin_config_id?;
                    Some(((node, config), h.host_id))
                })
                .collect(),
            Err(e) => {
                return make_error_response(request_id, &format!("failed to load PVE hosts: {e}"));
            }
        };

    // 4. Resolve each guest into bootstrap params, collecting immediate errors.
    let target_username = params
        .get("target_username")
        .and_then(|v| v.as_str())
        .unwrap_or("uptrakit")
        .to_string();
    let allow_all = param_bool(params, "allow_all");
    let remove_stale_keys = param_bool(params, "remove_stale_keys");

    let mut immediate_errors: Vec<serde_json::Value> = Vec::new();
    let mut tasks: Vec<(String, String, GuestBootstrapParams)> = Vec::new();

    for guest_id in &discovered_guests {
        match validate_and_resolve_guest(
            guest_id,
            &options,
            &pve_host_map,
            &target_username,
            allow_all,
            remove_stale_keys,
            ctx.service_id,
        ) {
            Ok(resolved) => tasks.push(resolved),
            Err(error_json) => immediate_errors.push(error_json),
        }
    }

    // 5. Run bootstraps with bounded concurrency via futures stream.
    //    Using `buffer_unordered` avoids the 'static lifetime requirement
    //    that `JoinSet::spawn` would impose on the `ctx` reference.
    use futures_util::StreamExt;

    let mut results = immediate_errors;

    let bootstrap_futures: Vec<_> = tasks
        .into_iter()
        .map(|(mapping_id, name, params)| {
            let host_id = params.host_id;

            async move {
                match ctx.guest_bootstrap.bootstrap_guest(params).await {
                    Ok(result) => {
                        tracing::info!(
                            %host_id,
                            hostname = %result.hostname,
                            mapping_id = %mapping_id,
                            "discovered guest bootstrap completed"
                        );

                        if let Err(e) =
                            db_ops::insert_pending_match(ctx.db, &host_id.to_string(), &mapping_id)
                                .await
                        {
                            tracing::warn!(
                                %host_id,
                                %mapping_id,
                                error = %e,
                                "failed to persist pending Proxmox match; \
                                 match must be done manually"
                            );
                        } else {
                            tracing::info!(
                                %host_id,
                                %mapping_id,
                                "pending Proxmox match saved; \
                                 will be applied after ReportHosts"
                            );
                        }

                        json!({
                            "mapping_id": mapping_id,
                            "name": name,
                            "host_id": host_id.to_string(),
                            "hostname": result.hostname,
                            "status": "ok",
                        })
                    }
                    Err(e) => {
                        tracing::error!(
                            mapping_id = %mapping_id,
                            error = %e,
                            "discovered guest bootstrap failed"
                        );
                        json!({
                            "mapping_id": mapping_id,
                            "name": name,
                            "status": "error",
                            "error": format!("bootstrap failed: {e}"),
                        })
                    }
                }
            }
        })
        .collect();

    let stream =
        futures_util::stream::iter(bootstrap_futures).buffer_unordered(MAX_CONCURRENT_BOOTSTRAPS);
    let task_results: Vec<serde_json::Value> = stream.collect().await;
    results.extend(task_results);

    let succeeded: usize = results.iter().filter(|r| r["status"] == "ok").count();
    let failed = results.len() - succeeded;

    if succeeded == 0 {
        let errors: Vec<&str> = results.iter().filter_map(|r| r["error"].as_str()).collect();
        return make_error_response(
            request_id,
            &format!(
                "all {} guest(s) failed to bootstrap: {}",
                failed,
                errors.join("; ")
            ),
        );
    }

    make_success_response(
        request_id,
        json!({
            "results": results,
            "succeeded": succeeded,
            "failed": failed,
        }),
    )
}

// ── Guest parsing and validation ─────────────────────────────────────────────

/// Extract the list of guest IDs from the `discovered_guests` request parameter.
///
/// Accepts either a JSON array of strings or a single string (which is first
/// tried as a JSON-encoded array, then treated as a single ID).
fn parse_discovered_guests(params: &serde_json::Map<String, serde_json::Value>) -> Vec<String> {
    match params.get("discovered_guests") {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(serde_json::Value::String(s)) => serde_json::from_str::<Vec<String>>(s)
            .unwrap_or_else(|_| {
                if s.is_empty() {
                    vec![]
                } else {
                    vec![s.clone()]
                }
            }),
        _ => vec![],
    }
}

/// Validate a single guest entry and resolve it into bootstrap parameters.
///
/// On success returns `(mapping_id, display_name, GuestBootstrapParams)`.
/// On failure returns a JSON error object suitable for inclusion in the
/// results array.
#[allow(clippy::too_many_arguments)] // mirrors the many fields needed for bootstrap
fn validate_and_resolve_guest(
    guest_id: &str,
    options: &[serde_json::Value],
    pve_host_map: &std::collections::HashMap<(String, String), String>,
    target_username: &str,
    allow_all: bool,
    remove_stale_keys: bool,
    service_id: Option<uuid::Uuid>,
) -> Result<(String, String, GuestBootstrapParams), serde_json::Value> {
    let guest = options
        .iter()
        .find(|o| {
            o.get("value")
                .and_then(|v| v.as_str())
                .is_some_and(|v| v == guest_id)
        })
        .ok_or_else(|| guest_error(guest_id, "guest not found in discovered guests list"))?;

    let vmid: u32 = match guest.get("proxmox_vmid").and_then(|v| v.as_i64()) {
        Some(v) if v > 0 => v as u32,
        _ => return Err(guest_error(guest_id, "invalid VMID in discovered guest")),
    };

    let guest_type_str = guest
        .get("proxmox_type")
        .and_then(|v| v.as_str())
        .unwrap_or("lxc");
    if !matches!(guest_type_str, "lxc" | "qemu") {
        return Err(guest_error(
            guest_id,
            "unknown guest type in discovered guest",
        ));
    }

    let proxmox_node = guest
        .get("proxmox_node")
        .and_then(|v| v.as_str())
        .ok_or_else(|| guest_error(guest_id, "missing proxmox_node in discovered guest"))?
        .to_string();

    let plugin_config_id = guest
        .get("plugin_config_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| guest_error(guest_id, "missing plugin_config_id in discovered guest"))?
        .to_string();

    let pve_host_id = pve_host_map
        .get(&(proxmox_node.clone(), plugin_config_id.clone()))
        .ok_or_else(|| {
            guest_error(
                guest_id,
                &format!(
                    "no PVE host found for node '{proxmox_node}' with plugin \
                     config '{plugin_config_id}'; run 'host sync' on PVE hosts first"
                ),
            )
        })?
        .clone();

    let name = guest
        .get("hostname")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            guest
                .get("proxmox_name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        })
        .map(str::to_string)
        .unwrap_or_else(|| match guest_type_str {
            "qemu" => format!("vm-{vmid}"),
            _ => format!("ct-{vmid}"),
        });

    let host_id = uuid::Uuid::now_v7();
    Ok((
        guest_id.to_string(),
        name.clone(),
        GuestBootstrapParams::new(
            pve_host_id,
            vmid,
            guest_type_str,
            name,
            target_username.to_string(),
            allow_all,
            remove_stale_keys,
            host_id,
            service_id,
        ),
    ))
}

/// Build a standardized guest-level error JSON object.
fn guest_error(guest_id: &str, error: &str) -> serde_json::Value {
    json!({
        "mapping_id": guest_id,
        "name": guest_id,
        "status": "error",
        "error": error,
    })
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn param_bool(params: &serde_json::Map<String, serde_json::Value>, key: &str) -> bool {
    match params.get(key) {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s == "true",
        _ => false,
    }
}

fn surface_action_result_or_null(response: &SurfaceActionResponse) -> serde_json::Value {
    response.result.clone().unwrap_or(serde_json::Value::Null)
}

fn surface_action_error_message(response: SurfaceActionResponse) -> Option<String> {
    response.error.map(|error| error.message)
}

fn make_success_response(request_id: uuid::Uuid, data: serde_json::Value) -> SurfaceActionResponse {
    SurfaceActionResponse {
        request_id,
        success: true,
        result: Some(data),
        error: None,
    }
}

fn make_error_response(request_id: uuid::Uuid, message: &str) -> SurfaceActionResponse {
    SurfaceActionResponse {
        request_id,
        success: false,
        result: None,
        error: Some(SurfaceActionError {
            code: SurfaceActionErrorCode::InvalidRequest,
            message: message.to_string(),
            details: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::surfaces::SurfaceActionErrorCode;

    #[test]
    fn error_response_preserves_request_id_and_structured_error() {
        let request_id = uuid::Uuid::now_v7();
        let response = make_error_response(request_id, "boom");

        assert_eq!(response.request_id, request_id);
        assert!(!response.success);
        assert!(response.result.is_none());
        let error = response.error.expect("error payload should be present");
        assert_eq!(error.code, SurfaceActionErrorCode::InvalidRequest);
        assert_eq!(error.message, "boom");
    }

    #[test]
    fn success_response_preserves_request_id_and_payload() {
        let request_id = uuid::Uuid::now_v7();
        let data = json!({ "ok": true });
        let response = make_success_response(request_id, data.clone());

        assert_eq!(response.request_id, request_id);
        assert!(response.success);
        assert_eq!(response.result, Some(data));
        assert!(response.error.is_none());
    }

    #[test]
    fn surface_action_result_or_null_preserves_payload() {
        let response = SurfaceActionResponse {
            request_id: uuid::Uuid::now_v7(),
            success: true,
            result: Some(json!({ "items": [1, 2, 3] })),
            error: None,
        };

        assert_eq!(
            surface_action_result_or_null(&response),
            json!({ "items": [1, 2, 3] })
        );
    }

    #[test]
    fn surface_action_result_or_null_defaults_to_null() {
        let response = SurfaceActionResponse {
            request_id: uuid::Uuid::now_v7(),
            success: false,
            result: None,
            error: Some(SurfaceActionError {
                code: SurfaceActionErrorCode::InvalidRequest,
                message: "boom".to_string(),
                details: None,
            }),
        };

        assert_eq!(
            surface_action_result_or_null(&response),
            serde_json::Value::Null
        );
    }

    #[test]
    fn surface_action_error_message_extracts_structured_error_message() {
        let response = SurfaceActionResponse {
            request_id: uuid::Uuid::now_v7(),
            success: false,
            result: None,
            error: Some(SurfaceActionError {
                code: SurfaceActionErrorCode::InvalidRequest,
                message: "boom".to_string(),
                details: None,
            }),
        };

        assert_eq!(
            surface_action_error_message(response).as_deref(),
            Some("boom")
        );
    }
}
