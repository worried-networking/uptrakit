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
use uptrakit_web_api_queries::queries::update_reaper::StalledCandidateService;
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_wire::DEFAULT_UPDATE_TIMEOUT;

use crate::AppState;

/// How often the reaper sweeps for overdue `InProgress` updates.
const REAPER_INTERVAL: Duration = Duration::from_secs(60);

/// Margin past the update's own budget before the backstop fires, leaving room
/// for agent-side detection (keepalive / interactive timeout) to report first.
const REAPER_GRACE: Duration = Duration::from_secs(300);

/// Wall-clock age (from `created_at`) past which a `Pending` row that was
/// never started is eligible for reaping — provided every live service linked
/// to its host has been absent (per persisted `services.last_seen_at`) for
/// this entire window. Connection absence is the evidence that the dispatch
/// was never delivered.
const PENDING_DISPATCH_GRACE: Duration = Duration::from_secs(600);

/// Connection-absent evidence for reaping one `Pending` candidate.
///
/// Evidence is the persisted `services.last_seen_at` (refreshed on every
/// service ping — survives controller restarts): `true` only when every live
/// linked service was last seen at least `grace` ago (never-seen counts as
/// absent since enrollment). A candidate with no live linked services has no
/// delivery path at all and is reapable on age alone (the list query already
/// enforces the row-age bar).
fn pending_row_is_reapable(
    now: OffsetDateTime,
    services: &[StalledCandidateService],
    grace: Duration,
) -> bool {
    services
        .iter()
        .all(|s| s.last_seen_at.is_none_or(|seen| now - seen >= grace))
}

/// Spawn the detached background reaper loop.
///
/// Every [`REAPER_INTERVAL`] it reaps `InProgress` updates older than each
/// row's own budget (default `DEFAULT_UPDATE_TIMEOUT`) + `REAPER_GRACE` and,
/// for each reaped row, emits the same `AdminEvent::UpdateCompleted` the
/// completion path sends (with `status: "interrupted"`), refreshes
/// per-tenant software state, and advances any owning batch.
pub fn spawn_update_reaper(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(REAPER_INTERVAL).await;
            let now = OffsetDateTime::now_utc();
            let reaped =
                match uptrakit_web_api_queries::queries::update_reaper::reap_overdue_updates(
                    state.db(),
                    now,
                    DEFAULT_UPDATE_TIMEOUT,
                    REAPER_GRACE,
                )
                .await
                {
                    Ok(reaped) => reaped,
                    Err(e) => {
                        tracing::error!(error = %e, "update reaper sweep failed");
                        continue;
                    }
                };

            let pending_reaped = match uptrakit_web_api_queries::queries::update_reaper::list_stalled_pending_updates(
                state.db(),
                now,
                PENDING_DISPATCH_GRACE,
            )
            .await
            {
                Ok(candidates) => {
                    let mut reapable: Vec<uuid::Uuid> = Vec::new();
                    let mut suppressed = 0usize;
                    'candidates: for c in &candidates {
                        if !pending_row_is_reapable(now, &c.services, PENDING_DISPATCH_GRACE) {
                            suppressed += 1;
                            continue;
                        }
                        // A service that reconnected mid-window may have just received a
                        // replay of this row (replay leaves it `Pending`) — drop it now so
                        // the reap cannot race the replay; the CAS in
                        // reap_stalled_pending_updates covers the residual milliseconds.
                        for s in &c.services {
                            if state.service_connections.is_connected(&s.service_id).await {
                                suppressed += 1;
                                continue 'candidates;
                            }
                        }
                        reapable.push(c.row.id);
                    }
                    if suppressed > 0 {
                        tracing::warn!(
                            count = suppressed,
                            "stalled Pending updates not reaped: a linked host service is live or recently seen — \
                             agent-side deadlines (M1) own the connected-but-wedged class"
                        );
                    }
                    if reapable.is_empty() {
                        vec![]
                    } else {
                        match uptrakit_web_api_queries::queries::update_reaper::reap_stalled_pending_updates(
                            state.db(),
                            now,
                            &reapable,
                        )
                        .await
                        {
                            Ok(rows) => rows,
                            Err(e) => {
                                tracing::error!(error = %e, "pending update reap failed");
                                vec![]
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "pending update candidate listing failed");
                    vec![]
                }
            };

            if reaped.is_empty() && pending_reaped.is_empty() {
                continue;
            }

            if !reaped.is_empty() {
                tracing::warn!(
                    count = reaped.len(),
                    "reaped overdue in_progress updates as Interrupted"
                );
            }
            if !pending_reaped.is_empty() {
                tracing::warn!(
                    count = pending_reaped.len(),
                    "reaped stalled pending updates as Interrupted (never started)"
                );
            }

            // Emit one admin/SSE event per reaped row and collect affected
            // tenants for a single software-state refresh each.
            let mut tenants: std::collections::HashSet<uuid::Uuid> =
                std::collections::HashSet::new();
            for row in reaped.iter().chain(&pending_reaped) {
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

            // Reaped `Pending` rows never had an owning service, so the
            // service-keyed advance above can never fire for them — advance
            // their batch by tenant_id instead.
            for row in &pending_reaped {
                if let Some(batch_id) = row.batch_id {
                    crate::routes::service_ws::handler::dispatch_next_batch_update_for_tenant(
                        &state,
                        row.tenant_id,
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

    #[test]
    fn pending_evidence_predicate() {
        use uptrakit_web_api_queries::queries::update_reaper::StalledCandidateService;
        let now = OffsetDateTime::now_utc();
        let grace = PENDING_DISPATCH_GRACE;
        let svc = |last_seen_at: Option<OffsetDateTime>| StalledCandidateService {
            service_id: uuid::Uuid::now_v7(),
            last_seen_at,
        };

        // No live linked services -> no delivery path -> reapable on age alone.
        assert!(pending_row_is_reapable(now, &[], grace));
        // Never-seen service -> absent -> reapable.
        assert!(pending_row_is_reapable(now, &[svc(None)], grace));
        // Seen recently -> not reapable.
        assert!(!pending_row_is_reapable(
            now,
            &[svc(Some(now - Duration::from_secs(30)))],
            grace
        ));
        // Seen exactly one grace ago -> absent (>= bound) -> reapable.
        assert!(pending_row_is_reapable(
            now,
            &[svc(Some(now - grace))],
            grace
        ));
        // Mixed: one long-absent + one recent -> NOT reapable (all() semantics).
        assert!(!pending_row_is_reapable(
            now,
            &[
                svc(Some(now - (grace + Duration::from_secs(1)))),
                svc(Some(now - Duration::from_secs(30)))
            ],
            grace
        ));
    }
}
