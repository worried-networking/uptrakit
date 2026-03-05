//! Update execution module for Uptrakit agents.
//!
//! Handles the complete update flow:
//! 1. Receive `ExecuteUpdate` message
//! 2. Detect current version (`from_version`)
//! 3. Run user shell pre-update hooks (from wire payload)
//! 4. Run plugin's `pre_update_hook` — abort if `should_proceed` is false
//! 5. Execute actual update (dispatched through Plugin Registry)
//! 6. Run plugin's `post_update_hook` — errors logged, non-fatal
//! 7. Run user shell post-update hooks (from wire payload)
//! 8. Detect to_version post-update
//! 9. Return `UpdateExecutionResult` with final status and accumulated output
//!
//! ## Shell Execution
//!
//! Commands are executed with fail-early shell settings:
//! - **Bash**: `set -euo pipefail` (exit on error, undefined vars, pipe failures)
//! - **Sh**: `set -eu` (exit on error, undefined vars)

use std::sync::Arc;

use rootcause::prelude::*;
use thiserror::Error as ThisError;
use tokio::sync::mpsc;
use uptrakit_command::{CommandExecutor, UpdateOutputLine};
use uptrakit_internal_wire::{
    AttestationStatus, ExecuteUpdatePayload, HookCommand, OutputStreamType, ReleaseInfo,
    UpdateFinalStatus, UpdateResultPayload,
};
use uptrakit_plugin_infrastructure_core::UpdateHookContext;
use uptrakit_plugin_infrastructure_registry::PluginRegistry;

use crate::error::AgentCoreError;

/// Maximum accumulated output size (10 MB) to prevent OOM from runaway commands.
const MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

/// Maximum execution time for a single hook command (5 minutes).
///
/// Prevents a single hook from consuming the entire update timeout budget.
/// The child process is killed via `kill_on_drop(true)` when the timeout
/// future is dropped.
const HOOK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Marker appended when output is truncated at the limit.
const TRUNCATION_MARKER: &str = "\n... [output truncated at 10 MB] ...\n";

/// Append text to a bounded buffer. Once the buffer reaches `max` bytes,
/// further text is silently dropped and a single truncation marker is appended.
fn append_bounded(buffer: &mut String, text: &str, max: usize) {
    if buffer.len() >= max {
        return;
    }
    let remaining = max - buffer.len();
    if text.len() <= remaining {
        buffer.push_str(text);
    } else {
        buffer.push_str(&text[..remaining]);
        buffer.push_str(TRUNCATION_MARKER);
    }
}

/// Errors that can occur during update execution.
#[derive(Debug, ThisError)]
pub(crate) enum UpdateError {
    #[error("install command failed: {0}")]
    InstallFailed(String),

    #[error("hook execution failed: {0}")]
    HookFailed(String),
}

pub(crate) type UpdateResult<T> = std::result::Result<T, Report<UpdateError>>;

/// Result of an update execution.
pub struct UpdateExecutionResult {
    pub result: UpdateResultPayload,
}

/// Output message sent during update execution.
pub struct UpdateOutputMessage {
    pub output: String,
    pub stream: OutputStreamType,
}

