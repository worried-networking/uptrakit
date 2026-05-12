# Graceful Reload — Plan 2: Subsystem Wiring

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement one `Reloadable` + `ReloadableErased` pair per long-lived Controller subsystem (HTTPS, PKI, DB
pool, NATS, TLS/CA snapshot, plugin registry, notification plugin registry, audit dispatcher, zeroconf advertiser,
embedded service supervisor). Refactor every `AppState` field consumed at read-time to a
`watch::Receiver<Arc<…>>`. Delete the existing `--reuseport` / `--takeover-from` / `SIGUSR1` graceful-restart path
and the 30-second `spawn_settings_reload` task. Land the full `ConfigReconciler` task that converts
`settings_version` bumps into `RuntimeConfigDelta` deliveries.

**Architecture:** Each subsystem hides its `Reloadable` impl behind its existing constructor surface. The
coordinator's registry (introduced in Plan 1) is now populated at boot. Each subsystem owns its pre-apply snapshot
internally — the coordinator only orchestrates. DB-rooted sections are reloaded by `ConfigReconciler` reading from
the DB on every `settings_version` bump and shoving an `RuntimeConfigDelta::Audit(...)` (or analogous) into the
coordinator's queue.

**Tech Stack:** Tokio 1, axum 0.8 (`with_graceful_shutdown`), SeaORM 1 / sqlx 0.8, `async-nats`,
`uptrakit-config-reload`, `arc-swap`, `parking_lot`, `rootcause::Report`, `tracing`.

**Spec:** `docs/superpowers/specs/2026-05-12-graceful-reload-design.md` (sections §10 + §13 + §20).

**Status:** Draft → Ready for review.

---

## Prerequisites

- Plan 1 merged. The coordinator, traits, triggers, and `ConfigReloadError` already compile.

## Snapshot binding

- "Use parking_lot::Mutex (never std::sync::Mutex or tokio::sync::Mutex) in async code" — every snapshot Mutex.
- "Use rootcause::Report" — every fallible Reloadable method.
- "`#[non_exhaustive]` on public extensible structs" — every new public wrapper (`DbConnHandle`,
  `HttpsListenerSpec`).
- "BEGIN IMMEDIATE for read-then-write" — `ConfigReconciler` reads `settings_version` + checks for bump (read-only,
  no transaction needed) but later DB rows must use BEGIN IMMEDIATE when bumping.
- "`Validate` trait" — Reloadable validate() is the in-process analogue.
- Workspace lints: `clippy::large_futures = "deny"` — no `join_all` over multi-second futures; use
  `FuturesUnordered` or sequential `.await`.
- "forbid `unwrap()` / `expect()` / `panic!()` in production".
- Conventional Commits: `feat(controller-runtime)`, `feat(web-api)`, `refactor(controller-runtime)`,
  `feat(plugins)`, `test(...)`.

---

## File Structure

**New files:**

- `crates/core/controller-runtime/src/reload/mod.rs` — module root exposing every subsystem's Reloadable impl.
- `crates/core/controller-runtime/src/reload/https_listener.rs`
- `crates/core/controller-runtime/src/reload/pki_listener.rs`
- `crates/core/controller-runtime/src/reload/db_pool.rs` — defines `DbConnHandle`.
- `crates/core/controller-runtime/src/reload/nats.rs`
- `crates/core/controller-runtime/src/reload/tls_snapshot.rs`
- `crates/core/controller-runtime/src/reload/audit.rs`
- `crates/core/controller-runtime/src/reload/zeroconf.rs`
- `crates/core/controller-runtime/src/reload/embedded.rs`
- `crates/ui/web-api-queries/src/reload/plugin_registry.rs` — Reloadable for the plugin registries (lives with
  queries because plugin lifecycle already lives there).
- `crates/core/controller-runtime/src/reload/reconciler.rs` — full `ConfigReconciler` task (replaces the skeleton
  helper in `uptrakit-config-reload`).
- Reloadable tests as `#[cfg(test)] mod tests` inside each file (per workspace convention).
- `tests/reload_integration.rs` (per crate) — multi-subsystem coordinator integration with a real `SqliteDatabase`
  in-memory fixture.

**Modified files:**

- `crates/ui/web-api/src/app_state.rs` — convert direct-owned fields (`audit_log_filter`,
  `audit_log_dispatcher`, `audit_emitter`, plus anything reload-affected) to
  `watch::Receiver<Arc<…>>`; update the builder.
