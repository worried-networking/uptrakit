# Graceful Reload Leftovers

Status: Draft for review
Author: Andrey Yantsen
Date: 2026-05-14
Parent spec: [`docs/superpowers/specs/2026-05-12-graceful-reload-design.md`](2026-05-12-graceful-reload-design.md)
ADR: references [`docs/adr/0008-graceful-reload-architecture.md`](../../adr/0008-graceful-reload-architecture.md)
Edition: Rust 2024

## 1. Goal

Three specific implementation gaps remain from the shipped graceful-reload design. This spec
covers only those gaps; the parent spec is authoritative for all architectural decisions.

| Item                            | Gap                                                                                                                                                    |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **A — Coordinator run loop**    | `ReloadCoordinator::run()` is a stub. `run_cycle()`, `triage::decide()`, `perform_reexec()` are implemented but never called.                          |
| **B — ConfigFileState**         | `_file_state_tx` in `reload_audit_bridge` is unused. The `/api/v1/instance/config-state` endpoint always shows boot-time file state.                   |
| **C — DB spawn-site migration** | `DbPoolReloadable::new()` drops the watch receiver. ~7 controller-runtime sites hold a `DatabaseConnection` clone and pin the old pool after a reload. |

## 2. Item A — Coordinator run loop + reexec wiring

### 2.1 Current state

```rust
// crates/shared/config-reload/src/coordinator/state_machine.rs
pub async fn run(mut self) {
    while let Some(req) = self.rx.recv().await {
        if let CoordinatorState::Degraded(_) = **self.state.load() { /* refuse */ }
        self.state.store(Arc::new(CoordinatorState::Reloading));
        // Plan 2 produces actual deltas from diffing old/new config.
        // This stub transitions back to Idle; run_cycle wired in Plan 2.
        self.state.store(Arc::new(CoordinatorState::Idle));
    }
}
```

`run_cycle(&self, deltas: Vec<RuntimeConfigDelta>) -> Result<BTreeMap<String, u64>, Report>`
is fully implemented (validate → apply → watchdog → revert-on-failure).

In `crates/core/controller-runtime/src/reexec/`:

- `triage::decide(prior, new) -> ReexecDecision` — implemented, dead_code. Checks `db.url`,
  `master_key.path`, `log.path`, `embedded_services` topology.
- `mod.rs`: `ReexecPlan` struct and `perform_reexec(plan, &[RawFd]) -> Result<Infallible, Report>`
  — implemented, dead_code. Both annotated `#[expect(dead_code, reason = "wired into coordinator
pre-apply hook in a future graceful-reload task")]`.

### 2.2 Architectural constraint

Plan 3 states: "only the controller-runtime owner of the coordinator knows about reexec — keep
config-reload crate ignorant of it." The `uptrakit-config-reload` crate must not import
`triage` or `perform_reexec`; those live in `controller-runtime`.

### 2.3 Design

#### 2.3.1 ReexecHook trait (in `uptrakit-config-reload`)

The canonical `ReexecHook` trait and `ReexecOutcome` enum are defined in §2.3.5. This section
describes the `ReloadCoordinator` struct additions only.

Add to `ReloadCoordinator` in `crates/shared/config-reload/src/coordinator/state_machine.rs`:

- Field: `reexec_hook: Option<Arc<dyn ReexecHook>>`
- Builder: `pub fn set_reexec_hook(&mut self, hook: Arc<dyn ReexecHook>)`

#### 2.3.2 Config diff state (in `uptrakit-config-reload`)

Add two fields to `ReloadCoordinator`:

```rust
/// Path of the TOML config file. Used to reload on Sighup/FileWatch.
config_path: Option<PathBuf>,

/// Most recent successfully-applied (or boot) RuntimeConfig.
/// Accessed only from the sequential `run()` loop — plain `Arc` is sufficient.
current_config: Arc<RuntimeConfig>,
```

Builder methods:

```rust
pub fn set_config_path(&mut self, path: PathBuf);
pub fn set_current_config(&mut self, config: Arc<RuntimeConfig>);
```

`boot_config()` in `controller-runtime` calls both after constructing the coordinator.

#### 2.3.3 Delta production helpers

Add a free function in `crates/shared/config-reload/src/coordinator/state_machine.rs`:

```rust
/// Diff `prior` and `new` and produce the minimal set of deltas for in-process
/// subsystems. Irreversibly-bound keys (db.url, master_key.path, log.path,
/// embedded topology) are not represented as deltas — they trigger reexec
/// before deltas are built.
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
    // EmbeddedServices topology changes trigger reexec (checked by the reexec hook before
    // build_deltas is reached). Never emit an EmbeddedServices delta here — if reexec was
    // needed and the hook present, it already diverged; if the hook is absent, the caller
    // (`process_request`) should warn and skip this path.
    deltas
}
```

`RuntimeConfig` must derive `PartialEq`. Each section struct must also derive `PartialEq`. Add
`#[derive(PartialEq)]` to `RuntimeConfig`, `DbConfig`, `NetworkConfig`, `NatsConfig`, `TlsConfig`,
`AuditConfig`, `ZeroconfConfig`, `EmbeddedServicesConfig` in
`crates/shared/config-reload/src/config.rs`. The `HashMap<String, toml::Value>` `_extra` field
makes `PartialEq` derivable as long as `toml::Value` implements `PartialEq` (it does).

For `DbBump` requests, map the incoming `sections` strings to deltas. Initially support
`"audit"` → `RuntimeConfigDelta::Audit` and `"audit_log"` → `RuntimeConfigDelta::Audit`.
Plugin bumps use `RuntimeConfigDelta::PluginsDbRefresh` (a new unit variant — see below).
Unknown sections are logged at `warn` and skipped (no delta).

**New `RuntimeConfigDelta` variant** (add to `crates/shared/config-reload/src/delta.rs`):

```rust
/// Signal `PluginsReloadable` to re-read plugin configuration from the DB.
/// Unlike `Plugins(Arc<PluginsConfig>)`, this variant carries no config payload —
/// the distinction is structural, eliminating the sentinel-value anti-pattern.
PluginsDbRefresh,
```

`PluginsReloadable::apply()` MUST match on `RuntimeConfigDelta::PluginsDbRefresh` and re-read
plugin config from the DB. It must NOT apply `Plugins(Arc<PluginsConfig>)` for DB-sourced
plugin changes.

```rust
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
    // Deduplicate by variant tag (keep last occurrence wins).
    dedup_deltas(deltas)
}
```

`dedup_deltas` definition (add alongside `sections_to_deltas`):

```rust
fn dedup_deltas(deltas: Vec<RuntimeConfigDelta>) -> Vec<RuntimeConfigDelta> {
    let mut seen = std::collections::HashSet::new();
    let mut result: Vec<RuntimeConfigDelta> = deltas
        .into_iter()
        .rev()
        .filter(|d| seen.insert(d.variant_tag()))
        .collect();
    result.reverse();
    result
}
```

`RuntimeConfigDelta::variant_tag()` returns a `&'static str` discriminant for each variant.
Add this method to `RuntimeConfigDelta` in `crates/shared/config-reload/src/delta.rs`.

#### 2.3.4 `run()` implementation

Replace the stub body with:

```rust
pub async fn run(mut self) {
    while let Some(req) = self.rx.recv().await {
        if let CoordinatorState::Degraded(_) = **self.state.load() {
            warn!(source = ?req.source, "ignoring reload request while Degraded");
            let _ = self.audit_tx.send(ReloadAuditEvent::Refused {
                source: req.source,
                reason: "coordinator is in Degraded state".into(),
            });
            continue;
        }

        // Clone source before moving `req` into process_request.
        let source = req.source.clone();
        self.state.store(Arc::new(CoordinatorState::Reloading));
        let _ = self.audit_tx.send(ReloadAuditEvent::Requested { source: source.clone() });

        let outcome = self.process_request(req).await;

        match outcome {
            Ok(per_ms) => {
                self.state.store(Arc::new(CoordinatorState::Idle));
                let _ = self.audit_tx.send(ReloadAuditEvent::Applied {
                    sections: per_ms.keys().cloned().collect(),
                    per_subsystem_ms: per_ms,
                    source,
                });
            }
            Err(e) => {
                // Degraded state is set inside revert_phase if revert fails.
                if !matches!(**self.state.load(), CoordinatorState::Degraded(_)) {
                    self.state.store(Arc::new(CoordinatorState::Idle));
                }
                let _ = self.audit_tx.send(ReloadAuditEvent::Failed {
                    phase: ReloadPhase::Apply,
                    subsystem: None,
                    error: e.to_string(),
                });
            }
        }
    }
}

/// Process one request: load/diff/triage for file triggers; map sections for DB bumps.
async fn process_request(
    &mut self,
    req: ReloadRequest,
) -> Result<BTreeMap<String, u64>, Report> {
    match &req.source {
        ReloadSource::Sighup | ReloadSource::FileWatch { .. } => {
            let config_path = match &self.config_path {
                Some(p) => p.clone(),
                None => {
                    return Err(rootcause::report!(
                        "coordinator has no config_path; cannot reload from file"
                    ));
                }
            };

            let loaded = TomlConfigLoader::load(&config_path)?;
            for w in &loaded.warnings {
                tracing::warn!("config reload: {w}");
            }
            let new_config = loaded.config;

            // Emit FileChanged so the audit bridge can compute and record pending_digest.
            // Digest computation (sha2) lives in controller-runtime; the coordinator emits
            // only the path — the bridge hashes the file when it receives this event.
            //
            // Note: if the reexec hook diverges below, the `Applied`/`Failed` events that
            // would clear `pending_digest` never fire. This is harmless — the new process
            // starts with fresh watch-channel state. Do not persist `ConfigFileState` across
            // reexec without revisiting this sequencing.
            let _ = self.audit_tx.send(ReloadAuditEvent::FileChanged {
                path: config_path.clone(),
            });

            let prior = Arc::clone(&self.current_config);

            // Reexec branch: check for irreversibly-bound key changes.
            // triage::decide lives in controller-runtime; we delegate via the hook.
            if let Some(hook) = &self.reexec_hook {
                match hook.check_and_trigger(&prior, &new_config) {
                    ReexecOutcome::ExecFailed(err) => return Err(err),
                    ReexecOutcome::NotNeeded => {}
                }
            }

            let deltas = build_deltas(&prior, &new_config);
            if deltas.is_empty() {
                tracing::info!("file reload: no section changes detected; no-op");
                return Ok(BTreeMap::new());
            }
            let per_ms = self.run_cycle(deltas).await?;
            // Update current config after successful apply.
            self.current_config = Arc::new(new_config);
            Ok(per_ms)
        }

        ReloadSource::DbBump { sections, .. } => {
            let current = Arc::clone(&self.current_config);
            let deltas = sections_to_deltas(sections, &current);
            if deltas.is_empty() {
                return Ok(BTreeMap::new());
            }
            self.run_cycle(deltas).await
        }

        ReloadSource::Boot | ReloadSource::Other(_) => {
            // Boot is handled at startup outside the coordinator loop;
            // Other is a forward-compat catch-all — log and skip.
            tracing::debug!(source = ?req.source, "coordinator: ignoring non-actionable source");
            Ok(BTreeMap::new())
        }
    }
}
```

**Note on `process_request`**: extracting this as a separate `async fn` satisfies
`clippy::large_futures` (the workspace lint setting). Keep each branch's body in its own
small `async fn` if the function grows.

#### 2.3.5 `ReexecHook` revised design — keeping triage in controller-runtime

The `ReexecHook` trait is kept minimal:

```rust
/// Return value from a reexec eligibility check.
///
/// `exec()` on success diverges (never returns), so this type is only ever
/// constructed on the two non-diverging paths: exec failure and no-reexec.
#[non_exhaustive]
#[must_use]
pub enum ReexecOutcome {
    /// Reexec was attempted but `exec()` failed. The process remains alive.
    /// The coordinator treats this as a reload failure and stays on the old config.
    ExecFailed(Report),
    /// No irreversibly-bound key changed; proceed with in-process reload.
    NotNeeded,
}

pub trait ReexecHook: Send + Sync {
    /// Inspect `prior` vs `new`, decide if reexec is needed, and if so perform it.
    ///
    /// On successful exec(), the function diverges and never returns. Returns
    /// `ReexecOutcome::ExecFailed(err)` if exec() fails. Returns
    /// `ReexecOutcome::NotNeeded` if no irreversibly-bound key changed.
    ///
    /// **Pre-exec cleanup**: any log flushing or pre-shutdown work must be done
    /// synchronously inside this method before calling `perform_reexec`, because
    /// the Tokio runtime will not get a chance to run after exec() replaces the
    /// process image. If the controller binary uses a non-blocking `tracing-appender`
    /// writer, its background thread will be killed without flushing its buffer.
    /// Prefer a synchronous tracing writer for the controller binary, or call the
    /// dispatcher's flush method if available before `perform_reexec`.
    #[must_use]
    fn check_and_trigger(
        &self,
        prior: &RuntimeConfig,
        new: &RuntimeConfig,
    ) -> ReexecOutcome;
}
```