/// Execute an update based on the payload.
///
/// This function runs the complete update flow and sends output lines through
/// the provided channel. The channel receiver should forward these as
/// `UpdateOutput` messages to the controller.
///
/// The `executor` parameter controls where commands run — `LocalCommandExecutor`
/// for the regular agent, `SshCommandExecutor` for the SSH agent.
#[tracing::instrument(skip_all, fields(software_item = %payload.software_item_name, update_history_id = %payload.update_history_id))]
pub async fn execute_update(
    payload: ExecuteUpdatePayload,
    executor: Arc<dyn CommandExecutor>,
    output_tx: mpsc::Sender<UpdateOutputMessage>,
) -> UpdateExecutionResult {
    tracing::info!(software_item = %payload.software_item_name, "starting update");
    let update_history_id = payload.update_history_id;

    // Detect current version (from_version)
    let from_version = detect_current_version(&payload, Arc::clone(&executor)).await;
    tracing::debug!(from_version = ?from_version, "detected current version before update");

    let mut accumulated_output = String::new();
    let mut final_error: Option<String> = None;
    let mut final_status = UpdateFinalStatus::Completed;

    // Run with timeout
    let timeout_duration = std::time::Duration::from_secs(u64::from(payload.timeout_seconds));
    let execution_result = tokio::time::timeout(timeout_duration, async {
        // Run pre-update hooks
        if !payload.pre_update_hooks.is_empty() {
            tracing::warn!(
                hook_count = payload.pre_update_hooks.len(),
                commands = %hook_summaries(&payload.pre_update_hooks),
                "security_audit: executing pre-update hooks"
            );
            send_output(
                &output_tx,
                "[pre-hook] Starting pre-update hooks...",
                OutputStreamType::System,
            )
            .await;

            for hook_cmd in &payload.pre_update_hooks {
                send_output(
                    &output_tx,
                    &format!("[pre-hook] Running: {hook_cmd}"),
                    OutputStreamType::PreHook,
                )
                .await;

                match run_hook_command(hook_cmd, OutputStreamType::PreHook, &output_tx).await {
                    Ok((output, exit_code)) => {
                        append_bounded(&mut accumulated_output, &output, MAX_OUTPUT_BYTES);
                        send_output(
                            &output_tx,
                            &format!("[pre-hook] (exit code {exit_code})"),
                            OutputStreamType::PreHook,
                        )
                        .await;
                    }
                    Err(e) => {
                        let error_msg = format!("[pre-hook] Failed: {e}");
                        send_output(&output_tx, &error_msg, OutputStreamType::System).await;
                        return Err(AgentCoreError::PreUpdateHookFailed(e.to_string()));
                    }
                }
            }
        }

        // Attestation gate — abort if policy requires a verified attestation
        // and none was found.
        check_attestation_gate(payload.release_info.as_ref(), &output_tx).await?;

        // Execute actual update based on plugin type
        send_output(
            &output_tx,
            &format!(
                "[update] Executing update to version {}...",
                payload.to_version
            ),
            OutputStreamType::System,
        )
        .await;

        tracing::debug!("executing plugin update");
        match execute_plugin_update(&payload, &output_tx, Arc::clone(&executor)).await {
            Ok(output) => {
                tracing::debug!("plugin update returned successfully");
                append_bounded(&mut accumulated_output, &output, MAX_OUTPUT_BYTES);
            }
            Err(e) => {
                let error_msg = e.to_string();
                tracing::warn!(
                    software_item = %payload.software_item_name,
                    error = %error_msg,
                    "plugin update command failed"
                );
                let formatted = format!("[error] {error_msg}\n");
                send_output(
                    &output_tx,
                    &format!("[error] {error_msg}"),
                    OutputStreamType::Stderr,
                )
                .await;
                append_bounded(&mut accumulated_output, &formatted, MAX_OUTPUT_BYTES);
                return Err(AgentCoreError::UpdateExecution(error_msg));
            }
        }

        // Run post-update hooks
        if !payload.post_update_hooks.is_empty() {
            tracing::warn!(
                hook_count = payload.post_update_hooks.len(),
                commands = %hook_summaries(&payload.post_update_hooks),
                "security_audit: executing post-update hooks"
            );
            send_output(
                &output_tx,
                "[post-hook] Starting post-update hooks...",
                OutputStreamType::System,
            )
            .await;

            for hook_cmd in &payload.post_update_hooks {
                send_output(
                    &output_tx,
                    &format!("[post-hook] Running: {hook_cmd}"),
                    OutputStreamType::PostHook,
                )
                .await;

                match run_hook_command(hook_cmd, OutputStreamType::PostHook, &output_tx).await {
                    Ok((output, exit_code)) => {
                        append_bounded(&mut accumulated_output, &output, MAX_OUTPUT_BYTES);
                        send_output(
                            &output_tx,
                            &format!("[post-hook] (exit code {exit_code})"),
                            OutputStreamType::PostHook,
                        )
                        .await;
                    }
                    Err(e) => {
                        let error_msg = format!("[post-hook] Failed: {e}");
                        send_output(&output_tx, &error_msg, OutputStreamType::System).await;
                        return Err(AgentCoreError::PostUpdateHookFailed(e.to_string()));
                    }
                }
            }
        }

        Ok(())
    })
    .await;

    // Handle timeout or execution result
    let to_version = match execution_result {
        Ok(Ok(())) => {
            tracing::info!(software_item = %payload.software_item_name, "update completed successfully");
            send_output(
                &output_tx,
                "[update] Update completed successfully",
                OutputStreamType::System,
            )
            .await;
            // Detect new version after update
            let to_version = detect_current_version(&payload, Arc::clone(&executor)).await;
            tracing::debug!(to_version = ?to_version, "post-update version detected");
            to_version
        }
        Ok(Err(e)) => {
            // The error was already logged and appended to accumulated_output in the
            // execute_plugin_update error arm above; here we only set the final state.
            final_status = UpdateFinalStatus::Failed;
            final_error = Some(e.to_string());
            None
        }
        Err(_) => {
            let timeout_msg = format!("Update timed out after {} seconds", payload.timeout_seconds);
            tracing::warn!(
                software_item = %payload.software_item_name,
                timeout_seconds = payload.timeout_seconds,
                "update timed out"
            );
            let formatted = format!("[error] {timeout_msg}\n");
            send_output(
                &output_tx,
                &format!("[update] {timeout_msg}"),
                OutputStreamType::System,
            )
            .await;
            append_bounded(&mut accumulated_output, &formatted, MAX_OUTPUT_BYTES);
            final_status = UpdateFinalStatus::Failed;
            final_error = Some(timeout_msg);
            None
        }
    };

    let result = UpdateResultPayload {
        update_history_id,
        status: final_status,
        from_version,
        to_version,
        output: accumulated_output,
        error: final_error,
    };

    UpdateExecutionResult { result }
}