- `crates/core/controller-runtime/src/tasks.rs` — **delete** `spawn_settings_reload` + `SETTINGS_POLL_INTERVAL`.
- `crates/core/controller-runtime/src/lib.rs` — remove `--reuseport` / `--takeover-from` plumbing.
- `crates/core/controller/src/cli.rs` (or wherever flags live) — drop `--reuseport`, `--takeover-from`,
  `SIGUSR1` handler.
- `crates/core/controller-runtime/src/startup/mod.rs` — populate the coordinator's `Vec<Arc<dyn ReloadableErased>>`
  before spawning `coordinator.run()`.
- Every call site that previously read `state.audit_log_filter` etc. — change to
  `state.audit_log_filter.borrow().clone()` (mechanical refactor; the reload subagent must walk every consumer).

---

## Task 1: `DbConnHandle` + DB-pool Reloadable

**Files:**

- Create: `crates/core/controller-runtime/src/reload/db_pool.rs`
- Modify: `crates/core/controller-runtime/src/reload/mod.rs`

- [ ] **Step 1: Write failing test**

Create `crates/core/controller-runtime/src/reload/db_pool.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const TEST_URL: &str = "sqlite::memory:";

    #[tokio::test(flavor = "current_thread")]
    async fn db_reloadable_validates_url_unchanged() {
        let pool = build_test_pool().await;
        let reloadable = DbPoolReloadable::new(pool.clone(), TEST_URL.to_string());
        let mut new = DbConfig::default();
        new.url = TEST_URL.to_string();
        new.pool_size = 32;
        new.acquire_timeout_ms = 6_000;
        reloadable.validate(&new).unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn db_reloadable_rejects_url_change() {
        let pool = build_test_pool().await;
        let reloadable = DbPoolReloadable::new(pool.clone(), TEST_URL.to_string());
        let mut new = DbConfig::default();
        new.url = "sqlite::memory:foo".to_string();
        let err = reloadable.validate(&new).unwrap_err();
        assert!(err.to_string().contains("db.url"));
    }

    async fn build_test_pool() -> sea_orm::DatabaseConnection {
        sea_orm::Database::connect(TEST_URL).await.expect("test pool")
    }
}
```

- [ ] **Step 2: Implement `DbConnHandle` + Reloadable**

```rust
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use rootcause::Report;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
use tokio::sync::watch;
use uptrakit_config_reload::config::DbConfig;
use uptrakit_config_reload::defaults::WATCHDOG_DB_POOL;
use uptrakit_config_reload::error::ConfigReloadError;
use uptrakit_config_reload::reloadable::{Reloadable, ReloadableErased};
use uptrakit_config_reload::delta::RuntimeConfigDelta;

#[non_exhaustive]
pub struct DbConnHandle {
    inner: DatabaseConnection,
}

impl DbConnHandle {
    pub fn new(inner: DatabaseConnection) -> Self { Self { inner } }
    pub fn conn(&self) -> &DatabaseConnection { &self.inner }
}

impl Drop for DbConnHandle {
    fn drop(&mut self) {
        // Detach pool close so callers don't await it. SeaORM's DatabaseConnection drop already
        // closes the pool gracefully; the detached close() is a no-op safety net.
    }
}

pub struct DbPoolReloadable {
    current_url: String,
    tx: watch::Sender<Arc<DbConnHandle>>,
    snapshot: Mutex<Option<Arc<DbConnHandle>>>,
}

impl DbPoolReloadable {
    /// `url` must be the exact URL that produced `initial`. SeaORM does not expose the
    /// post-construction URL, so the caller passes it through explicitly. Without this, the
    /// `validate()` URL-equality check (which triggers the reexec branch on change) would
    /// silently always succeed.
    pub fn new(initial: DatabaseConnection, url: String) -> Self {
        let handle = Arc::new(DbConnHandle::new(initial));
        let (tx, _rx) = watch::channel(handle.clone());
        Self {
            current_url: url,
            tx,
            snapshot: Mutex::new(None),
        }
    }

    pub fn receiver(&self) -> watch::Receiver<Arc<DbConnHandle>> { self.tx.subscribe() }
}

impl Reloadable for DbPoolReloadable {
    type Config = DbConfig;

    fn name(&self) -> &'static str { "db_pool" }

    fn validate(&self, new: &DbConfig) -> Result<(), Report> {
        if new.url != self.current_url {
            return Err(ConfigReloadError::Validate(format!(
                "db.url change requires reexec (current = {}, new = {})",
                self.current_url, new.url
            ))
            .into());
        }
        if new.pool_size == 0 {
            return Err(ConfigReloadError::Validate("db.pool_size must be >= 1".into()).into());
        }
        Ok(())
    }

    async fn apply(&self, new: Arc<DbConfig>) -> Result<(), Report> {
        let mut opt = ConnectOptions::new(new.url.clone());
        opt.max_connections(new.pool_size);
        opt.acquire_timeout(Duration::from_millis(new.acquire_timeout_ms));
        let pool = Database::connect(opt).await.map_err(|e| {
            ConfigReloadError::ApplyFailed { subsystem: "db_pool".into(), message: e.to_string() }
        })?;
        let new_handle = Arc::new(DbConnHandle::new(pool));
        let mut guard = self.snapshot.lock();
        *guard = Some(self.tx.borrow().clone());
        drop(guard);
        let _ = self.tx.send(new_handle);
        Ok(())
    }

    async fn revert(&self) -> Result<(), Report> {
        let guard = self.snapshot.lock();
        if let Some(prior) = guard.clone() {
            let _ = self.tx.send(prior);
        }
        Ok(())
    }

    async fn health_check(&self) -> Result<(), Report> {
        let handle = self.tx.borrow().clone();
        let _ = handle.conn().execute_unprepared("SELECT 1").await.map_err(|e| {
            ConfigReloadError::HealthFailed { subsystem: "db_pool".into(), message: e.to_string() }
        })?;
        Ok(())
    }

    fn rollback_window(&self) -> Duration { WATCHDOG_DB_POOL }
}

#[async_trait]
impl ReloadableErased for DbPoolReloadable {
    fn name(&self) -> &'static str { <Self as Reloadable>::name(self) }
    fn validate(&self, delta: &RuntimeConfigDelta) -> Result<(), Report> {
        if let RuntimeConfigDelta::Db(cfg) = delta {
            <Self as Reloadable>::validate(self, cfg)
        } else { Ok(()) }
    }
    async fn apply(&self, delta: &RuntimeConfigDelta) -> Result<(), Report> {
        if let RuntimeConfigDelta::Db(cfg) = delta {
            <Self as Reloadable>::apply(self, cfg.clone()).await
        } else { Ok(()) }
    }
    async fn revert(&self) -> Result<(), Report> { <Self as Reloadable>::revert(self).await }
    async fn health_check(&self) -> Result<(), Report> {
        <Self as Reloadable>::health_check(self).await
    }
    fn rollback_window(&self) -> Duration { <Self as Reloadable>::rollback_window(self) }
}
```

