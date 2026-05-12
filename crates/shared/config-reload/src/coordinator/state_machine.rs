//! Full coordinator state-machine implementation.

use std::collections::BTreeMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use futures::stream::{FuturesUnordered, StreamExt};
use rootcause::Report;
use time::OffsetDateTime;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::audit::ReloadAuditEvent;
use crate::coordinator::{CoordinatorState, DegradedInfo, ReloadCoordinatorHandle, ReloadRequest};
use crate::delta::RuntimeConfigDelta;
use crate::reloadable::ReloadableErased;

/// The coordinator state machine.
///
/// Drives reload cycles for a heterogeneous set of [`ReloadableErased`]
/// subsystems. Obtained via [`ReloadCoordinator::new`]; tests use
/// [`ReloadCoordinator::new_for_test`].
pub struct ReloadCoordinator {
    state: Arc<ArcSwap<CoordinatorState>>,
    reloadables: Vec<Arc<dyn ReloadableErased>>,
    rx: mpsc::Receiver<ReloadRequest>,
    handle: ReloadCoordinatorHandle,
    audit_tx: mpsc::UnboundedSender<ReloadAuditEvent>,
}

impl ReloadCoordinator {
    /// Create a new coordinator with the given subsystems.
    ///
    /// Returns both the coordinator (which must be spawned via `run()`) and a
    /// cheaply-cloneable [`ReloadCoordinatorHandle`].
    #[must_use]
    pub fn new(
        reloadables: Vec<Arc<dyn ReloadableErased>>,
        audit_tx: mpsc::UnboundedSender<ReloadAuditEvent>,
    ) -> (Self, ReloadCoordinatorHandle) {
        let state = Arc::new(ArcSwap::new(Arc::new(CoordinatorState::Idle)));
        let (tx, rx) = mpsc::channel(64);
        let handle = ReloadCoordinatorHandle {
            state: state.clone(),
            tx,
        };
        let coord = Self {
            state,
            reloadables,
            rx,
            handle: handle.clone(),
            audit_tx,
        };
        (coord, handle)
    }

    /// Return a clone of the coordinator handle.
    #[must_use]
    pub fn handle(&self) -> ReloadCoordinatorHandle {
        self.handle.clone()
    }

    /// Return a snapshot of the current coordinator state.
    #[must_use]
    pub fn state(&self) -> CoordinatorState {
        (**self.state.load()).clone()
    }

    /// Drive the coordinator loop until the sender side of the channel is
    /// dropped.
    pub async fn run(mut self) {
        while let Some(req) = self.rx.recv().await {
            // Requests arriving while run_cycle executes wait in the 64-entry channel buffer
            // (back-pressure coalescing). No audit event for queued-but-not-yet-processed requests.
            if let CoordinatorState::Degraded(_) = **self.state.load() {
                warn!(
                    source = ?req.source,
                    "ignoring reload request while coordinator is Degraded"
                );
                if self
                    .audit_tx
                    .send(ReloadAuditEvent::Refused {
                        source: req.source,
                        reason: "coordinator is in Degraded state".into(),
                    })
                    .is_err()
                {
                    warn!("audit channel closed; Refused event dropped");
                }
                continue;
            }
            self.state.store(Arc::new(CoordinatorState::Reloading));
            // Plan 2 produces actual deltas from diffing old/new config.
            // This stub transitions back to Idle; run_cycle wired in Plan 2.
            self.state.store(Arc::new(CoordinatorState::Idle));
        }
    }

    /// Construct a coordinator for integration tests, without an audit
    /// consumer.
    ///
    /// The audit-event receiver is immediately dropped; send errors are
    /// silently ignored. This crate is `publish = false`; production code
    /// uses [`ReloadCoordinator::new`].
    #[must_use]
    pub fn new_for_test(reloadables: Vec<Arc<dyn ReloadableErased>>) -> Self {
        let (audit_tx, _rx) = mpsc::unbounded_channel();
        let state = Arc::new(ArcSwap::new(Arc::new(CoordinatorState::Idle)));
        let (tx, rx) = mpsc::channel(64);
        let handle = ReloadCoordinatorHandle {
            state: state.clone(),
            tx,
        };
        Self {
            state,
            reloadables,
            rx,
            handle,
            audit_tx,
        }
    }

    /// Run a single reload cycle directly, bypassing the request queue.
    ///
    /// Intended for integration tests only. Production code submits
    /// [`ReloadRequest`]s via [`ReloadCoordinatorHandle::enqueue`].
    pub async fn enqueue_and_drain(&self, delta: RuntimeConfigDelta) {
        // Result is intentionally ignored; tests assert via coord.state().
        let _result = self.run_cycle(vec![delta]).await;
    }

    /// Execute one full reload cycle for the given deltas.
    ///
    /// Steps:
    /// 1. Validate all deltas against all subsystems.
    /// 2. Apply in registration order; revert all on first failure.
    /// 3. Run health checks concurrently within per-subsystem rollback windows.
    /// 4. Revert all on health-check failure.
    ///
    /// # Errors
    ///
    /// Returns an error if validation, apply, or watchdog fails. In the revert
    /// path, errors from `revert()` put the coordinator into
    /// [`CoordinatorState::Degraded`].
    pub async fn run_cycle(
        &self,
        deltas: Vec<RuntimeConfigDelta>,
    ) -> Result<BTreeMap<String, u64>, Report> {
        self.validate_phase(&deltas)?;
        self.apply_and_watch_phase(deltas).await
    }