/// Detect the current version of a software item by delegating to the plugin registry.
///
/// Uses the `detect_version_plugin` from the payload if available.
async fn detect_current_version(
    payload: &ExecuteUpdatePayload,
    executor: Arc<dyn CommandExecutor>,
) -> Option<String> {
    // The connection context has already been merged into the plugin config
    // by the caller before the update task was spawned.
    let detect_assignment = payload.detect_version_plugin.as_ref();
    let outcome = crate::version_check::check_version(
        detect_assignment,
        None,
        executor,
        &crate::connection_context::ConnectionContext::default(),
    )
    .await;
    if let Some(e) = outcome.error {
        tracing::warn!(
            software_item = %payload.software_item_name,
            error = %e,
            "failed to detect current version"
        );
    }
    tracing::debug!(version = ?outcome.installed_version, "current version detected");
    outcome.installed_version
}

/// Execute the plugin-specific update logic, including pre/post lifecycle hooks.
///
/// Plugin-level hooks run adjacent to `execute_update`:
/// ```text
/// plugin.pre_update_hook(ctx, tx)   ← abort if !should_proceed
/// plugin.execute_update(...)         ← the actual update
/// plugin.post_update_hook(ctx, tx)   ← errors logged at WARN, non-fatal
/// ```
#[tracing::instrument(skip_all, fields(plugin_type = %payload.execute_update_plugin.plugin_type))]
async fn execute_plugin_update(
    payload: &ExecuteUpdatePayload,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
    executor: Arc<dyn CommandExecutor>,
) -> UpdateResult<String> {
    let eu = &payload.execute_update_plugin;
    let plugin = PluginRegistry::create_plugin(eu.plugin_type.clone(), &eu.config, executor)
        .await
        .map_err(|e| report!(UpdateError::InstallFailed(e.to_string())))?;

    let hook_ctx = UpdateHookContext {
        package_identifier: eu.package_identifier.clone(),
        to_version: payload.to_version.clone(),
        release_info: payload.release_info.clone(),
    };

    // Bridge plugin output (UpdateOutputLine) -> agent output (UpdateOutputMessage)
    let make_bridge = |output_tx: &mpsc::Sender<UpdateOutputMessage>| {
        let (plugin_tx, mut plugin_rx) = mpsc::channel::<UpdateOutputLine>(100);
        let bridge_output_tx = output_tx.clone();
        let bridge_handle = tokio::spawn(async move {
            while let Some(line) = plugin_rx.recv().await {
                let _ = bridge_output_tx
                    .send(UpdateOutputMessage {
                        output: line.text,
                        stream: line.stream,
                    })
                    .await;
            }
        });
        (plugin_tx, bridge_handle)
    };

    // --- Pre-update hook ---
    {
        let (plugin_tx, bridge_handle) = make_bridge(output_tx);
        let pre_result = plugin
            .pre_update_hook(&hook_ctx, &plugin_tx)
            .await
            .map_err(|e| {
                // Avoid "install command failed: install command failed: ..." by
                // extracting the inner message when the plugin already wrapped the
                // error in PluginError::InstallFailed.
                use uptrakit_plugin_infrastructure_core::PluginError;
                let msg = match e.current_context() {
                    PluginError::InstallFailed(s) => s.clone(),
                    other => other.to_string(),
                };
                report!(UpdateError::InstallFailed(msg))
            });
        drop(plugin_tx);
        let _ = bridge_handle.await;

        let pre_result = pre_result?;
        if !pre_result.should_proceed {
            let reason = pre_result
                .abort_reason
                .unwrap_or_else(|| "plugin pre-update hook aborted the update".to_string());
            tracing::warn!(reason, "plugin pre_update_hook aborted the update");
            return Err(report!(UpdateError::InstallFailed(reason)));
        }
    }

    // --- Execute update ---
    let (plugin_tx, bridge_handle) = make_bridge(output_tx);
    let update_result = plugin
        .execute_update(
            &eu.package_identifier,
            &payload.to_version,
            payload.release_info.as_ref(),
            &plugin_tx,
        )
        .await
        .map_err(|e| {
            // Avoid "install command failed: install command failed: ..." by
            // extracting the inner message when the plugin already wrapped the
            // error in PluginError::InstallFailed.
            use uptrakit_plugin_infrastructure_core::PluginError;
            let msg = match e.current_context() {
                PluginError::InstallFailed(s) => s.clone(),
                other => other.to_string(),
            };
            report!(UpdateError::InstallFailed(msg))
        });
    drop(plugin_tx);
    let _ = bridge_handle.await;
    let update_output = update_result?;

    // --- Post-update hook ---
    {
        let (plugin_tx, bridge_handle) = make_bridge(output_tx);
        let post_result = plugin.post_update_hook(&hook_ctx, &plugin_tx).await;
        drop(plugin_tx);
        let _ = bridge_handle.await;

        if let Err(e) = post_result {
            tracing::warn!(
                error = %e,
                "plugin post_update_hook failed (non-fatal); continuing"
            );
        }
    }

    Ok(update_output)
}