- [ ] **Step 3: Test + commit**

```bash
cargo test -p uptrakit-controller-runtime --lib reload::db_pool
git add crates/core/controller-runtime/src/reload/
git commit -m "feat(controller-runtime): DbConnHandle + DbPoolReloadable

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: HTTPS listener Reloadable

**Files:**

- Create: `crates/core/controller-runtime/src/reload/https_listener.rs`

- [ ] **Step 1: Sketch the supervisor + Reloadable**

```rust
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use rootcause::Report;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use uptrakit_config_reload::config::HttpsConfig;
use uptrakit_config_reload::defaults::{HTTPS_DRAIN_TIMEOUT, WATCHDOG_HTTPS};
use uptrakit_config_reload::error::ConfigReloadError;
use uptrakit_config_reload::reloadable::{Reloadable, ReloadableErased};
use uptrakit_config_reload::delta::RuntimeConfigDelta;

pub struct HttpsListenerReloadable {
    tx: watch::Sender<Arc<HttpsConfig>>,
    snapshot: Mutex<Option<Arc<HttpsConfig>>>,
    /// Cancellation token signalling the current serve task to drain.
    current_drain: Mutex<Option<CancellationToken>>,
    /// Whether a drain is currently in flight (skip pre-bind in validate when true).
    draining: Mutex<bool>,
}

impl HttpsListenerReloadable {
    pub fn new(initial: HttpsConfig, tx: watch::Sender<Arc<HttpsConfig>>) -> Self {
        Self {
            tx,
            snapshot: Mutex::new(None),
            current_drain: Mutex::new(None),
            draining: Mutex::new(false),
        }
    }
}

impl Reloadable for HttpsListenerReloadable {
    type Config = HttpsConfig;
    fn name(&self) -> &'static str { "https_listener" }

    fn validate(&self, new: &HttpsConfig) -> Result<(), Report> {
        let current = self.tx.borrow().clone();
        if new.addr == current.addr {
            return Ok(()); // No bind change; skip probe.
        }
        if *self.draining.lock() {
            // A drain is in flight; the old listener still holds the prior addr. Defer
            // pre-bind probe to apply phase; rely on apply to surface bind errors.
            return Ok(());
        }
        // Try to bind a probe; close immediately.
        let probe = std::net::TcpListener::bind(&new.addr).map_err(|e| {
            ConfigReloadError::Validate(format!("network.https.addr bind probe failed: {e}"))
        })?;
        drop(probe);
        Ok(())
    }

