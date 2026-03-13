//! Extension action handlers for the Proxmox agent infrastructure plugin.
//!
//! Handles: `list-pve-hosts`, `list-discovered-guests`, `bootstrap-proxmox`,
//! `bootstrap-proxmox-guest`.

use serde_json::json;
use uptrakit_extension_framework::{ExtensionRequestPayload, ExtensionResponsePayload};
use uptrakit_plugin_infrastructure_core::agent_infra::{GuestBootstrapParams, InfraPluginContext};

use super::db_ops;

/// Dispatch an extension action to the appropriate handler.
///
/// Returns `Some(response)` if this plugin handles the action, `None` otherwise.
pub async fn handle_action(
    ctx: &InfraPluginContext<'_>,
    request: &ExtensionRequestPayload,
) -> Option<ExtensionResponsePayload> {
    match request.action_id.as_str() {
        "list-pve-hosts" => Some(handle_list_pve_hosts(&request.request_id, ctx).await),
        "list-discovered-guests" => {
            Some(handle_list_discovered_guests(&request.request_id, ctx).await)
        }
        "bootstrap-proxmox" => {
            Some(handle_bootstrap_proxmox(&request.request_id, &request.params, ctx).await)
        }
        "bootstrap-proxmox-guest" => {
            Some(handle_bootstrap_proxmox_guest(&request.request_id, &request.params, ctx).await)
        }
        _ => None,
    }
}

// ── list-pve-hosts ───────────────────────────────────────────────────────────

/// List PVE hosts for dynamic select options.
async fn handle_list_pve_hosts(
    request_id: &str,
    ctx: &InfraPluginContext<'_>,
) -> ExtensionResponsePayload {
    // We need host names/hostnames from the ssh_hosts table. Since we don't
    // have direct access to ssh_hosts from the plugin, we query our own
    // proxmox_host_state table for PVE host_ids and return them.
    // The agent-ssh wraps this to include host details from its own table.
    match db_ops::find_pve_hosts(ctx.db).await {
        Ok(hosts) => {
            let options: Vec<serde_json::Value> = hosts
                .into_iter()
                .map(|h| {
                    json!({
                        "value": h.host_id,
                        "label": h.pve_node_name.unwrap_or_else(|| h.host_id.clone()),
                    })
                })
                .collect();
            make_success_response(request_id, json!({ "options": options }))
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to list PVE hosts");
            make_error_response(request_id, "failed to list PVE hosts")
        }
    }
}

// ── list-discovered-guests ───────────────────────────────────────────────────

