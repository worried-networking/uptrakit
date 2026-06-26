//! Best-effort post-update finalization (hook scale-down + protection finalize).

use std::sync::Arc;
use std::time::Duration;

use uptrakit_shared_db::entity::update_history;

use crate::AppState;

/// Run post-update finalization best-effort: the resource-restore hook first
/// (when `plugin-ops` is enabled), then protection finalization.
///
/// `recovery_timeout`:
/// - `None` — normal completion path (`finalize_post_update`).
/// - `Some(t)` — reconnect-recovery path (`finalize_post_update_with_timeout`).
///
/// `context` distinguishes the two paths in warning logs.
pub(super) async fn finalize_post_update_best_effort(
    state: &Arc<AppState>,
    record: &update_history::Model,
    recovery_timeout: Option<Duration>,
) {
    let context = if recovery_timeout.is_some() {
        " during reconnect recovery"
    } else {
        ""
    };

    // Hook first (scale down) — must run before protection finalization.
    #[cfg(feature = "plugin-ops")]
    if let Err(error) = crate::queries::update_dispatch::finalize_post_update_hook(
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
            "post-update hook (resource restore) failed{context}"
        );
    }

    // Then protection finalization.
    let result = match recovery_timeout {
        Some(timeout) => {
            crate::queries::update_dispatch::finalize_post_update_with_timeout(
                state.db(),
                state.controller_update_protection(),
                record,
                timeout,
            )
            .await
        }
        None => {
            crate::queries::update_dispatch::finalize_post_update(
                state.db(),
                state.controller_update_protection(),
                record,
            )
            .await
        }
    };
    if let Err(error) = result {
        tracing::warn!(
            error = %error,
            update_id = %record.id,
            "post-update finalization failed{context}"
        );
    }
}