    async fn apply(&self, new: Arc<HttpsConfig>) -> Result<(), Report> {
        let prior = self.tx.borrow().clone();
        *self.snapshot.lock() = Some(prior.clone());
        // Caller (controller runtime) listens on this watch and re-spawns the axum serve
        // when the addr changes; we just publish the new value and toggle the drain flag.
        let _ = self.tx.send(new);
        Ok(())
    }

    async fn revert(&self) -> Result<(), Report> {
        if let Some(prior) = self.snapshot.lock().clone() {
            let _ = self.tx.send(prior);
        }
        Ok(())
    }

    async fn health_check(&self) -> Result<(), Report> {
        let cfg = self.tx.borrow().clone();
        let probe_addr = pick_probe_addr(&cfg.addr)?;
        let _stream = tokio::time::timeout(
            Duration::from_secs(1),
            tokio::net::TcpStream::connect(&probe_addr),
        )
        .await
        .map_err(|_| ConfigReloadError::HealthFailed {
            subsystem: "https_listener".into(),
            message: format!("connect to {} timed out", probe_addr),
        })?
        .map_err(|e| ConfigReloadError::HealthFailed {
            subsystem: "https_listener".into(),
            message: e.to_string(),
        })?;
        Ok(())
    }

    fn rollback_window(&self) -> Duration { WATCHDOG_HTTPS }
}

fn pick_probe_addr(bound: &str) -> Result<String, Report> {
    let sa: std::net::SocketAddr = bound.parse().map_err(|e: std::net::AddrParseError| {
        ConfigReloadError::HealthFailed {
            subsystem: "https_listener".into(),
            message: e.to_string(),
        }
    })?;
    if sa.ip().is_unspecified() {
        Ok(format!("127.0.0.1:{}", sa.port()))
    } else {
        Ok(bound.to_string())
    }
}

#[async_trait]
impl ReloadableErased for HttpsListenerReloadable {
    fn name(&self) -> &'static str { <Self as Reloadable>::name(self) }
    fn validate(&self, delta: &RuntimeConfigDelta) -> Result<(), Report> {
        if let RuntimeConfigDelta::Network(n) = delta {
            <Self as Reloadable>::validate(self, &n.https)
        } else { Ok(()) }
    }
    async fn apply(&self, delta: &RuntimeConfigDelta) -> Result<(), Report> {
        if let RuntimeConfigDelta::Network(n) = delta {
            let https = Arc::new(n.https.clone());
            <Self as Reloadable>::apply(self, https).await
        } else { Ok(()) }
    }
    async fn revert(&self) -> Result<(), Report> { <Self as Reloadable>::revert(self).await }
    async fn health_check(&self) -> Result<(), Report> {
        <Self as Reloadable>::health_check(self).await
    }
    fn rollback_window(&self) -> Duration { <Self as Reloadable>::rollback_window(self) }
}
```

- [ ] **Step 2: Wire listener supervisor**

In `controller-runtime/src/startup/mod.rs`, the existing HTTPS listener code is replaced with a task that:

1. Subscribes to `tx.subscribe()`.
2. On each `changed()`, spawns a new `axum::serve` task bound to the new addr.
3. Signals the old serve task to drain via `with_graceful_shutdown` + `HTTPS_DRAIN_TIMEOUT`.

Concrete code lives next to the existing listener bootstrap; the diff is mechanical.

- [ ] **Step 3: Test + commit**

```rust
#[tokio::test]
async fn https_reloadable_skip_pre_bind_on_same_addr() {
    let cfg = HttpsConfig {
        addr: "127.0.0.1:0".into(),
        ..Default::default()
    };
    let (tx, _rx) = watch::channel(Arc::new(cfg.clone()));
    let r = HttpsListenerReloadable::new(cfg.clone(), tx);
    r.validate(&cfg).unwrap();
}
```

```bash
cargo test -p uptrakit-controller-runtime --lib reload::https_listener
git add crates/core/controller-runtime/src/reload/https_listener.rs
git commit -m "feat(controller-runtime): HttpsListenerReloadable with drain semantics

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: PKI listener Reloadable

**Files:**

- Create: `crates/core/controller-runtime/src/reload/pki_listener.rs`

Same pattern as HTTPS but `rollback_window = WATCHDOG_PKI` and `drain_timeout = PKI_DRAIN_TIMEOUT` (5 seconds).
Probe addr uses `pick_probe_addr` from the HTTPS module; refactor that into a shared helper at
`crates/core/controller-runtime/src/reload/probe.rs`.