    // ── private phase helpers ────────────────────────────────────────────────

    fn validate_phase(&self, deltas: &[RuntimeConfigDelta]) -> Result<(), Report> {
        for r in &self.reloadables {
            for d in deltas {
                r.validate(d)?;
            }
        }
        Ok(())
    }

    /// Apply then run the watchdog; revert on failure.
    ///
    /// Kept as a small `async fn` to satisfy `clippy::large_futures`.
    async fn apply_and_watch_phase(
        &self,
        deltas: Vec<RuntimeConfigDelta>,
    ) -> Result<BTreeMap<String, u64>, Report> {
        let (applied, per_ms) = match self.apply_phase(&deltas).await {
            Ok(pair) => pair,
            Err((partial, e)) => {
                self.revert_phase(&partial).await;
                return Err(e);
            }
        };

        match self.watchdog_phase(&applied, per_ms).await {
            Ok(merged) => Ok(merged),
            Err((_timing, e)) => {
                self.revert_phase(&applied).await;
                Err(e)
            }
        }
    }

    /// Apply each subsystem in order; return partial applied list on error.
    async fn apply_phase(
        &self,
        deltas: &[RuntimeConfigDelta],
    ) -> Result<
        (Vec<Arc<dyn ReloadableErased>>, BTreeMap<String, u64>),
        (Vec<Arc<dyn ReloadableErased>>, Report),
    > {
        let mut applied: Vec<Arc<dyn ReloadableErased>> = Vec::new();
        let mut per_ms: BTreeMap<String, u64> = BTreeMap::new();

        for r in &self.reloadables {
            for d in deltas {
                let start = std::time::Instant::now();
                if let Err(e) = r.apply(d).await {
                    return Err((applied, e));
                }
                let took = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                // Accumulate time across multiple deltas for the same subsystem
                // rather than overwriting on each iteration.
                per_ms
                    .entry(r.name().to_string())
                    .and_modify(|v| *v += took)
                    .or_insert(took);
            }
            // Track each subsystem once regardless of how many deltas were applied.
            applied.push(r.clone());
        }
        Ok((applied, per_ms))
    }

    /// Run health checks concurrently; return accumulated timing on success.
    async fn watchdog_phase(
        &self,
        applied: &[Arc<dyn ReloadableErased>],
        mut per_ms: BTreeMap<String, u64>,
    ) -> Result<BTreeMap<String, u64>, (BTreeMap<String, u64>, Report)> {
        let mut unordered: FuturesUnordered<_> = applied
            .iter()
            .map(|r| {
                let r2 = r.clone();
                let window = r.rollback_window();
                async move {
                    let start = std::time::Instant::now();
                    let outcome = tokio::time::timeout(window, r2.health_check()).await;
                    (r2.name(), outcome, start.elapsed())
                }
            })
            .collect();

        while let Some((name, outcome, elapsed)) = unordered.next().await {
            let took = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
            per_ms.entry(name.to_string()).and_modify(|v| *v += took);

            let err = match outcome {
                Ok(Ok(())) => continue,
                Ok(Err(e)) => e,
                Err(_timeout) => rootcause::report!("watchdog timed out for {name}"),
            };
            return Err((per_ms, err));
        }

        Ok(per_ms)
    }

    /// Revert in reverse-registration order. A failing revert puts the
    /// coordinator into [`CoordinatorState::Degraded`].
    async fn revert_phase(&self, applied: &[Arc<dyn ReloadableErased>]) {
        for r in applied.iter().rev() {
            if let Err(e) = r.revert().await {
                error!(
                    subsystem = r.name(),
                    error = %e,
                    "revert failed; coordinator entering Degraded"
                );
                self.state
                    .store(Arc::new(CoordinatorState::Degraded(DegradedInfo::new(
                        OffsetDateTime::now_utc(),
                        vec![r.name().to_string()],
                        format!("revert returned Err on {}: {e}", r.name()),
                    ))));
            }
        }
    }

    /// Re-check all subsystem health; transition back to Idle if all healthy.
    ///
    /// # Errors
    ///
    /// Returns an error if any subsystem is still unhealthy; the coordinator
    /// remains in (or re-enters) [`CoordinatorState::Degraded`].
    pub async fn clear_degraded_internal(&self) -> Result<(), Report> {
        let mut still_failing: Vec<String> = Vec::new();
        let mut last_err: Option<Report> = None;

        for r in &self.reloadables {
            if let Err(e) = r.health_check().await {
                still_failing.push(r.name().to_string());
                last_err = Some(e);
            }
        }

        // `still_failing` is non-empty iff `last_err` is `Some` — both are
        // populated together in the same loop body.
        match last_err {
            None => {
                self.state.store(Arc::new(CoordinatorState::Idle));
                info!("Degraded state cleared; coordinator returning to Idle");
                Ok(())
            }
            Some(err) => {
                self.state
                    .store(Arc::new(CoordinatorState::Degraded(DegradedInfo::new(
                        OffsetDateTime::now_utc(),
                        still_failing.clone(),
                        format!(
                            "clear-degraded failed: still unhealthy: {}",
                            still_failing.join(", ")
                        ),
                    ))));
                Err(err)
            }
        }
    }
}