Implementation in `controller-runtime` (captures listener FDs and plan args at startup):

```rust
struct ControllerReexecHook {
    /// Resolved at startup via `std::env::current_exe()` before this hook is constructed.
    current_exe: PathBuf,
    config_path: PathBuf,
    master_key_file: Option<String>,
    generation: u64,
    // Populated after server bind, before coordinator.run() is spawned.
    listener_fds: Vec<RawFd>,
}

impl ReexecHook for ControllerReexecHook {
    fn check_and_trigger(&self, prior: &RuntimeConfig, new: &RuntimeConfig) -> ReexecOutcome {
        let decision = reexec::triage::decide(prior, new);
        if !decision.needed {
            return ReexecOutcome::NotNeeded;
        }
        tracing::info!(reasons = ?decision.reasons, "reexec required");

        let plan = ReexecPlan {
            current_exe: self.current_exe.clone(),
            config_path: self.config_path.clone(),
            master_key_file: self.master_key_file.clone(),
            listener_count: self.listener_fds.len(),
            generation: self.generation,
        };

        match reexec::perform_reexec(&plan, &self.listener_fds) {
            Ok(infallible) => match infallible {},
            Err(e) => ReexecOutcome::ExecFailed(e),
        }
    }
}
```

Wire in `run_server()` after the HTTPS and PKI servers bind and before
`tokio::spawn(b.coordinator.run())`. Resolve `current_exe` here so errors propagate via
`run_server`'s `Result` return rather than panicking inside the hook:

```rust
let current_exe = std::env::current_exe()
    .map_err(|e| rootcause::report!("resolve current_exe: {e}"))?;
let listener_fds = collect_listener_fds(&server_handle, &pki_handle);
b.coordinator.set_config_path(config_path.clone());
b.coordinator.set_current_config(Arc::clone(&booted.runtime_arc));
b.coordinator.set_reexec_hook(Arc::new(ControllerReexecHook {
    current_exe,
    config_path: config_path.clone(),
    master_key_file: args.master_key_from.clone(),
    generation: reexec::listenfd::current_generation(),
    listener_fds,
}));
```

`boot_config()` stores `Arc<RuntimeConfig>` in `BootedConfig` (`booted.runtime_arc`). The
coordinator seeds `current_config: Arc::clone(&booted.runtime_arc)` via `set_current_config`.

#### 2.3.6 `ReloadAuditEvent` additions

Add `source: ReloadSource` to `Applied` and a new `FileChanged` variant:

```rust
#[non_exhaustive]
pub enum ReloadAuditEvent {
    Refused { source: ReloadSource, reason: String },
    Requested { source: ReloadSource },
    /// New variant: fired when coordinator loads a new TOML before applying it.
    /// The bridge computes the SHA-256 digest from `path`; the coordinator does not hash.
    FileChanged { path: PathBuf },
    Applied {
        sections: Vec<String>,
        per_subsystem_ms: BTreeMap<String, u64>,
        /// Added: lets the bridge know whether to re-read the file for digest update.
        source: ReloadSource,
    },
    Failed { phase: ReloadPhase, subsystem: Option<String>, error: String },
    Reverted { subsystem: String, reason: String },
}
```

`Applied` gains `source`. Update all match arms in `reload_audit_bridge` and any test code.
`#[non_exhaustive]` ensures forward compatibility.

#### 2.3.7 Digest strategy

`sha2` is not a dependency of `uptrakit-config-reload`. The coordinator emits
`FileChanged { path }` (no digest field). The `reload_audit_bridge` in `controller-runtime`
computes the SHA-256 digest from `path` when it receives the event (`sha2` is already in
`controller-runtime/Cargo.toml`). This keeps `uptrakit-config-reload` free of a hashing dep.

---

## 3. Item B — ConfigFileState live updates

### 3.1 Current state