- [ ] **Step 1: Extract `pick_probe_addr` to shared module**
- [ ] **Step 2: Implement `PkiListenerReloadable`** — same shape, different rollback window
- [ ] **Step 3: Unit test (same as Task 2 step 3)**
- [ ] **Step 4: Commit** with `feat(controller-runtime): PkiListenerReloadable`

---

## Task 4: NATS Reloadable

**Files:**

- Create: `crates/core/controller-runtime/src/reload/nats.rs`

```rust
use std::sync::Arc;
use std::time::Duration;

use async_nats::Client;
use async_trait::async_trait;
use parking_lot::Mutex;
use rootcause::Report;
use tokio::sync::watch;
use uptrakit_config_reload::config::NatsConfig;
use uptrakit_config_reload::defaults::WATCHDOG_NATS;
use uptrakit_config_reload::error::ConfigReloadError;
use uptrakit_config_reload::reloadable::{Reloadable, ReloadableErased};
use uptrakit_config_reload::delta::RuntimeConfigDelta;

pub struct NatsReloadable {
    tx: watch::Sender<Arc<Client>>,
    snapshot: Mutex<Option<Arc<Client>>>,
    snapshot_url: Mutex<Option<String>>,
    current_url: Mutex<String>,
}

impl NatsReloadable {
    pub fn new(initial_client: Client, url: String) -> Self {
        let (tx, _rx) = watch::channel(Arc::new(initial_client));
        Self {
            tx,
            snapshot: Mutex::new(None),
            snapshot_url: Mutex::new(None),
            current_url: Mutex::new(url),
        }
    }

    pub fn receiver(&self) -> watch::Receiver<Arc<Client>> { self.tx.subscribe() }
}

impl Reloadable for NatsReloadable {
    type Config = NatsConfig;
    fn name(&self) -> &'static str { "nats" }

    fn validate(&self, new: &NatsConfig) -> Result<(), Report> {
        if new.url.is_empty() {
            return Err(ConfigReloadError::Validate("nats.url is empty".into()).into());
        }
        // URL syntactic check; full reachability probe happens at apply time.
        Ok(())
    }

    async fn apply(&self, new: Arc<NatsConfig>) -> Result<(), Report> {
        let client = async_nats::connect(&new.url).await.map_err(|e| {
            ConfigReloadError::ApplyFailed { subsystem: "nats".into(), message: e.to_string() }
        })?;
        let new_arc = Arc::new(client);
        let prior = self.tx.borrow().clone();
        *self.snapshot.lock() = Some(prior);
        *self.snapshot_url.lock() = Some(self.current_url.lock().clone());
        *self.current_url.lock() = new.url.clone();
        let _ = self.tx.send(new_arc);
        Ok(())
    }

    async fn revert(&self) -> Result<(), Report> {
        if let Some(prior) = self.snapshot.lock().clone() {
            let _ = self.tx.send(prior);
        }
        if let Some(url) = self.snapshot_url.lock().clone() {
            *self.current_url.lock() = url;
        }
        Ok(())
    }

    async fn health_check(&self) -> Result<(), Report> {
        let client = self.tx.borrow().clone();
        tokio::time::timeout(Duration::from_secs(5), client.flush())
            .await
            .map_err(|_| ConfigReloadError::HealthFailed {
                subsystem: "nats".into(),
                message: "flush timed out".into(),
            })?
            .map_err(|e| ConfigReloadError::HealthFailed {
                subsystem: "nats".into(),
                message: e.to_string(),
            })?;
        Ok(())
    }

    fn rollback_window(&self) -> Duration { WATCHDOG_NATS }
}

#[async_trait]
impl ReloadableErased for NatsReloadable {
    fn name(&self) -> &'static str { <Self as Reloadable>::name(self) }
    fn validate(&self, delta: &RuntimeConfigDelta) -> Result<(), Report> {
        if let RuntimeConfigDelta::Nats(cfg) = delta {
            <Self as Reloadable>::validate(self, cfg)
        } else { Ok(()) }
    }
    async fn apply(&self, delta: &RuntimeConfigDelta) -> Result<(), Report> {
        if let RuntimeConfigDelta::Nats(cfg) = delta {
            <Self as Reloadable>::apply(self, cfg.clone()).await
        } else { Ok(()) }
    }
    async fn revert(&self) -> Result<(), Report> { <Self as Reloadable>::revert(self).await }
    async fn health_check(&self) -> Result<(), Report> {
        <Self as Reloadable>::health_check(self).await
    }
    fn rollback_window(&self) -> Duration { <Self as Reloadable>::rollback_window(self) }
}
```