/// Execute a `HookCommand`, dispatching to shell or direct exec as appropriate.
///
/// Each hook is wrapped in [`HOOK_TIMEOUT`] — if a single hook exceeds the
/// limit, its child process is killed (`kill_on_drop(true)`) and a
/// `HookFailed` error is returned.
#[tracing::instrument(skip_all, fields(hook = ?hook_cmd))]
async fn run_hook_command(
    hook_cmd: &HookCommand,
    stream_type: OutputStreamType,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> UpdateResult<(String, i32)> {
    tracing::debug!(hook = ?hook_cmd, "running update hook");

    match tokio::time::timeout(
        HOOK_TIMEOUT,
        run_hook_command_inner(hook_cmd, stream_type, output_tx),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            let summary = hook_summary(hook_cmd);
            tracing::warn!(
                hook = summary,
                timeout_secs = HOOK_TIMEOUT.as_secs(),
                "security_audit: hook command timed out"
            );
            Err(report!(UpdateError::HookFailed(format!(
                "hook timed out after {} seconds: {summary}",
                HOOK_TIMEOUT.as_secs()
            ))))
        }
    }
}

/// Inner implementation of hook execution (no timeout wrapper).
async fn run_hook_command_inner(
    hook_cmd: &HookCommand,
    stream_type: OutputStreamType,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> UpdateResult<(String, i32)> {
    // Bridge plugin output -> agent output
    let (plugin_tx, mut plugin_rx) = mpsc::channel::<UpdateOutputLine>(100);
    let bridge_output_tx = output_tx.clone();
    let bridge_stream_type = stream_type;
    let bridge_handle = tokio::spawn(async move {
        while let Some(line) = plugin_rx.recv().await {
            let stream = match line.stream {
                OutputStreamType::Stdout => bridge_stream_type,
                _ => OutputStreamType::Stderr,
            };
            let _ = bridge_output_tx
                .send(UpdateOutputMessage {
                    output: line.text,
                    stream,
                })
                .await;
        }
    });

    let result = match hook_cmd {
        HookCommand::Shell { command, shell } => {
            uptrakit_command::run_command_with_shell(command, *shell, &plugin_tx).await
        }
        HookCommand::Exec {
            program,
            args,
            working_dir,
        } => {
            uptrakit_command::run_command_exec(program, args, working_dir.as_deref(), &plugin_tx)
                .await
        }
        _ => {
            // Unknown HookCommand variant — warn and skip. This is the forward-compatibility
            // contract for #[non_exhaustive] HookCommand: an older agent must never abort an
            // update when a newer controller sends a hook type it does not recognise. Skipping
            // allows the rest of the update pipeline to proceed normally.
            // See docs/development/coding-standards.md § "#[non_exhaustive] on public enums".
            tracing::warn!("unknown HookCommand variant; skipping hook for forward-compatibility");
            drop(plugin_tx);
            let _ = bridge_handle.await;
            return Ok((String::new(), 0));
        }
    };

    // Drop the sender so the bridge task finishes
    drop(plugin_tx);
    let _ = bridge_handle.await;

    let result = result.map_err(|e| report!(UpdateError::HookFailed(e.to_string())));
    if let Ok((_, exit_code)) = &result {
        tracing::debug!(exit_code, "hook completed");
    }
    result
}

/// Execute a hook command for batch operations.
///
/// Unlike `run_hook_command` (which streams output into an update output
/// channel), this variant is fire-and-forget: it runs the hook, checks the
/// exit code, and returns an error if non-zero.
///
/// Each hook is wrapped in [`HOOK_TIMEOUT`].
pub(crate) async fn run_hook_for_batch(hook_cmd: &HookCommand) -> UpdateResult<()> {
    match tokio::time::timeout(HOOK_TIMEOUT, run_hook_for_batch_inner(hook_cmd)).await {
        Ok(result) => result,
        Err(_) => {
            let summary = hook_summary(hook_cmd);
            tracing::warn!(
                hook = summary,
                timeout_secs = HOOK_TIMEOUT.as_secs(),
                "security_audit: batch hook command timed out"
            );
            Err(report!(UpdateError::HookFailed(format!(
                "hook timed out after {} seconds: {summary}",
                HOOK_TIMEOUT.as_secs()
            ))))
        }
    }
}

/// Inner implementation of batch hook execution (no timeout wrapper).
async fn run_hook_for_batch_inner(hook_cmd: &HookCommand) -> UpdateResult<()> {
    let (plugin_tx, mut plugin_rx) = mpsc::channel::<UpdateOutputLine>(100);
    // Drain output in the background — we don't stream it for batch hooks.
    let drain_handle = tokio::spawn(async move { while plugin_rx.recv().await.is_some() {} });

    let result = match hook_cmd {
        HookCommand::Shell { command, shell } => {
            uptrakit_command::run_command_with_shell(command, *shell, &plugin_tx).await
        }
        HookCommand::Exec {
            program,
            args,
            working_dir,
        } => {
            uptrakit_command::run_command_exec(program, args, working_dir.as_deref(), &plugin_tx)
                .await
        }
        _ => {
            // Unknown HookCommand variant — warn and skip (see run_hook_command_inner for the
            // full rationale). The forward-compatibility contract for #[non_exhaustive] enums
            // requires skipping, not aborting, on unknown variants.
            tracing::warn!(
                "unknown HookCommand variant; skipping batch hook for forward-compatibility"
            );
            drop(plugin_tx);
            let _ = drain_handle.await;
            return Ok(());
        }
    };

    drop(plugin_tx);
    let _ = drain_handle.await;

    result
        .map(|(_, _)| ())
        .map_err(|e| report!(UpdateError::HookFailed(e.to_string())))
}

/// Produce a short summary of a hook command for audit logging.
fn hook_summary(hook_cmd: &HookCommand) -> String {
    match hook_cmd {
        HookCommand::Shell { command, shell } => {
            let truncated = if command.len() > 80 {
                format!("{}…", &command[..80])
            } else {
                command.clone()
            };
            format!("{shell:?}: {truncated}")
        }
        HookCommand::Exec { program, args, .. } => {
            let args_summary = if args.len() > 3 {
                format!("{} (+{} more)", args[..3].join(" "), args.len() - 3)
            } else {
                args.join(" ")
            };
            format!("exec: {program} {args_summary}")
        }
        _ => "unknown".to_string(),
    }
}

/// Produce a comma-separated summary of all hooks for audit logging.
fn hook_summaries(hooks: &[HookCommand]) -> String {
    hooks
        .iter()
        .map(hook_summary)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Send an output message.
async fn send_output(
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
    message: &str,
    stream: OutputStreamType,
) {
    let _ = output_tx
        .send(UpdateOutputMessage {
            output: message.to_string(),
            stream,
        })
        .await;
}

// ---------------------------------------------------------------------------
// Attestation gate
// ---------------------------------------------------------------------------

/// Parse `owner` and `repo` from a GitHub HTML release URL.
///
/// Accepts `https://github.com/{owner}/{repo}/releases/...` and returns
/// `Some((owner, repo))`, or `None` for any other URL format.
fn parse_github_owner_repo(url: &str) -> Option<(String, String)> {
    let path = url.strip_prefix("https://github.com/")?;
    let mut parts = path.splitn(3, '/');
    let owner = parts.next().filter(|s| !s.is_empty())?.to_string();
    let repo = parts.next().filter(|s| !s.is_empty())?.to_string();
    Some((owner, repo))
}

/// Query the GitHub Attestations API for the given asset SHA-256 digest.
///
/// Returns:
/// - [`AttestationStatus::Verified`] if one or more attestations are found.
/// - [`AttestationStatus::NotFound`] if the API returns 404 or an empty list.
/// - [`AttestationStatus::Unverified`] on any network or parse error.
async fn independently_verify_attestation(
    owner: &str,
    repo: &str,
    digest_hex: &str,
) -> AttestationStatus {
    #[derive(serde::Deserialize)]
    struct ApiResponse {
        #[serde(default)]
        attestations: Vec<serde_json::Value>,
    }

    let url =
        format!("https://api.github.com/repos/{owner}/{repo}/attestations/sha256:{digest_hex}");

    let client = match reqwest::Client::builder()
        .user_agent(concat!("uptrakit-agent/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "failed to build reqwest client for attestation check");
            return AttestationStatus::Unverified;
        }
    };

    let response = match client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "attestation API request failed");
            return AttestationStatus::Unverified;
        }
    };

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return AttestationStatus::NotFound;
    }

    if !response.status().is_success() {
        tracing::warn!(
            status = %response.status(),
            "attestation API returned unexpected status"
        );
        return AttestationStatus::Unverified;
    }

    match response.json::<ApiResponse>().await {
        Ok(body) if !body.attestations.is_empty() => AttestationStatus::Verified,
        Ok(_) => AttestationStatus::NotFound,
        Err(e) => {
            tracing::warn!(error = %e, "failed to parse attestation API response");
            AttestationStatus::Unverified
        }
    }
}