`reload_audit_bridge` receives `_file_state_tx` (underscore-prefixed, unused). The watch
channel is seeded once at boot with a size-based digest stub and never updated.

### 3.2 Design

#### 3.2.1 Updated `reload_audit_bridge` signature

```rust
async fn reload_audit_bridge(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<uptrakit_config_reload::ReloadAuditEvent>,
    emitter: uptrakit_audit_log::AuditEmitter,
    file_state_tx: tokio::sync::watch::Sender<uptrakit_config_reload::ConfigFileState>,  // was _file_state_tx
    last_reload_tx: tokio::sync::watch::Sender<Option<uptrakit_config_reload::LastReloadInfo>>,
    recent_events_tx: tokio::sync::watch::Sender<Vec<serde_json::Value>>,
    config_path: std::path::PathBuf,  // NEW
)
```

Pass `config_path` from `run_server()` (the value comes from `booted` / `boot_config`).

#### 3.2.2 Digest computation helper (in controller-runtime)

```rust
/// Compute a SHA-256 hex digest of the file at `path`.
/// Returns a `"sha256:<hex>"` string on success, or `"size:<N>"` fallback on I/O error.
fn file_digest(path: &std::path::Path) -> String {
    use sha2::{Digest as _, Sha256};
    match std::fs::read(path) {
        Ok(bytes) => {
            let mut h = Sha256::new();
            h.update(&bytes);
            format!("sha256:{:x}", h.finalize())
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "could not read config for digest; using size stub");
            format!("size:{}", path.metadata().map(|m| m.len()).unwrap_or(0))
        }
    }
}
```

Also upgrade `boot_config()` in `startup/mod.rs` to use SHA-256 (import `sha2` — it is already
a direct dep of `controller-runtime`). Replace:

```rust
let digest = format!("size:{}", file_bytes.len());
```

with:

```rust
let digest = file_digest(&config_path);
```

#### 3.2.3 Bridge event handling

```rust
match &event {
    ReloadAuditEvent::FileChanged { path } => {
        // Coordinator loaded a new TOML; mark it as pending while apply runs.
        let pending_digest = file_digest(path);
        file_state_tx.send_modify(|s| {
            s.pending_digest = Some(pending_digest);
            s.pending_detected_at = Some(time::OffsetDateTime::now_utc());
        });
    }

    ReloadAuditEvent::Applied { source, .. } => {
        // ...existing last_reload_tx and recent_events_tx updates...
        let info = uptrakit_config_reload::LastReloadInfo::new(
            time::OffsetDateTime::now_utc(),
            sections.clone(),
            per_subsystem_ms.clone(),
        );
        drop(last_reload_tx.send(Some(info)));

        // Update file state when a file-sourced reload succeeds.
        match source {
            uptrakit_config_reload::ReloadSource::Sighup
            | uptrakit_config_reload::ReloadSource::FileWatch { .. } => {
                let new_digest = file_digest(&config_path);
                file_state_tx.send_modify(|s| {
                    s.digest = new_digest;
                    s.loaded_at = time::OffsetDateTime::now_utc();
                    s.pending_digest = None;
                    s.pending_detected_at = None;
                });
            }
            _ => {}
        }

        // ...existing recent_events_tx update...
    }

    ReloadAuditEvent::Failed { .. } => {
        // Clear pending on failure (the file did not apply).
        file_state_tx.send_modify(|s| {
            s.pending_digest = None;
            s.pending_detected_at = None;
        });
        // ...existing recent_events_tx update...
    }

    _ => {}
}
```

#### 3.2.4 `ConfigFileState` mutability

`ConfigFileState` has `pub` fields already. `send_modify` (which takes `&mut ConfigFileState`)
requires fields to be directly assignable. No structural change needed.

---

## 4. Item C — DB pool spawn-site migration

### 4.1 Current state

```rust
// reload/db_pool.rs
pub(crate) fn new(initial: DatabaseConnection, url: String) -> Self {
    let handle = Arc::new(DbConnHandle::new(initial));
    let (tx, _rx) = watch::channel(handle);   // receiver dropped immediately
    Self { current_url: url, tx, snapshot: Mutex::new(None) }
}
```

`tx.send(new_handle)` in `apply()` has no receivers; the new pool is published to nobody.

### 4.2 Design

#### 4.2.1 Expose a subscriber

Add to `DbPoolReloadable`:

```rust
/// Return a new receiver for the current (and future) pool handle.
/// The receiver yields the latest `Arc<DbConnHandle>` atomically.
pub(crate) fn subscribe(&self) -> watch::Receiver<Arc<DbConnHandle>> {
    self.tx.subscribe()
}
```

#### 4.2.2 Propagate the receiver through startup

In `run_server()`, extract the receiver before moving `db_reloadable` into the
`reloadables` vec:

```rust
let db_reloadable = reload::db_pool::DbPoolReloadable::new(db_conn.clone(), db_url.clone());
let db_rx = db_reloadable.subscribe();   // NEW: grab receiver before moving
// ... other reloadables ...
reloadables.push(Arc::new(db_reloadable));
```

#### 4.2.3 AppState migration

Change `AppState` to hold the watch receiver:

```rust
// web-api/src/app_state.rs
pub db: tokio::sync::watch::Receiver<Arc<uptrakit_config_reload::DbConnHandle>>,
```

`AppState::db()` becomes:

```rust
pub fn db(&self) -> Arc<uptrakit_config_reload::DbConnHandle> {
    self.db.borrow().clone()
}
```

Route handlers and background tasks that currently do `state.db()` receive an
`Arc<DbConnHandle>`; they call `.conn()` to get `&DatabaseConnection`. The call signature at
use sites changes from `state.db()` (returned `&DatabaseConnection`) to
`state.db().conn()` (returned `&DatabaseConnection`). This is a mechanical refactor across
`web-api` routes.

The `AppState` builder method changes from:

```rust
pub fn db(mut self, conn: DatabaseConnection) -> Self
```

to:

```rust
pub fn db(mut self, rx: tokio::sync::watch::Receiver<Arc<DbConnHandle>>) -> Self
```

In `run_server()` replace `.db(db_conn.clone())` with `.db(db_rx.clone())`.

#### 4.2.4 The seven spawn sites

For each site the migration strategy is either **re-read** (clone `Arc<DbConnHandle>` per
iteration from the watch receiver) or **accept boot-time handle** (use the initial
`Arc<DbConnHandle>` from `db_rx.borrow()` — acceptable for components where pool-size
changes take effect on next cold start, not live; or where the component itself short-circuits
to a fresh borrow from AppState.db() on every request).

| Site                                                      | File                       | Strategy                                                                                                                                    |
| --------------------------------------------------------- | -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `AppState.db`                                             | `web-api/src/app_state.rs` | watch receiver (§4.2.3)                                                                                                                     |
| `spawn_config_reconciler(db_conn.clone(), …)`             | `lib.rs:722`               | re-read: pass `db_rx.clone()` to reconciler; each `poll_once` call does `let db = db_rx.borrow().clone(); db.conn()`                        |
| `DatabaseBackend::new(db_conn.clone())`                   | `lib.rs:1068`              | initial handle: `DatabaseBackend::new(db_rx.borrow().conn().clone())` — audit backend is essentially stateless per query; acceptable for V1 |
| `DbActorEnricher::new(db_conn.clone())`                   | `lib.rs:1075`              | initial handle: same rationale as audit backend                                                                                             |
| `NotificationDispatcher::new(db_conn.clone(), …)`         | `lib.rs:553`               | initial handle: notification dispatch uses short-lived queries; acceptable for V1                                                           |
| `SurfaceProxy` local executor `Arc::new(db_conn.clone())` | `lib.rs:599`               | initial handle: acceptable for V1                                                                                                           |
| `GlobalProviders::new(db_conn.clone())`                   | `lib.rs:483`               | initial handle: acceptable for V1                                                                                                           |

**V1 stance**: The most important migration is AppState and the reconciler. All other sites get
the initial `Arc<DbConnHandle>` from `db_rx.borrow()` at construction. They continue to use
that handle until the process restarts (which happens anyway on a `db.url` change via reexec).
Pool-size-only changes (no reexec) will not propagate to these components.

**Operational note**: The primary reason to change `db.pool_size` in a running system is
connection exhaustion. If an operator changes only `db.pool_size` (no URL change, no reexec),
five of seven components remain on the old pool. The `Applied` audit event shows success, but
the exhaustion symptom continues for those components.