/// List discovered Proxmox guests via the controller's Proxmox plugin.
async fn handle_list_discovered_guests(
    request_id: &str,
    ctx: &InfraPluginContext<'_>,
) -> ExtensionResponsePayload {
    let response = ctx
        .action_invoker
        .invoke(
            "proxmox.hosts",
            "list-all-unmatched",
            serde_json::Value::Object(serde_json::Map::new()),
        )
        .await;

    match response {
        Ok(proxy_resp) if proxy_resp.success => make_success_response(request_id, proxy_resp.data),
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

// ── bootstrap-proxmox ────────────────────────────────────────────────────────

/// Bootstrap a guest via a known PVE host (manual VMID entry).
async fn handle_bootstrap_proxmox(
    request_id: &str,
    params: &serde_json::Value,
    ctx: &InfraPluginContext<'_>,
) -> ExtensionResponsePayload {
    let pve_host_id = match params.get("pve_host_id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => return make_error_response(request_id, "missing required field 'pve_host_id'"),
    };

    let vmid_str = match params.get("vmid").and_then(|v| v.as_str()) {
        Some(v) => v,
        None => return make_error_response(request_id, "missing required field 'vmid'"),
    };
    let vmid: u32 = match vmid_str.parse() {
        Ok(v) => v,
        Err(_) => return make_error_response(request_id, "vmid must be a number"),
    };

    let guest_type_str = params
        .get("guest_type")
        .and_then(|v| v.as_str())
        .unwrap_or("lxc");
    match guest_type_str {
        "lxc" | "qemu" => {}
        _ => return make_error_response(request_id, "guest_type must be 'lxc' or 'qemu'"),
    }

    let name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => return make_error_response(request_id, "missing required field 'name'"),
    };

    let target_username = params
        .get("target_username")
        .and_then(|v| v.as_str())
        .unwrap_or("uptrakit")
        .to_string();

    let allow_all = param_bool(params, "allow_all");
    let remove_stale_keys = param_bool(params, "remove_stale_keys");
    let host_id = uuid::Uuid::now_v7();

    let bootstrap_params = GuestBootstrapParams::new(
        pve_host_id,
        vmid,
        guest_type_str,
        name,
        target_username,
        allow_all,
        remove_stale_keys,
        host_id,
        ctx.service_id,
    );

    match ctx.guest_bootstrap.bootstrap_guest(bootstrap_params).await {
        Ok(result) => {
            tracing::info!(
                %host_id,
                hostname = %result.hostname,
                "Proxmox guest bootstrap completed successfully"
            );
            make_success_response(
                request_id,
                json!({
                    "host_id": host_id.to_string(),
                    "hostname": result.hostname,
                }),
            )
        }
        Err(e) => {
            tracing::error!(error = %e, "Proxmox guest bootstrap failed");
            make_error_response(request_id, &format!("bootstrap failed: {e}"))
        }
    }
}

// ── bootstrap-proxmox-guest ──────────────────────────────────────────────────

/// Bootstrap one or more discovered Proxmox guests (multi-guest, parallel).
async fn handle_bootstrap_proxmox_guest(
    request_id: &str,
    params: &serde_json::Value,
    ctx: &InfraPluginContext<'_>,
) -> ExtensionResponsePayload {
    const MAX_CONCURRENT_BOOTSTRAPS: usize = 4;

    // 1. Parse selected guest IDs.
    let discovered_guests: Vec<String> = match params.get("discovered_guests") {
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
    };

    if discovered_guests.is_empty() {
        return make_error_response(request_id, "no guests selected");
    }

    // 2. Fetch all unmatched guests (one proxy call).
    let guests_data = match ctx
        .action_invoker
        .invoke(
            "proxmox.hosts",
            "list-all-unmatched",
            serde_json::Value::Object(serde_json::Map::new()),
        )
        .await
    {
        Ok(resp) if resp.success => resp.data,
        Ok(resp) => {
            let err = resp
                .error
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
        .get("options")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // 3. Load PVE hosts and build (node, config_id) → host_id map.
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

    // 4. Resolve each guest.
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
        let guest = options.iter().find(|o| {
            o.get("value")
                .and_then(|v| v.as_str())
                .is_some_and(|v| v == guest_id)
        });

        let guest = match guest {
            Some(g) => g,
            None => {
                immediate_errors.push(json!({
                    "mapping_id": guest_id,
                    "name": guest_id,
                    "status": "error",
                    "error": "guest not found in discovered guests list",
                }));
                continue;
            }
        };

        let vmid: u32 = match guest.get("proxmox_vmid").and_then(|v| v.as_i64()) {
            Some(v) if v > 0 => v as u32,
            _ => {
                immediate_errors.push(json!({
                    "mapping_id": guest_id,
                    "name": guest_id,
                    "status": "error",
                    "error": "invalid VMID in discovered guest",
                }));
                continue;
            }
        };

        let guest_type_str = guest
            .get("proxmox_type")
            .and_then(|v| v.as_str())
            .unwrap_or("lxc");
        match guest_type_str {
            "lxc" | "qemu" => {}
            _ => {
                immediate_errors.push(json!({
                    "mapping_id": guest_id,
                    "name": guest_id,
                    "status": "error",
                    "error": "unknown guest type in discovered guest",
                }));
                continue;
            }
        }

        let proxmox_node = match guest.get("proxmox_node").and_then(|v| v.as_str()) {
            Some(n) => n.to_string(),
            None => {
                immediate_errors.push(json!({
                    "mapping_id": guest_id,
                    "name": guest_id,
                    "status": "error",
                    "error": "missing proxmox_node in discovered guest",
                }));
                continue;
            }
        };

        let plugin_config_id = match guest.get("plugin_config_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => {
                immediate_errors.push(json!({
                    "mapping_id": guest_id,
                    "name": guest_id,
                    "status": "error",
                    "error": "missing plugin_config_id in discovered guest",
                }));
                continue;
            }
        };

        let pve_host_id = match pve_host_map.get(&(proxmox_node.clone(), plugin_config_id.clone()))
        {
            Some(id) => id.clone(),
            None => {
                immediate_errors.push(json!({
                    "mapping_id": guest_id,
                    "name": guest_id,
                    "status": "error",
                    "error": format!(
                        "no PVE host found for node '{proxmox_node}' with plugin \
                         config '{plugin_config_id}'; run 'host sync' on PVE hosts first"
                    ),
                }));
                continue;
            }
        };

        // Auto-derive name.
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
        tasks.push((
            guest_id.clone(),
            name.clone(),
            GuestBootstrapParams::new(
                pve_host_id,
                vmid,
                guest_type_str,
                name,
                target_username.clone(),
                allow_all,
                remove_stale_keys,
                host_id,
                ctx.service_id,
            ),
        ));
    }

    // 5. Run bootstraps in parallel with bounded concurrency.
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_BOOTSTRAPS));
    let mut join_set: tokio::task::JoinSet<serde_json::Value> = tokio::task::JoinSet::new();

    // We need to clone the state_dir for spawned tasks to init their own DB.
    let state_dir = ctx.state_dir.to_path_buf();

    for (mapping_id, name, bootstrap_params) in tasks {
        let sem = std::sync::Arc::clone(&semaphore);
        let host_id = bootstrap_params.host_id;
        let state_dir = state_dir.clone();

        // We can't pass ctx.guest_bootstrap into spawn directly because it's
        // not 'static. Instead, we run bootstraps sequentially within the
        // semaphore-bounded tasks but on the current task.
        // Actually, GuestBootstrapExecutor is Send+Sync, but the reference
        // isn't 'static. We need a different approach.
        // Let's collect params and run them after.
        join_set.spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore is never closed");
            // We'll fill in the result below.
            json!({
                "mapping_id": mapping_id,
                "name": name,
                "host_id": host_id.to_string(),
                "state_dir": state_dir.to_string_lossy().to_string(),
                "status": "pending",
            })
        });
    }

    // Actually, the parallel approach won't work cleanly with the trait object
    // reference. Let's run sequentially with a semaphore instead.
    // Drop the join_set and re-do this properly.
    drop(join_set);

    // Re-resolve tasks for sequential execution.
    let mut results = immediate_errors;

    // Re-parse tasks.
    let mut task_params: Vec<(String, String, GuestBootstrapParams)> = Vec::new();
    for guest_id in &discovered_guests {
        // Skip guests that already had errors.
        if results.iter().any(|r| {
            r.get("mapping_id")
                .and_then(|v| v.as_str())
                .is_some_and(|v| v == guest_id)
        }) {
            continue;
        }

        let guest = options.iter().find(|o| {
            o.get("value")
                .and_then(|v| v.as_str())
                .is_some_and(|v| v == guest_id)
        });
        let Some(guest) = guest else { continue };

        let vmid = guest
            .get("proxmox_vmid")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as u32;
        let guest_type_str = guest
            .get("proxmox_type")
            .and_then(|v| v.as_str())
            .unwrap_or("lxc");
        let proxmox_node = guest
            .get("proxmox_node")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let plugin_config_id = guest
            .get("plugin_config_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let Some(pve_host_id) = pve_host_map.get(&(proxmox_node, plugin_config_id)).cloned() else {
            continue;
        };

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
        task_params.push((
            guest_id.clone(),
            name.clone(),
            GuestBootstrapParams::new(
                pve_host_id,
                vmid,
                guest_type_str,
                name,
                target_username.clone(),
                allow_all,
                remove_stale_keys,
                host_id,
                ctx.service_id,
            ),
        ));
    }

    // Run with bounded concurrency via semaphore.
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_BOOTSTRAPS));

    // Since we can't move the trait object into spawned tasks, we use
    // `JoinSet` with `spawn_on` won't work either. Instead, use
    // `tokio::task::JoinSet` with locally-scoped futures via a channel.
    // The simplest correct approach: collect all params, then spawn tasks
    // that call back to us. But GuestBootstrapExecutor is Send+Sync...
    // we just need to Arc-wrap it if possible.

    // Actually, let's just use futures::stream with buffered concurrency.
    // This avoids the 'static lifetime issue entirely.
    use futures_util::StreamExt;

    let bootstrap_futures: Vec<_> = task_params
        .into_iter()
        .map(|(mapping_id, name, params)| {
            let sem = std::sync::Arc::clone(&semaphore);
            let host_id = params.host_id;

            async move {
                let _permit = sem.acquire().await.expect("semaphore is never closed");

                match ctx.guest_bootstrap.bootstrap_guest(params).await {
                    Ok(result) => {
                        tracing::info!(
                            %host_id,
                            hostname = %result.hostname,
                            mapping_id = %mapping_id,
                            "discovered guest bootstrap completed"
                        );

                        // Defer the Proxmox mapping match.
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

// ── Helpers ──────────────────────────────────────────────────────────────────

fn param_bool(params: &serde_json::Value, key: &str) -> bool {
    match params.get(key) {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s == "true",
        _ => false,
    }
}

fn make_success_response(request_id: &str, data: serde_json::Value) -> ExtensionResponsePayload {
    ExtensionResponsePayload {
        request_id: request_id.to_string(),
        success: true,
        data,
        error: None,
    }
}

fn make_error_response(request_id: &str, message: &str) -> ExtensionResponsePayload {
    ExtensionResponsePayload {
        request_id: request_id.to_string(),
        success: false,
        data: serde_json::Value::Null,
        error: Some(message.to_string()),
    }
}