- [ ] **Step 1: Implement as above**
- [ ] **Step 2: Unit test against real `async_nats` test server** (use `testcontainers::nats` already in dev-deps)
- [ ] **Step 3: Commit** — `feat(controller-runtime): NatsReloadable with flush health check`

---

## Task 5: TLS / CA snapshot Reloadable

**Files:**

- Create: `crates/core/controller-runtime/src/reload/tls_snapshot.rs`

The existing `CaSnapshotReceiver` already publishes via `watch`. The Reloadable wraps it: validate parses cert + key
from the configured paths; apply re-loads from disk and re-publishes through the existing channel; revert
re-publishes the prior snapshot; health_check performs an in-process TLS handshake to the HTTPS listener with the
new cert.

- [ ] **Step 1: Implement Reloadable wrapper around existing `CaSnapshotReceiver`**
- [ ] **Step 2: Unit test parse-and-publish** (use `rcgen` to generate test cert + key)
- [ ] **Step 3: Commit** — `feat(controller-runtime): TlsSnapshotReloadable wrapper`

---

## Task 6: Plugin registry Reloadable

**Files:**

- Create: `crates/ui/web-api-queries/src/reload/plugin_registry.rs`

Both `PluginRegistry` (instance plugins) and `NotificationPluginRegistry` get one Reloadable each. The Reloadable
detects which plugins' configs changed (`HashMap` diff), instantiates fresh plugin instances via the existing
constructor, and atomic-swaps the registry's `Arc<dyn Plugin>` map.

- [ ] **Step 1: Define `PluginRegistryReloadable`** wrapping the existing registry handle
- [ ] **Step 2: Drop-and-recreate logic — call existing `from_config(new)` per changed config**
- [ ] **Step 3: No `Plugin::health` method exists, so `health_check` is `Ok(())` for the registry** (the spec is
      explicit on this — single-shot validate of constructor success is the only signal)
- [ ] **Step 4: Unit test with a stub plugin that increments a counter on construction**
- [ ] **Step 5: Commit** — `feat(web-api-queries): PluginRegistryReloadable`

---

## Task 7: Audit dispatcher Reloadable

**Files:**

- Create: `crates/core/controller-runtime/src/reload/audit.rs`

`AuditLogDispatcher` has an internal mpsc sender + background flush task. The Reloadable handles:

- Apply: swap the filter (`AuditFilter` is `Copy`), reconfigure retention sweep interval.
- Health_check: verify the dispatcher's mpsc `is_closed() == false`. `tokio::sync::mpsc` does **not** expose queue
  depth on stable, so we don't attempt a depth-saturation check. If saturation monitoring becomes a real need
  later, the dispatcher can wrap the sender with an `Arc<AtomicUsize>` counter incremented on send and decremented
  on receive — that work is out of scope here.

- [ ] **Step 1: Implement Reloadable + ReloadableErased**
- [ ] **Step 2: Unit test filter swap + channel-open check**
- [ ] **Step 3: Commit** — `feat(controller-runtime): AuditDispatcherReloadable`

---

## Task 8: Zeroconf Reloadable

**Files:**

- Create: `crates/core/controller-runtime/src/reload/zeroconf.rs`

The existing zeroconf advertiser already supports re-publish. Wrap it as Reloadable; `health_check` queries the
local mDNS responder (or skips on platforms without mDNS).

- [ ] **Step 1: Implement Reloadable**
- [ ] **Step 2: Unit test republish on URL/addr change**
- [ ] **Step 3: Commit** — `feat(controller-runtime): ZeroconfReloadable`

---

## Task 9: Embedded services Reloadable

**Files:**

- Create: `crates/core/controller-runtime/src/reload/embedded.rs`

In-place tunables only: per-service heartbeat interval, log level. Topology changes (enable/disable agent/agent-ssh/
mqtt/scheduler) fail validation and force the reexec branch (Plan 3).

- [ ] **Step 1: Define a snapshot of the boot-time topology**
- [ ] **Step 2: validate() rejects any change to the topology booleans**
- [ ] **Step 3: apply() pushes new heartbeat + log level into per-service control channels**
      (each embedded service exposes a `tokio::sync::watch<EmbeddedTunables>` already; if not, add one)
- [ ] **Step 4: health_check awaits a `tokio::sync::oneshot` ack from each service within 2 s**
- [ ] **Step 5: Commit** — `feat(controller-runtime): EmbeddedServicesReloadable in-place tunables`

---

## Task 10: `ConfigReconciler` full task (DB poll + delta emission)

**Files:**

- Create: `crates/core/controller-runtime/src/reload/reconciler.rs`