/// Pre-install attestation gate.
///
/// Checks the GitHub Actions attestation for the release before allowing the
/// update to proceed. The gate is skipped when:
///
/// - `release_info` is `None` (no GitHub release source).
/// - The release URL does not parse as a `https://github.com/{owner}/{repo}/…`
///   URL (non-GitHub update paths are unaffected).
/// - No asset with a known `sha256_digest` is available for re-verification.
///
/// When the controller already set `attestation_status = Verified`, the agent
/// trusts that verdict and returns `Ok` immediately without a live API call.
///
/// Otherwise, the agent independently re-queries the GitHub Attestations API
/// using the first asset's `sha256_digest`. The result governs whether to
/// block (`require_attestation = true`) or warn.
async fn check_attestation_gate(
    release_info: Option<&ReleaseInfo>,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> std::result::Result<(), AgentCoreError> {
    let Some(ri) = release_info else {
        return Ok(());
    };

    let Some((owner, repo)) = parse_github_owner_repo(&ri.release_url) else {
        return Ok(());
    };

    // Trust the controller's Verified verdict — already confirmed at fetch time.
    if ri.attestation_status == Some(AttestationStatus::Verified) {
        send_output(
            output_tx,
            &format!("[attestation] GitHub Actions attestation verified for {owner}/{repo}"),
            OutputStreamType::System,
        )
        .await;
        return Ok(());
    }

    // Independent re-verify using an asset SHA-256 digest.
    let digest = ri.assets.iter().find_map(|a| a.sha256_digest.as_deref());
    let Some(digest) = digest else {
        send_output(
            output_tx,
            "[attestation] No asset digest available for independent attestation check; proceeding",
            OutputStreamType::System,
        )
        .await;
        return Ok(());
    };

    let verified_status = independently_verify_attestation(&owner, &repo, digest).await;

    match verified_status {
        AttestationStatus::Verified => {
            send_output(
                output_tx,
                &format!(
                    "[attestation] GitHub Actions attestation independently verified for \
                     {owner}/{repo}"
                ),
                OutputStreamType::System,
            )
            .await;
            Ok(())
        }
        AttestationStatus::NotFound => {
            if ri.require_attestation {
                let msg = format!(
                    "no GitHub Actions attestation found for {owner}/{repo}; \
                     update blocked by require_attestation policy"
                );
                send_output(
                    output_tx,
                    &format!("[attestation] [error] {msg}"),
                    OutputStreamType::Stderr,
                )
                .await;
                Err(AgentCoreError::AttestationFailed(msg))
            } else {
                send_output(
                    output_tx,
                    &format!(
                        "[attestation] [warning] No GitHub Actions attestation found for \
                         {owner}/{repo}. Proceeding (require_attestation is false)."
                    ),
                    OutputStreamType::System,
                )
                .await;
                Ok(())
            }
        }
        AttestationStatus::Unverified => {
            send_output(
                output_tx,
                &format!(
                    "[attestation] Attestation check inconclusive for {owner}/{repo}; \
                     proceeding with install"
                ),
                OutputStreamType::System,
            )
            .await;
            Ok(())
        }
        _ => {
            // Forward-compatible: treat unknown statuses as Unverified.
            send_output(
                output_tx,
                "[attestation] Attestation check returned unknown status; proceeding with install",
                OutputStreamType::System,
            )
            .await;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_command::LocalCommandExecutor;
    use uptrakit_internal_wire::{HookShell, PluginType};

    fn test_payload() -> ExecuteUpdatePayload {
        ExecuteUpdatePayload {
            host_machine_id: String::new(),
            update_history_id: uuid::Uuid::nil(),
            software_item_id: uuid::Uuid::nil(),
            software_item_name: "Test App".to_string(),
            to_version: "2.0.0".to_string(),
            detect_version_plugin: None,
            execute_update_plugin: uptrakit_internal_wire::PluginAssignment {
                plugin_type: PluginType::ReleasesGithub,
                package_identifier: "test-app".to_string(),
                config: serde_json::json!({}),
            },
            pre_update_hooks: vec![],
            post_update_hooks: vec![],
            release_info: None,
            timeout_seconds: 60,
        }
    }

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    // ── Bounded output tests ────────────────────────────────────────────────

    #[test]
    fn append_bounded_below_limit() {
        let mut buf = String::new();
        append_bounded(&mut buf, "hello", 100);
        assert_eq!(buf, "hello");
    }

    #[test]
    fn append_bounded_at_limit_truncates() {
        let mut buf = String::new();
        append_bounded(&mut buf, "abcde", 3);
        assert!(buf.starts_with("abc"));
        assert!(buf.contains(TRUNCATION_MARKER));
    }

    #[test]
    fn append_bounded_already_full_drops() {
        let mut buf = "x".repeat(100);
        append_bounded(&mut buf, "more data", 100);
        assert_eq!(buf.len(), 100);
    }

    #[test]
    fn append_bounded_exact_fit() {
        let mut buf = String::new();
        append_bounded(&mut buf, "abc", 3);
        assert_eq!(buf, "abc");
        append_bounded(&mut buf, "d", 3);
        assert_eq!(buf, "abc");
    }

    // ── Hook execution tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_execute_update_with_pre_hook() {
        let (tx, mut rx) = mpsc::channel(100);

        let mut payload = test_payload();
        payload.pre_update_hooks = vec![HookCommand::Shell {
            command: "echo 'pre-hook executed'".to_string(),
            shell: HookShell::Bash,
        }];
        payload.release_info = None;
        payload.execute_update_plugin.config = serde_json::json!({});

        let result = execute_update(payload, test_executor(), tx).await;

        assert_eq!(result.result.update_history_id, uuid::Uuid::nil());

        rx.close();
        let mut found_pre_hook = false;
        while let Some(msg) = rx.recv().await {
            if msg.output.contains("pre-hook executed") {
                found_pre_hook = true;
            }
        }
        assert!(found_pre_hook);
    }

    #[tokio::test]
    async fn test_execute_update_pre_hook_failure() {
        let (tx, mut rx) = mpsc::channel(100);

        let mut payload = test_payload();
        payload.pre_update_hooks = vec![HookCommand::Shell {
            command: "exit 1".to_string(),
            shell: HookShell::Bash,
        }];

        let result = execute_update(payload, test_executor(), tx).await;

        assert_eq!(result.result.status, UpdateFinalStatus::Failed);
        assert!(result.result.error.is_some(), "Expected error but got None");
        let error_msg = result.result.error.as_ref().unwrap();
        assert!(
            error_msg.contains("Pre-update hook failed"),
            "Expected error to contain 'Pre-update hook failed', got: {error_msg}"
        );

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn test_execute_update_with_sh_shell() {
        let (tx, mut rx) = mpsc::channel(100);

        let mut payload = test_payload();
        payload.pre_update_hooks = vec![HookCommand::Shell {
            command: "echo 'using sh shell'".to_string(),
            shell: HookShell::Sh,
        }];

        let result = execute_update(payload, test_executor(), tx).await;

        assert_eq!(result.result.update_history_id, uuid::Uuid::nil());

        rx.close();
        let mut found_output = false;
        while let Some(msg) = rx.recv().await {
            if msg.output.contains("using sh shell") {
                found_output = true;
            }
        }
        assert!(found_output);
    }

    // ── Per-hook timeout tests ────────────────────────────────────────────

    #[tokio::test(start_paused = true)]
    async fn run_hook_command_timeout() {
        let (tx, _rx) = mpsc::channel(100);
        let hook = HookCommand::Shell {
            command: "sleep 600".to_string(),
            shell: HookShell::Bash,
        };
        let result = run_hook_command(&hook, OutputStreamType::PreHook, &tx).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("timed out"),
            "expected timeout error, got: {err_msg}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn run_hook_for_batch_timeout() {
        let hook = HookCommand::Shell {
            command: "sleep 600".to_string(),
            shell: HookShell::Bash,
        };
        let result = run_hook_for_batch(&hook).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("timed out"),
            "expected timeout error, got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn run_hook_command_completes_within_timeout() {
        let (tx, _rx) = mpsc::channel(100);
        let hook = HookCommand::Shell {
            command: "echo hello".to_string(),
            shell: HookShell::Bash,
        };
        let result = run_hook_command(&hook, OutputStreamType::PreHook, &tx).await;
        assert!(result.is_ok());
        let (output, exit_code) = result.unwrap();
        assert_eq!(exit_code, 0);
        assert!(output.contains("hello"));
    }

    // ── Forward-compatibility: known variants must succeed ───────────────
    //
    // The `#[non_exhaustive]` contract on `HookCommand` requires that the
    // wildcard arm in `run_hook_command_inner` / `run_hook_for_batch_inner`
    // warns and skips rather than returning an error.  The unknown-variant
    // path cannot be exercised directly within this crate (adding a new
    // variant requires a recompile), but these tests confirm that every
    // *known* variant is dispatched without error, and that the fallthrough
    // `_ =>` arm is present and correct by code inspection.

    #[tokio::test]
    async fn run_hook_command_shell_returns_ok() {
        let (tx, _rx) = mpsc::channel(100);
        let hook = HookCommand::Shell {
            command: "true".to_string(),
            shell: HookShell::Sh,
        };
        assert!(
            run_hook_command(&hook, OutputStreamType::PreHook, &tx)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn run_hook_command_exec_returns_ok() {
        let (tx, _rx) = mpsc::channel(100);
        let hook = HookCommand::Exec {
            program: "true".to_string(),
            args: vec![],
            working_dir: None,
        };
        assert!(
            run_hook_command(&hook, OutputStreamType::PreHook, &tx)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn run_hook_for_batch_shell_returns_ok() {
        let hook = HookCommand::Shell {
            command: "true".to_string(),
            shell: HookShell::Sh,
        };
        assert!(run_hook_for_batch(&hook).await.is_ok());
    }

    #[tokio::test]
    async fn run_hook_for_batch_exec_returns_ok() {
        let hook = HookCommand::Exec {
            program: "true".to_string(),
            args: vec![],
            working_dir: None,
        };
        assert!(run_hook_for_batch(&hook).await.is_ok());
    }

    // ── Hook summary tests ───────────────────────────────────────────────

    #[test]
    fn hook_summary_shell() {
        let hook = HookCommand::Shell {
            command: "echo test".to_string(),
            shell: HookShell::Bash,
        };
        let summary = hook_summary(&hook);
        assert!(summary.contains("echo test"));
    }

    #[test]
    fn hook_summary_long_command_truncated() {
        let hook = HookCommand::Shell {
            command: "a".repeat(200),
            shell: HookShell::Bash,
        };
        let summary = hook_summary(&hook);
        assert!(summary.len() < 200);
        assert!(summary.contains('…'));
    }

    #[test]
    fn hook_summary_exec() {
        let hook = HookCommand::Exec {
            program: "/usr/bin/test".to_string(),
            args: vec!["--flag".to_string(), "value".to_string()],
            working_dir: None,
        };
        let summary = hook_summary(&hook);
        assert!(summary.contains("/usr/bin/test"));
        assert!(summary.contains("--flag"));
    }

    #[test]
    fn hook_summaries_multiple() {
        let hooks = vec![
            HookCommand::Shell {
                command: "echo 1".to_string(),
                shell: HookShell::Bash,
            },
            HookCommand::Shell {
                command: "echo 2".to_string(),
                shell: HookShell::Sh,
            },
        ];
        let summaries = hook_summaries(&hooks);
        assert!(summaries.contains("echo 1"));
        assert!(summaries.contains("echo 2"));
        assert!(summaries.contains(", "));
    }

    // ── Attestation gate tests ───────────────────────────────────────────

    fn make_release_info_with_status(
        status: Option<AttestationStatus>,
        require: bool,
    ) -> ReleaseInfo {
        ReleaseInfo {
            tag: "v1.0.0".to_string(),
            release_url: "https://github.com/owner/repo/releases/tag/v1.0.0".to_string(),
            assets: vec![uptrakit_internal_wire::ReleaseAsset {
                name: "app.tar.gz".to_string(),
                download_url: "https://github.com/owner/repo/releases/download/v1.0.0/app.tar.gz"
                    .to_string(),
                size: None,
                content_type: None,
                sha256_digest: None,
            }],
            attestation_status: status,
            require_attestation: require,
        }
    }

    #[test]
    fn parse_github_owner_repo_valid() {
        let r = parse_github_owner_repo("https://github.com/owner/repo/releases/tag/v1.0.0");
        assert_eq!(r, Some(("owner".to_string(), "repo".to_string())));
    }

    #[test]
    fn parse_github_owner_repo_non_github() {
        assert!(
            parse_github_owner_repo("https://gitlab.com/owner/repo/releases/tag/v1.0.0").is_none()
        );
    }

    #[test]
    fn parse_github_owner_repo_missing_repo() {
        assert!(parse_github_owner_repo("https://github.com/owner/").is_none());
    }

    #[test]
    fn parse_github_owner_repo_empty_owner() {
        assert!(parse_github_owner_repo("https://github.com//repo/releases").is_none());
    }

    #[tokio::test]
    async fn check_attestation_gate_none_release_info_ok() {
        let (tx, _rx) = mpsc::channel(10);
        let result = check_attestation_gate(None, &tx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn check_attestation_gate_already_verified_ok() {
        let (tx, mut rx) = mpsc::channel(10);
        let ri = make_release_info_with_status(Some(AttestationStatus::Verified), false);
        let result = check_attestation_gate(Some(&ri), &tx).await;
        assert!(result.is_ok());
        rx.close();
        let mut found = false;
        while let Some(msg) = rx.recv().await {
            if msg.output.contains("verified") {
                found = true;
            }
        }
        assert!(found);
    }

    #[tokio::test]
    async fn check_attestation_gate_non_github_url_ok() {
        let (tx, _rx) = mpsc::channel(10);
        let mut ri = make_release_info_with_status(None, true);
        ri.release_url = "https://example.com/releases/v1.0.0".to_string();
        let result = check_attestation_gate(Some(&ri), &tx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn check_attestation_gate_no_digest_ok() {
        let (tx, mut rx) = mpsc::channel(10);
        // No sha256_digest on any asset → skip independent verify.
        let ri = make_release_info_with_status(None, false);
        let result = check_attestation_gate(Some(&ri), &tx).await;
        assert!(result.is_ok());
        rx.close();
        let mut found = false;
        while let Some(msg) = rx.recv().await {
            if msg.output.contains("No asset digest") {
                found = true;
            }
        }
        assert!(found);
    }
}