To prevent silent-fix confusion, emit a warning **after** `run_cycle` succeeds and **only** when
the URL is unchanged (i.e., pool-size-only change — not a reexec path). Place this in the
`Ok(per_ms)` arm of `run()`, inside `process_request`'s Sighup/FileWatch branch, after the
successful `run_cycle` call:

```rust
if prior.db.pool_size != new_config.db.pool_size && prior.db.url == new_config.db.url {
    tracing::warn!(
        "db.pool_size changed in-process; components using initial-handle pattern \
         require a restart to pick up the new pool size"
    );
}
```

A follow-up task can migrate each remaining site to the watch receiver when operational telemetry
shows a need.

#### 4.2.5 `spawn_config_reconciler` migration

Change signature:

```rust
pub(crate) fn spawn_config_reconciler(
    db_rx: watch::Receiver<Arc<DbConnHandle>>,  // was: DatabaseConnection
    tx: mpsc::Sender<ReloadRequest>,
    cache: SettingsVersionCache,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()>
```

Inside `poll_once`:

```rust
async fn poll_once(
    db_rx: &watch::Receiver<Arc<DbConnHandle>>,
    tx: &mpsc::Sender<ReloadRequest>,
    cache: &SettingsVersionCache,
) -> Result<(), Report> {
    let db = db_rx.borrow().clone();
    let rows = settings_version::Entity::find()
        .all(db.conn())
        .await
        // ...
}
```

The reconciler now always picks up the current pool on each tick (re-read pattern).

#### 4.2.6 Background tasks in tasks.rs

Lines 285 and 360 do:

```rust
let db = app_state.db().clone();
```

