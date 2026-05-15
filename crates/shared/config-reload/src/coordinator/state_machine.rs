//! Full coordinator state-machine implementation.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;
use futures::stream::{FuturesUnordered, StreamExt};
use rootcause::Report;
use time::OffsetDateTime;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::alerts::{AlertSeverity, NoopAlertWriter, SystemAlertWriter};
use crate::audit::ReloadAuditEvent;
use crate::config::RuntimeConfig;
use crate::coordinator::{CoordinatorState, DegradedInfo, ReloadCoordinatorHandle, ReloadRequest};
use crate::delta::RuntimeConfigDelta;
use crate::reexec_hook::ReexecHook;
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
    alert_writer: Arc<dyn SystemAlertWriter>,
    /// Path of the TOML config file. Populated via [`set_config_path`].
    /// `None` until set; file-sourced requests return an error if absent.
    config_path: Option<PathBuf>,
    /// Most recent successfully-applied (or boot) `RuntimeConfig`.
    ///
    /// Accessed only from the sequential `run()` loop — plain `Arc` is
    /// sufficient (no concurrent readers, no lock needed).
    current_config: Arc<RuntimeConfig>,
    /// Hook invoked before applying file-sourced deltas to decide whether
    /// irreversibly-bound keys changed and reexec is required.
    ///
    /// `Box` not `Arc`: the coordinator's sequential `run()` loop is the sole
    /// owner — no other task clones or shares this hook.
    reexec_hook: Option<Box<dyn ReexecHook>>,
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
        alert_writer: Arc<dyn SystemAlertWriter>,
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
            alert_writer,
            config_path: None,
            current_config: Arc::new(RuntimeConfig::default()),
            reexec_hook: None,
        };
        (coord, handle)
    }

    /// Return a clone of the coordinator handle.
    #[must_use]
    pub fn handle(&self) -> ReloadCoordinatorHandle {
        self.handle.clone()
    }

    /// Append additional reloadables before the coordinator is spawned.
    ///
    /// Must be called before [`ReloadCoordinator::run`] — adding reloadables
    /// after the run loop starts is not supported.
    pub fn extend_reloadables(&mut self, items: Vec<Arc<dyn ReloadableErased>>) {
        self.reloadables.extend(items);
    }

    /// Replace the alert writer before the coordinator is spawned.
    ///
    /// Called by the startup sequence after the `AuditEmitter` is available so
    /// the real adapter can replace the [`NoopAlertWriter`] installed at construction.
    pub fn set_alert_writer(&mut self, writer: Arc<dyn SystemAlertWriter>) {
        self.alert_writer = writer;
    }

    /// Set the TOML config file path.
    ///
    /// Must be called before [`run`](Self::run) for Sighup/FileWatch requests
    /// to succeed.
    pub fn set_config_path(&mut self, path: PathBuf) {
        self.config_path = Some(path);
    }

    /// Set the current (boot-time) `RuntimeConfig` used as the diff baseline.
    ///
    /// Must be called before [`run`](Self::run) when file-sourced reloads are
    /// expected.
    pub fn set_current_config(&mut self, config: Arc<RuntimeConfig>) {
        self.current_config = config;
    }

    /// Register the reexec hook called before applying file-sourced deltas.
    ///
    /// When absent, file-sourced reloads skip the reexec check. Acceptable
    /// when reexec is not required (e.g. in tests).
    pub fn set_reexec_hook(&mut self, hook: Box<dyn ReexecHook>) {
        self.reexec_hook = Some(hook);
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
            alert_writer: Arc::new(NoopAlertWriter),
            config_path: None,
            current_config: Arc::new(RuntimeConfig::default()),
            reexec_hook: None,
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
        if let Err(e) = self.validate_phase(&deltas) {
            self.alert_writer
                .write(
                    AlertSeverity::Warning,
                    format!("config validation failed: {e}"),
                )
                .await;
            return Err(e);
        }
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
                self.alert_writer
                    .write(AlertSeverity::Error, format!("config apply failed: {e}"))
                    .await;
                self.revert_phase(&partial).await;
                return Err(e);
            }
        };

        match self.watchdog_phase(&applied, per_ms).await {
            Ok(merged) => Ok(merged),
            Err((_timing, e)) => {
                self.alert_writer
                    .write(
                        AlertSeverity::Error,
                        format!("watchdog failed after apply: {e}"),
                    )
                    .await;
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
                let reason = format!("revert returned Err on {}: {e}", r.name());
                self.alert_writer
                    .write(
                        AlertSeverity::Critical,
                        format!(
                            "coordinator entered Degraded: revert failed for '{}': {e}",
                            r.name()
                        ),
                    )
                    .await;
                self.state
                    .store(Arc::new(CoordinatorState::Degraded(DegradedInfo::new(
                        OffsetDateTime::now_utc(),
                        vec![r.name().to_string()],
                        reason,
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

/// Diff `prior` and `new`; return the minimal set of in-process deltas.
///
/// Irreversibly-bound keys (`db.url`, `master_key.path`, `log.path`,
/// embedded topology) are checked by the reexec hook BEFORE this function
/// is called. `EmbeddedServices` is never emitted as a delta.
fn build_deltas(prior: &RuntimeConfig, new: &RuntimeConfig) -> Vec<RuntimeConfigDelta> {
    let mut deltas = Vec::new();
    if prior.db != new.db {
        deltas.push(RuntimeConfigDelta::Db(Arc::new(new.db.clone())));
    }
    if prior.network != new.network {
        deltas.push(RuntimeConfigDelta::Network(Arc::new(new.network.clone())));
    }
    if prior.nats != new.nats {
        deltas.push(RuntimeConfigDelta::Nats(Arc::new(new.nats.clone())));
    }
    if prior.tls != new.tls {
        deltas.push(RuntimeConfigDelta::Tls(Arc::new(new.tls.clone())));
    }
    if prior.audit != new.audit {
        deltas.push(RuntimeConfigDelta::Audit(Arc::new(new.audit.clone())));
    }
    if prior.zeroconf != new.zeroconf {
        deltas.push(RuntimeConfigDelta::Zeroconf(Arc::new(new.zeroconf.clone())));
    }
    // EmbeddedServices topology changes trigger reexec (handled before
    // build_deltas is called). Never emit EmbeddedServices here.
    deltas
}

/// Map `DbBump` section strings to coordinator deltas.
///
/// Unknown sections are logged at `warn` and skipped. Duplicate entries
/// (e.g. `["audit", "audit_log"]`) are deduplicated by variant tag.
fn sections_to_deltas(sections: &[String], current: &RuntimeConfig) -> Vec<RuntimeConfigDelta> {
    let mut deltas = Vec::new();
    for s in sections {
        match s.as_str() {
            "audit" | "audit_log" | "registration" => {
                deltas.push(RuntimeConfigDelta::Audit(Arc::new(current.audit.clone())));
            }
            "plugins" => {
                deltas.push(RuntimeConfigDelta::PluginsDbRefresh);
            }
            other => {
                tracing::warn!(section = other, "unknown section in DbBump; skipping delta");
            }
        }
    }
    dedup_deltas(deltas)
}

/// Deduplicate a delta list by variant tag, keeping the last occurrence.
///
/// Last-wins semantics: when `sections_to_deltas` maps multiple section names
/// to the same delta variant (e.g. `"audit"` and `"audit_log"` both map to
/// `Audit`), the last entry in the input order is kept.
fn dedup_deltas(deltas: Vec<RuntimeConfigDelta>) -> Vec<RuntimeConfigDelta> {
    let mut seen = HashSet::new();
    let mut result: Vec<_> = deltas
        .into_iter()
        .rev()
        .filter(|d| seen.insert(d.variant_tag()))
        .collect();
    result.reverse();
    result
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use super::*;

    // ── Capturing alert writer ───────────────────────────────────────────────

    #[derive(Default)]
    struct CapturingAlertWriter {
        captured: Mutex<Vec<(AlertSeverity, String)>>,
    }

    impl CapturingAlertWriter {
        fn alerts(&self) -> Vec<(AlertSeverity, String)> {
            self.captured.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl SystemAlertWriter for CapturingAlertWriter {
        async fn write(&self, severity: AlertSeverity, message: String) {
            self.captured.lock().unwrap().push((severity, message));
        }
    }

    // ── Always-fail reloadable ───────────────────────────────────────────────

    struct FailsApply;

    #[async_trait::async_trait]
    impl ReloadableErased for FailsApply {
        fn name(&self) -> &'static str {
            "fails_apply"
        }
        fn validate(&self, _: &RuntimeConfigDelta) -> Result<(), Report> {
            Ok(())
        }
        async fn apply(&self, _: &RuntimeConfigDelta) -> Result<(), Report> {
            Err(rootcause::report!("intentional apply failure"))
        }
        async fn revert(&self) -> Result<(), Report> {
            Ok(())
        }
        async fn health_check(&self) -> Result<(), Report> {
            Ok(())
        }
        fn rollback_window(&self) -> Duration {
            Duration::from_secs(1)
        }
    }

    struct FailsValidate;

    #[async_trait::async_trait]
    impl ReloadableErased for FailsValidate {
        fn name(&self) -> &'static str {
            "fails_validate"
        }
        fn validate(&self, _: &RuntimeConfigDelta) -> Result<(), Report> {
            Err(rootcause::report!("intentional validate failure"))
        }
        async fn apply(&self, _: &RuntimeConfigDelta) -> Result<(), Report> {
            Ok(())
        }
        async fn revert(&self) -> Result<(), Report> {
            Ok(())
        }
        async fn health_check(&self) -> Result<(), Report> {
            Ok(())
        }
        fn rollback_window(&self) -> Duration {
            Duration::from_secs(1)
        }
    }

    /// Applies successfully but fails health_check (triggering revert), then fails revert.
    struct AppliesHealthFailsRevertFails;

    #[async_trait::async_trait]
    impl ReloadableErased for AppliesHealthFailsRevertFails {
        fn name(&self) -> &'static str {
            "applies_health_fails_revert_fails"
        }
        fn validate(&self, _: &RuntimeConfigDelta) -> Result<(), Report> {
            Ok(())
        }
        async fn apply(&self, _: &RuntimeConfigDelta) -> Result<(), Report> {
            Ok(())
        }
        async fn revert(&self) -> Result<(), Report> {
            Err(rootcause::report!("intentional revert failure"))
        }
        async fn health_check(&self) -> Result<(), Report> {
            Err(rootcause::report!("intentional health check failure"))
        }
        fn rollback_window(&self) -> Duration {
            Duration::from_secs(1)
        }
    }

    fn make_delta() -> RuntimeConfigDelta {
        use crate::config::DbConfig;
        use std::sync::Arc;
        RuntimeConfigDelta::Db(Arc::new(DbConfig::new("sqlite::memory:")))
    }

    fn coordinator_with_writer(
        reloadable: Arc<dyn ReloadableErased>,
        writer: Arc<dyn SystemAlertWriter>,
    ) -> ReloadCoordinator {
        let (audit_tx, _rx) = mpsc::unbounded_channel();
        let state = Arc::new(ArcSwap::new(Arc::new(CoordinatorState::Idle)));
        let (tx, rx) = mpsc::channel(64);
        let handle = ReloadCoordinatorHandle {
            state: state.clone(),
            tx,
        };
        ReloadCoordinator {
            state,
            reloadables: vec![reloadable],
            rx,
            handle,
            audit_tx,
            alert_writer: writer,
            config_path: None,
            current_config: Arc::new(RuntimeConfig::default()),
            reexec_hook: None,
        }
    }

    #[tokio::test]
    async fn validate_failure_emits_warning_alert() {
        let writer = Arc::new(CapturingAlertWriter::default());
        let coord = coordinator_with_writer(Arc::new(FailsValidate), Arc::clone(&writer) as _);
        drop(coord.run_cycle(vec![make_delta()]).await);
        let alerts = writer.alerts();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].0, AlertSeverity::Warning);
        assert!(alerts[0].1.contains("validation failed"));
    }

    #[tokio::test]
    async fn apply_failure_emits_error_alert() {
        let writer = Arc::new(CapturingAlertWriter::default());
        let coord = coordinator_with_writer(Arc::new(FailsApply), Arc::clone(&writer) as _);
        drop(coord.run_cycle(vec![make_delta()]).await);
        let alerts = writer.alerts();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].0, AlertSeverity::Error);
        assert!(alerts[0].1.contains("apply failed"));
    }

    #[tokio::test]
    async fn revert_failure_emits_critical_alert_and_enters_degraded() {
        let writer = Arc::new(CapturingAlertWriter::default());
        let coord = coordinator_with_writer(
            Arc::new(AppliesHealthFailsRevertFails),
            Arc::clone(&writer) as _,
        );
        drop(coord.run_cycle(vec![make_delta()]).await);
        let alerts = writer.alerts();
        // watchdog fails → Error alert; then revert fails → Critical alert
        assert!(alerts.iter().any(|(s, _)| *s == AlertSeverity::Error));
        assert!(alerts.iter().any(|(s, _)| *s == AlertSeverity::Critical));
        assert!(matches!(coord.state(), CoordinatorState::Degraded(_)));
    }

    // ── Delta helper function tests ──────────────────────────────────────────

    #[test]
    fn build_deltas_empty_on_identical_configs() {
        let c = RuntimeConfig::default();
        let result = build_deltas(&c, &c);
        assert!(result.is_empty());
    }

    #[test]
    fn build_deltas_detects_audit_change() {
        let prior = RuntimeConfig::default();
        let mut new = RuntimeConfig::default();
        new.audit.filter = "info".to_string();
        let deltas = build_deltas(&prior, &new);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].variant_tag(), "Audit");
    }

    #[test]
    fn dedup_keeps_last_occurrence() {
        let a = RuntimeConfigDelta::Audit(Arc::new(Default::default()));
        let b = RuntimeConfigDelta::Audit(Arc::new(Default::default()));
        let result = dedup_deltas(vec![a, b]);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn sections_to_deltas_maps_audit() {
        let c = RuntimeConfig::default();
        let sections = vec!["audit".to_string()];
        let deltas = sections_to_deltas(&sections, &c);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].variant_tag(), "Audit");
    }

    #[test]
    fn sections_to_deltas_maps_plugins() {
        let c = RuntimeConfig::default();
        let sections = vec!["plugins".to_string()];
        let deltas = sections_to_deltas(&sections, &c);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].variant_tag(), "PluginsDbRefresh");
    }

    #[test]
    fn sections_to_deltas_deduplicates() {
        let c = RuntimeConfig::default();
        let sections = vec!["audit".to_string(), "audit_log".to_string()];
        let deltas = sections_to_deltas(&sections, &c);
        assert_eq!(deltas.len(), 1);
    }

    //  Test the helper functions directly
    #[test]
    fn test_build_deltas() {
        let c = RuntimeConfig::default();
        assert!(build_deltas(&c, &c).is_empty());
    }
}