```rust
use std::sync::Arc;
use std::time::Duration;

use rootcause::Report;
use sea_orm::DatabaseConnection;
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{error, warn};
use uptrakit_config_reload::config::Scope;
use uptrakit_config_reload::coordinator::{ReloadRequest, ReloadSource};
use uptrakit_config_reload::defaults::RECONCILER_POLL;
use uptrakit_config_reload::reconciler::SettingsVersionCache;
use uptrakit_shared_db::entity::settings_version;
use sea_orm::EntityTrait;
use sea_orm::QueryFilter;
use sea_orm::ColumnTrait;
use time::OffsetDateTime;

pub fn spawn_config_reconciler(
    db: DatabaseConnection,
    tx: mpsc::Sender<ReloadRequest>,
    cache: SettingsVersionCache,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = interval(RECONCILER_POLL);
        loop {
            tick.tick().await;
            if let Err(e) = poll_once(&db, &tx, &cache).await {
                warn!(error = %e, "reconciler poll failed");
            }
        }
    })
}

async fn poll_once(
    db: &DatabaseConnection,
    tx: &mpsc::Sender<ReloadRequest>,
    cache: &SettingsVersionCache,
) -> Result<(), Report> {
    let rows = settings_version::Entity::find().all(db).await?;
    for row in rows {
        let scope = Scope::Tenant(row.tenant_id);
        let global_scope = Scope::Global;
        let prior_global = cache.get(global_scope).unwrap_or(0);
        let prior_tenant = cache.get(scope).unwrap_or(0);
        let new_global = u64::try_from(row.global_version).unwrap_or(0);
        let new_tenant = u64::try_from(row.version).unwrap_or(0);

        if new_global > prior_global {
            cache.update(global_scope, new_global);
            let _ = tx
                .send(ReloadRequest {
                    source: ReloadSource::DbBump {
                        scope: global_scope,
                        sections: vec!["audit".into(), "registration".into()],
                    },
                    timestamp: OffsetDateTime::now_utc(),
                })
                .await;
        }
        if new_tenant > prior_tenant {
            cache.update(scope, new_tenant);
            let _ = tx
                .send(ReloadRequest {
                    source: ReloadSource::DbBump {
                        scope,
                        sections: vec!["audit_log".into()],
                    },
                    timestamp: OffsetDateTime::now_utc(),
                })
                .await;
        }
    }
    Ok(())
}
```

- [ ] **Step 1: Write integration test using SQLite in-memory** with FK constraints on, seeded
      `settings_version` row, bump via direct UPDATE
- [ ] **Step 2: Implement as above**
- [ ] **Step 3: Test passes within 5 s** (poll interval is 2 s)
- [ ] **Step 4: Commit** — `feat(controller-runtime): ConfigReconciler task`

---

## Task 11: Remove `--reuseport` / `--takeover-from` / `SIGUSR1`

**Files:**

- Modify: `crates/core/controller-runtime/src/lib.rs` — delete the SIGUSR1 takeover handshake and the parent-drain
  logic
- Modify: `crates/core/controller/src/cli.rs` (or wherever the flags live) — drop `--reuseport`, `--takeover-from`
- Modify: `crates/core/controller-standalone/src/cli.rs` — same
- Modify: docs that previously referenced the path (move to Plan 4)

- [ ] **Step 1: grep for every variant**

```bash
rg -n 'reuseport|reuse_port|reuse-port|SO_REUSEPORT|takeover|take_over|take-over|SIGUSR1|set_reuseport' \
   --hidden -g '!.git' \
   --type rust --type toml --type md
```

Capture both snake_case (Rust API), kebab-case (CLI flag), camelCase, and the POSIX constant. Don't restrict to
`src/` — the search must cover tests, docs, and Cargo.toml metadata.

- [ ] **Step 2: Delete every match (production + test)**
- [ ] **Step 3: Run full quality gate suite**
- [ ] **Step 4: Commit** — `refactor(controller): remove --reuseport/SIGUSR1 graceful-restart path`

---

## Task 12: Remove `spawn_settings_reload` + `SETTINGS_POLL_INTERVAL`

**Files:**

- Modify: `crates/core/controller-runtime/src/tasks.rs` — delete the function + constant
- Modify: every caller — switch to the per-section `watch::Receiver<Arc<…>>` from the coordinator

- [ ] **Step 1: grep call sites of `spawn_settings_reload`**
- [ ] **Step 2: Each call site moves to its Reloadable's `receiver()` from `AppState`**
- [ ] **Step 3: Delete the function + constant**
- [ ] **Step 4: Run full quality gate suite**
- [ ] **Step 5: Commit** — `refactor(controller-runtime): replace spawn_settings_reload with ConfigReconciler`

---

