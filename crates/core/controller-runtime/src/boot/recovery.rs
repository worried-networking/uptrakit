//! Boot phase: startup recovery.
//!
//! Runs immediately after [`super::app_state::assemble`] returns and before
//! background tasks are spawned.  Performs three operations in order:
//!
//! 1. Emits the GitHub global-provider diagnostic (non-fatal).
//! 2. Marks any in-progress updates as failed (owner-aware rollout cleanup),
//!    then re-dispatches the next queued item for each affected host/batch.
//! 3. Seeds the in-memory token denylist from the database so that
//!    revocations made before a restart are honoured.

use std::sync::Arc;
use std::time::Duration;

use rootcause::prelude::*;
use uptrakit_web_api::AppState;

use crate::AppError;

/// Run all startup-recovery steps.
///
/// All inputs are reachable via `state` accessors — no extra parameters are
/// needed.  The function is intentionally not `pub` beyond `pub(crate)` so
/// that it remains an implementation detail of the boot sequence.
pub(crate) async fn run(state: &Arc<AppState>) -> crate::Result<()> {
    // Step 1: GitHub global-provider diagnostic (non-fatal, best-effort).
    uptrakit_web_api::global_providers::github::emit_global_github_provider_diagnostic_if_needed(
        state.db(),
        &state.notification.event_broadcaster,
    )
    .await;

    // Step 2: Owner-aware rollout cleanup — mark any update records that were
    // left in-progress (e.g. from a previous crash) as failed, then
    // re-dispatch the next queued item for each affected host or batch.
    let recovered =
        uptrakit_web_api::queries::update_batches::mark_all_in_progress_as_failed_for_rollout(
            state.db(),
        )
        .await
        .map_err(|e| {
            report!(AppError::Config(format!(
                "failed to run owner-aware rollout cleanup: {e}"
            )))
        })?;

    if !recovered.is_empty() {
        tracing::warn!(
            count = recovered.len(),
            "marked pre-existing in-progress updates as failed during owner-aware rollout cleanup"
        );

        for record in &recovered {
            #[cfg(feature = "plugin-ops")]
            if let Err(error) =
                uptrakit_web_api::queries::update_dispatch::finalize_post_update_hook(
                    state.db(),
                    state.controller_update_hook(),
                    state.plugin.plugin_ops.as_ref(),
                    record,
                )
                .await
            {
                tracing::warn!(
                    error = %error,
                    update_id = %record.id,
                    "post-update hook (resource restore) failed during startup cleanup"
                );
            }

            if let Err(error) =
                uptrakit_web_api::queries::update_dispatch::finalize_post_update_with_timeout(
                    state.db(),
                    state.controller_update_protection(),
                    record,
                    Duration::from_secs(2),
                )
                .await
            {
                tracing::warn!(
                    error = %error,
                    update_id = %record.id,
                    "post-update finalization failed during startup cleanup"
                );
            }

            if let Some(batch_id) = record.batch_id {
                match uptrakit_web_api::queries::update_batches::dispatch_next_in_batch(
                    state.db(),
                    uptrakit_web_api::queries::update_dispatch::DispatchContext {
                        notifier: &state.notification.notification_service,
                        protection: state.controller_update_protection(),
                        #[cfg(feature = "plugin-ops")]
                        hook: state.controller_update_hook(),
                        #[cfg(feature = "plugin-ops")]
                        notification_ops: Some(state.plugin.plugin_ops.as_ref()),
                    },
                    batch_id,
                    record.host_id,
                    record.tenant_id,
                )
                .await
                {
                    Ok(Some(completion)) => {
                        tracing::debug!(
                            %batch_id,
                            status = %completion.status.as_str(),
                            completed = completion.completed_count,
                            failed = completion.failed_count,
                            "startup rollout cleanup intentionally does not replay retroactive batch-completion notifications"
                        );
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(
                            error = %error,
                            %batch_id,
                            host_id = %record.host_id,
                            "failed to promote next queued batch item after rollout cleanup"
                        );
                    }
                }
            } else if let Err(error) =
                uptrakit_web_api::queries::update_batches::dispatch_next_queued_for_host(
                    state.db(),
                    uptrakit_web_api::queries::update_dispatch::DispatchContext {
                        notifier: &state.notification.notification_service,
                        protection: state.controller_update_protection(),
                        #[cfg(feature = "plugin-ops")]
                        hook: state.controller_update_hook(),
                        #[cfg(feature = "plugin-ops")]
                        notification_ops: Some(state.plugin.plugin_ops.as_ref()),
                    },
                    record.host_id,
                    record.tenant_id,
                )
                .await
            {
                tracing::warn!(
                    error = %error,
                    host_id = %record.host_id,
                    "failed to dispatch next queued update after rollout cleanup"
                );
            }
        }
    }

    // Step 3: Seed the in-memory token denylist from DB before accepting
    // traffic.  This ensures revocations made before a controller restart are
    // honoured.
    state
        .auth
        .token_denylist
        .load_from_db()
        .await
        .map_err(|e| {
            report!(AppError::Config(format!(
                "failed to seed token denylist: {e}"
            )))
        })?;

    Ok(())
}
