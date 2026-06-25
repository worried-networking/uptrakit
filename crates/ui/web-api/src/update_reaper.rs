//! Controller backstop loop: periodically reap `InProgress` updates that have
//! exceeded their budget and force them to a terminal `Interrupted` state.
//!
//! The query work lives in
//! [`uptrakit_web_api_queries::queries::update_reaper::reap_overdue_updates`].
//! This module is the thin glue that drives it on a fixed interval and mirrors
//! the live UI side effects the normal completion path performs: an admin/SSE
//! event per reaped row, a per-tenant software-state refresh, and batch
//! advancement so a batch whose last item is reaped still reaches a terminal
//! status.
//!
//! The loop reads wall-clock time (`OffsetDateTime::now_utc()`) per tick rather
//! than the tokio clock, so a host sleep that freezes the loop is observed as
//! real elapsed time on the first post-wake tick.

use std::sync::Arc;
use std::time::Duration;

use time::OffsetDateTime;
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_wire::DEFAULT_UPDATE_TIMEOUT;

use crate::AppState;

/// How often the reaper sweeps for overdue `InProgress` updates.
const REAPER_INTERVAL: Duration = Duration::from_secs(60);

/// Margin past the update's own budget before the backstop fires, leaving room
/// for agent-side detection (keepalive / interactive timeout) to report first.
const REAPER_GRACE: Duration = Duration::from_secs(300);

/// Spawn the detached background reaper loop.
///
/// Every [`REAPER_INTERVAL`] it reaps `InProgress` updates older than
/// `DEFAULT_UPDATE_TIMEOUT + REAPER_GRACE` and, for each reaped row, emits the
/// same `AdminEvent::UpdateCompleted` the completion path sends (with
/// `status: "interrupted"`), refreshes per-tenant software state, and advances
/// any owning batch.
pub fn spawn_update_reaper(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(REAPER_INTERVAL).await;
            let now = OffsetDateTime::now_utc();
            let reaped =
                match uptrakit_web_api_queries::queries::update_reaper::reap_overdue_updates(
                    state.db(),
                    now,
                    DEFAULT_UPDATE_TIMEOUT + REAPER_GRACE,
                )
                .await
                {
                    Ok(reaped) => reaped,
                    Err(e) => {
                        tracing::error!(error = %e, "update reaper sweep failed");
                        continue;
                    }
                };

            if reaped.is_empty() {
                continue;
            }

            tracing::warn!(
                count = reaped.len(),
                "reaped overdue in_progress updates as Interrupted"
            );

            // Emit one admin/SSE event per reaped row and collect affected
            // tenants for a single software-state refresh each.
            let mut tenants: std::collections::HashSet<uuid::Uuid> =
                std::collections::HashSet::new();
            for row in &reaped {
                tenants.insert(row.tenant_id);
                state
                    .notification
                    .event_broadcaster
                    .send(
                        row.tenant_id,
                        AdminEvent::UpdateCompleted {
                            update_history_id: row.id,
                            host_id: row.host_id,
                            software_item_id: row.software_item_id,
                            status: uptrakit_shared_types::UpdateStatus::Interrupted
                                .as_str()
                                .to_string(),
                        },
                    )
                    .await;
            }

            for tenant_id in tenants {
                state
                    .notification
                    .notification_service
                    .push_software_states_for_tenant(state.db(), tenant_id)
                    .await;
            }

            // Advance any batch that owned a reaped row so a batch whose last
            // item was reaped still reaches a terminal status. Owner-less rows
            // are left to the reconnect-orphan recovery path.
            for row in &reaped {
                if let (Some(batch_id), Some(service_id)) =
                    (row.batch_id, row.execution_owner_service_id)
                {
                    crate::routes::service_ws::handler::dispatch_next_batch_update(
                        &state,
                        service_id,
                        batch_id,
                        row.host_id,
                    )
                    .await;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reaper_constants_are_sane() {
        assert_eq!(REAPER_INTERVAL, std::time::Duration::from_secs(60));
        assert_eq!(REAPER_GRACE, std::time::Duration::from_secs(300));
    }
}