## Task 13: `AppState` refactor — `watch::Receiver` everywhere

**Files:**

- Modify: `crates/ui/web-api/src/app_state.rs`
- Modify: every consumer of `state.audit_log_filter`, `state.audit_log_dispatcher`, `state.audit_emitter` etc.

For each previously direct-owned field that the new design reloads:

- Change the field type to `watch::Receiver<Arc<…>>`.
- Update the builder.
- At every call site, change `state.field` to `state.field.borrow().clone()` (`.clone()` is cheap on `Arc`).

- [ ] **Step 1: List affected fields** — at minimum: `audit_log_filter`, `audit_log_dispatcher`, `audit_emitter`,
      `service_connections` (no — already shared), `notification` (mostly via plugin registry — covered separately).
      Walk `AppState` and pick the ones with reloadable config touch points.
- [ ] **Step 2: Mechanical refactor**
- [ ] **Step 3: Each PR commit covers one field group** so reviewers can follow the call-site diff
- [ ] **Step 4: Run full quality gate suite after every commit**

---

## Task 14: Wire all Reloadables into `controller-runtime::boot_config`

**Files:**

- Modify: `crates/core/controller-runtime/src/startup/mod.rs`

Build each Reloadable from the loaded `RuntimeConfig` + the already-constructed subsystem handles; collect into
`Vec<Arc<dyn ReloadableErased>>`; pass into `ReloadCoordinator::new` (Plan 1's stub).

- [ ] **Step 1: After existing subsystem construction, instantiate each Reloadable in dependency order**
      (DB → NATS → TLS → Listeners → Audit → Zeroconf → Plugins → Embedded)
- [ ] **Step 2: Pass them to the coordinator**
- [ ] **Step 3: Spawn the full `ConfigReconciler` task**
- [ ] **Step 4: Smoke test — boot the controller against an in-memory SQLite + minimal TOML; verify it stays up**
- [ ] **Step 5: Commit** — `feat(controller-runtime): wire all Reloadables into coordinator`

---

## Task 15: Coordinator integration test with real Reloadables

**Files:**

- Create: `crates/core/controller-runtime/tests/reload_integration.rs`

End-to-end test:

1. Build a TOML in `tempdir`.
2. Boot the full controller against in-memory SQLite.
3. Use `nix::sys::signal::raise(SIGHUP)` to trigger reload.
4. Assert that all Reloadables stay healthy.
5. Mutate one `global_settings` row via SeaORM + `bump_global_settings_version`.
6. Within 5 s, assert the relevant Reloadable's `apply` was called.

- [ ] **Step 1: Write the test** (`#[tokio::test(start_paused = false)]` because SQLx)
- [ ] **Step 2: Confirm it passes**
- [ ] **Step 3: Commit** — `test(controller-runtime): coordinator end-to-end reload test`

---

## Task 16: Quality gates + PR

- [ ] **Step 1:** `cargo fmt --all -- --check`
- [ ] **Step 2:** `cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings`
- [ ] **Step 3:** `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] **Step 4:** `cargo deny check`
- [ ] **Step 5:** `cargo test --no-default-features --features db-sqlite`
- [ ] **Step 6:** `cargo test --all-features`
- [ ] **Step 7:** Open PR titled `feat(controller-runtime): wire all subsystems into config reload coordinator`
      with body referencing spec §10 and §13, plus the breaking-change note about `--reuseport`/`SIGUSR1` removal
      (full operator-facing migration in Plan 4 docs).

## Self-review

- Spec §10.1 HTTPS listener: pre-bind probe skipped on same-addr + during drain ✓
- Spec §10.2 PKI listener: 5 s rollback window + drain timeout ✓
- Spec §10.3 DB pool: `DbConnHandle` defined; reconnect model; natural Drop-based drain ✓
- Spec §10.4 NATS: `flush()` health check ✓
- Spec §10.5 TLS: re-uses existing `CaSnapshotReceiver` ✓
- Spec §10.6 plugins: drop-and-recreate; no `Plugin::health` ✓
- Spec §10.7 audit: channel-open check ✓
- Spec §10.8 zeroconf: republish on change ✓
- Spec §10.9 embedded: in-place tunables only; topology change escalates to reexec (Plan 3) ✓
- Spec §13 multi-controller: `ConfigReconciler` polls `settings_version` rows ✓
- Spec §20 removals: `--reuseport`/SIGUSR1 + `spawn_settings_reload` both deleted ✓
- Snapshot rules: every Reloadable uses `rootcause::Report`; every snapshot uses `parking_lot::Mutex`; every
  wire-exposed enum carries `Other(String)`; no `unwrap()` in production paths.