After the AppState migration (§4.2.3), `app_state.db()` returns `Arc<DbConnHandle>`. `.clone()`
is a cheap Arc clone of the current handle. These lines are correct for short-lived use (they
get the handle at task-start time, which is already an improvement over old
`DatabaseConnection::clone()` — the handle's pool will drain correctly).

For background tasks that run in a long loop, the idiomatic pattern is to re-borrow per
iteration. The coding standard (§5 below) documents both patterns. The two existing sites in
`tasks.rs` are inside short-lived futures (CA reload, CA rotation); no change needed at these
call sites beyond what `app_state.db()` now returns.

### 4.3 `DbConnHandle` visibility

`DbConnHandle` is currently `pub(crate)` in `controller-runtime`. The `AppState` type lives in
`web-api`. For `AppState` to hold `watch::Receiver<Arc<DbConnHandle>>`:

- Move `DbConnHandle` to `uptrakit-config-reload` (the shared crate that both `controller-runtime`
  and `web-api` depend on), making it `pub`. It is already the right home conceptually — it is
  the reload-aware DB handle.
- Update `DbPoolReloadable` to import it from the new location.
- `web-api/Cargo.toml` must already depend on `uptrakit-config-reload` (or add it).

`DbConnHandle` in `uptrakit-config-reload/src/`:

```rust
// crates/shared/config-reload/src/db_handle.rs (new file)
use sea_orm::DatabaseConnection;

/// Reload-aware database connection wrapper.
///
/// Distributed to consumers via a `tokio::sync::watch` channel so that
/// in-flight requests finish against the old pool while new requests pick
/// up a replacement pool atomically.
#[non_exhaustive]
pub struct DbConnHandle {
    inner: DatabaseConnection,
}

impl DbConnHandle {
    /// Wrap a raw connection.
    #[must_use]
    pub fn new(inner: DatabaseConnection) -> Self {
        Self { inner }
    }

    /// Borrow the underlying connection.
    pub fn conn(&self) -> &DatabaseConnection {
        &self.inner
    }
}
```

Add `sea-orm` to `uptrakit-config-reload/Cargo.toml` using the workspace entry to avoid a
second resolution (the project enforces `multiple-versions = "deny"` in `deny.toml`):

```toml
[dependencies]
sea-orm = { workspace = true, default-features = false }
```

Include only the features needed (no extras required for `DbConnHandle` — just the type).
The workspace already pins `sea-orm` at a specific version; reusing `{ workspace = true }`
satisfies `cargo deny check` without any exception entry.

**Coupling trade-off**: adding `sea-orm` to `uptrakit-config-reload` makes it a database-aware
crate. Any future crate that depends on `uptrakit-config-reload` for reload signaling only will
inherit the `sea-orm` dep. If this boundary matters, create a new thin shared crate
(e.g. `uptrakit-db-handle`) containing only `DbConnHandle` and depending on `sea-orm`. Both
`controller-runtime` and `web-api` depend on it; `uptrakit-config-reload` does not.

**Decision for V1**: use the direct dep (`sea-orm = { workspace = true }` in `uptrakit-config-reload`)
since the crate is `publish = false` and the boundary cost is low. Document the alternative in
the implementation commit message if the team later prefers the thin-crate split.

---

## 5. Documentation deliverables

Implementation is not complete until all items below are addressed.

- `docs/development/coding-standards.md` — new subsection "Database pool migration patterns":
  - **Re-read pattern**: long-lived tasks hold `watch::Receiver<Arc<DbConnHandle>>`; at the
    top of each loop iteration call `let db = rx.borrow().clone()` to get the current pool.
    Old pool drains when the last `Arc` from the previous iteration drops.
  - **Initial-handle pattern**: short-lived components accept `Arc<DbConnHandle>` at
    construction (from `db_rx.borrow().clone()`). Acceptable when: the component does not
    long-outlive a single request, or when pool-size live-updates are not a correctness
    requirement for that component. Document the V1 scope.
  - **Rule**: no `DatabaseConnection::clone()` captured inside a `move` closure outside a
    per-iteration scope. `DbConnHandle::clone()` (an `Arc` clone) is acceptable inside a
    per-iteration scope.

- `docs/development/coding-standards.md` — update existing "DB pool" references to point
  to `DbConnHandle` instead of `DatabaseConnection` where describing the reload-aware path.

- OpenAPI client / schema: no changes (no new HTTP surface).

- No new ADR (this implements ADR-0008's remaining scope).

- No CONTEXT.md changes (no new domain terms; `ConfigReconciler`, `Reloadable` etc. were
  added in the parent spec).

---

## 6. Test strategy

### 6.1 Coordinator unit tests (add to `crates/shared/config-reload/tests/coordinator.rs`)

- `run_loop_calls_run_cycle_on_sighup` — mock `TomlConfigLoader`, mock a single Reloadable,
  enqueue a Sighup request, assert `apply()` was called and `Applied` audit event emitted.
- `run_loop_emits_requested_before_apply` — assert `Requested` event precedes `Applied`.
- `run_loop_noop_on_empty_deltas` — two identical TOML files → no deltas → `Applied` with
  empty sections, no `apply()` call.
- `run_loop_calls_reexec_hook_on_url_change` — set up a mock `ReexecHook` that returns
  `NotNeeded` or `ExecFailed(err)`; assert coordinator handles both correctly.
- `run_loop_db_bump_maps_sections_to_deltas` — send `DbBump { sections: ["audit"] }`, assert
  `RuntimeConfigDelta::Audit` passed to `run_cycle`.

### 6.2 ConfigFileState bridge tests

- `bridge_updates_file_state_on_applied` — send `Applied` with `Sighup` source, assert
  `file_state_rx` shows updated digest and cleared `pending_*`.
- `bridge_sets_pending_on_file_changed` — send `FileChanged`, assert `pending_digest` populated.
- `bridge_clears_pending_on_failure` — send `Failed`, assert `pending_digest` cleared.

### 6.3 DB pool migration tests

- `db_pool_subscribe_returns_active_receiver` — call `subscribe()`, apply a new config,
  assert the receiver yields the new handle via `assert!(!Arc::ptr_eq(&old_handle, &new_handle))`.
- `db_pool_multiple_subscribers` — two receivers both see the new handle after apply.
- Integration test (extend existing coordinator integration test): change `db.pool_size` in
  TOML, send Sighup, assert old pool's `Arc::strong_count` drops to zero within 5 s.

All tests follow the project conventions: `parking_lot` for locks, `#[tokio::test(start_paused =
true)]` only for time-dependent branches, no mocking of upstream crate internals.

---

## 7. Quality gates

After every commit:

```sh
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
```

---

## 8. Out of scope

- Full migration of all audit-backend and notification-dispatcher sites to the watch receiver
  (V1 uses initial-handle pattern; tracked as follow-up).
- Postgres `LISTEN/NOTIFY` for reconciler (explicit future work in parent spec §21).
- Fork-then-exec for accepted-connection preservation during reexec (parent spec §21).
- Dashboard UI for reload coordinator state (requires separate spec).
