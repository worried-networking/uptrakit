# Graceful Reload — Plan 1: Foundation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the new `uptrakit-config-reload` crate — `RuntimeConfig` + section structs, TOML loader,
`Reloadable` / `ReloadableErased` traits, the single-task `ReloadCoordinator` (with Idle / Reloading / Degraded
states), the file-watch + SIGHUP triggers, and the `ConfigReconciler` skeleton that replaces the existing 30-second
settings poll. No subsystem `Reloadable` impls are wired up yet — those land in Plan 2. The foundation must compile,
pass clippy, and ship a working `--check-config` CLI.

**Architecture:** A single new crate at `crates/shared/config-reload/` exposes typed config sections, the
reload trait pair, and the coordinator. The coordinator is owned by `controller-runtime` and constructed at startup
with an empty `Vec<Box<dyn ReloadableErased>>`. Per-section `tokio::sync::watch<Arc<SectionConfig>>` channels are
created at boot and exposed through `AppState`. The reconciler runs as its own Tokio task and pushes
`ReloadRequest::DbBump { … }` into the coordinator's `mpsc::Receiver` whenever it observes a `settings_version` bump.
Tests cover phase ordering, atomic revert-all, and Degraded-state entry.

**Tech Stack:** Rust 2024, Tokio 1, `serde` + `toml` 0.8, `notify-debouncer-full`, `async-trait` 0.1 (workspace dep),
`futures::stream::FuturesUnordered`, `arc_swap::ArcSwap`, `parking_lot::Mutex`, `rootcause::Report`, `tracing`,
SeaORM 1, `time::OffsetDateTime`, `uuid`.

**Spec:** `docs/superpowers/specs/2026-05-12-graceful-reload-design.md` (sections §4, §5, §6, §7, §8, §9, §13).

**Status:** Draft → Ready for review.

---

## Prerequisites

None. This plan is the prerequisite for Plans 2, 3, and 4.

## Snapshot binding

Tasks in this plan exercise the following Binding Rules and Tooling Constraints from
`.superpowers/standards-snapshot.md`:

- "Use rootcause::Report + thiserror::Error for all error types at module boundaries"
- "Use parking_lot::Mutex (never std::sync::Mutex or tokio::sync::Mutex) in async code"
- "#[non_exhaustive] on all extensible public enums and structs"
- "Wire-safe enums must have Other(String) catch-all" — `ReloadSource`, `ReloadPhase`
- "Typed enums for internal write-path discriminators" — `ReloadPhase` variants, `RuntimeConfigDelta`
- "Use `#[expect(lint, reason = "...")]` instead of `#[allow(...)]`" — when suppressing any lint
- "All HTTP request types implement `Validate` trait" — `RuntimeConfig::validate()` etc.
- "Forbid `unwrap()` / `expect()` / `panic!()` in production"
- "Workspace lints: warnings = deny, `unreachable_pub` = deny, `unfulfilled_lint_expectations` = deny,
  `clippy::large_futures` = deny" → coordinator must avoid `join_all`-style large futures
- "cargo fmt --all", "cargo clippy --all-targets --all-features", "cargo deny check", "cargo test --all-features",
  "cargo test --no-default-features --features db-sqlite" gate every PR
- Conventional Commits: `feat(config-reload)`, `feat(controller-runtime)`, `test(...)`, scoped + small
- "Use `#[tokio::test(start_paused = true)]` for time-dependent tests"
- "BEGIN IMMEDIATE for read-then-write transactions" — `ConfigReconciler` DB reads

---

## File Structure

**New crate** at `crates/shared/config-reload/`:

- `Cargo.toml` — workspace member declaring deps on `serde`, `toml`, `tokio`, `async-trait`, `arc-swap`,
  `parking_lot`, `rootcause`, `futures`, `notify-debouncer-full`, `time`, `tracing`, `thiserror`,
  `uptrakit-shared-macros` (workspace).
- `src/lib.rs` — crate root, public re-exports, module list.
- `src/config/mod.rs` — `RuntimeConfig` struct + cross-section `Validate` impl + `warn_about_extras()`.
- `src/config/db.rs` — `DbConfig` section.
- `src/config/network.rs` — `NetworkConfig`, `HttpsConfig`, `PkiConfig`.
- `src/config/nats.rs` — `NatsConfig` section.
- `src/config/tls.rs` — `TlsConfig` section.
- `src/config/audit.rs` — `AuditConfig` section.
- `src/config/log.rs` — `LogConfig` section.
- `src/config/master_key.rs` — `MasterKeyConfig` section.
- `src/config/embedded.rs` — `EmbeddedServicesConfig` section.
- `src/config/zeroconf.rs` — `ZeroconfConfig` section.
- `src/config/scope.rs` — `Scope` enum (`Global` | `Tenant(Uuid)`); helpers.
- `src/loader.rs` — `TomlConfigLoader::load(path)`, `load_and_validate(path)`, `validate_only(path)`.
- `src/delta.rs` — `RuntimeConfigDelta` enum (in-process only).
- `src/reloadable/mod.rs` — `Reloadable` (typed) + `ReloadableErased` (dyn-compat via `#[async_trait]`).
- `src/coordinator/mod.rs` — `ReloadCoordinator`, `ReloadRequest`, `ReloadSource`, `ReloadPhase`,
  `CoordinatorState`, `DegradedInfo`.
- `src/coordinator/state_machine.rs` — split-out helpers (`run_validate_phase`, `run_apply_phase`,
  `run_watchdog_phase`, `run_revert_phase`) to keep each future small (per `clippy::large_futures` deny).
- `src/triggers/sighup.rs` — SIGHUP handler task.
- `src/triggers/file_watch.rs` — `notify_debouncer_full::Debouncer` wrapper.
- `src/reconciler.rs` — `ConfigReconciler` task; `arc_swap::ArcSwap<HashMap<Scope, u64>>` cache.
- `src/defaults.rs` — `WATCHDOG_*`, `*_DRAIN_TIMEOUT`, `FILE_WATCH_DEBOUNCE`, `RECONCILER_POLL` constants.
- `src/error.rs` — `ConfigReloadError` thiserror enum + `impl_report_conversion!`.
- `src/audit.rs` — typed `ReloadAuditEvent` enum (the audit-log variants spec §15.1 will be glued to
  `uptrakit_audit_log::AuditEvent` in Plan 3; this file holds the structured payloads).
- `tests/coordinator.rs` — coordinator integration tests with mock Reloadables.
- `tests/loader.rs` — TOML loader integration tests with `tempfile::TempDir`.
- `tests/triggers.rs` — SIGHUP + file-watch coalescing tests.

**Modified crates / files**:

- `Cargo.toml` (workspace root) — add `notify-debouncer-full`, `listenfd`, `sd-notify`, `toml` to
  `[workspace.dependencies]`. (`listenfd` and `sd-notify` belong to Plan 3 but their lines land here for license
  review; the crate is added to deps in Plan 3.)
- `Cargo.toml` (workspace root, `members`) — add `crates/shared/config-reload`.
- `deny.toml` — confirm `notify-debouncer-full`, `toml`, `notify`, `arc-swap` licenses pass the allowlist.
- `crates/core/controller/src/main.rs` — add `--config <path>` flag; route `--check-config` to
  `TomlConfigLoader::validate_only` and exit 0/1.
- `crates/core/controller-standalone/src/main.rs` — same `--config` / `--check-config` wiring.
- `crates/core/controller-runtime/src/startup/mod.rs` — load TOML at boot, construct per-section
  `watch::Sender`/`Receiver` pairs seeded with the TOML values, build `ReloadCoordinator` with empty Reloadable
  list, spawn `ConfigReconciler`, spawn SIGHUP + file-watch tasks. Wire receivers into `AppStateBuilder`.
- `crates/core/controller-runtime/src/tasks.rs` — **untouched in Plan 1.** `spawn_settings_reload` +
  `SETTINGS_POLL_INTERVAL` deletion ships in Plan 2 alongside the consumer migration to per-section receivers,
  so Plan 1's PR cannot break a running controller mid-rollout.
- `crates/ui/web-api/src/app_state.rs` — add per-section `watch::Receiver<Arc<…>>` fields; add
  `coordinator_handle: ReloadCoordinatorHandle` for state introspection (used by Plan 3 endpoint).

---

## Task 1: Workspace deps + new crate scaffold

**Files:**

- Modify: `Cargo.toml` (workspace root)
- Modify: `deny.toml`
- Create: `crates/shared/config-reload/Cargo.toml`
- Create: `crates/shared/config-reload/src/lib.rs`

- [ ] **Step 1: Add deps to `[workspace.dependencies]`**

Open `Cargo.toml` (workspace root). Find the `[workspace.dependencies]` table. Add (alphabetically):

```toml
notify-debouncer-full = "0.5"
toml = { version = "0.8", default-features = false, features = ["parse"] }
```

Confirm `arc-swap`, `async-trait`, `futures`, `parking_lot`, `rootcause`, `serde`, `tokio`, `tracing`,
`thiserror`, `time`, `uuid` already exist; do nothing for those.

- [ ] **Step 2: Add the new crate to workspace `members`**

In the same `Cargo.toml`, find `[workspace] members = [`. Insert `"crates/shared/config-reload",` in
alphabetical position (between `command` and `db` or wherever the sort lands).

- [ ] **Step 3: Confirm licenses in `deny.toml`**

Run:

```bash
cargo deny check licenses
```

Expected: `licenses ok`. If `notify-debouncer-full` or `toml` pulls a non-allowlisted license, **stop and surface to
the user** — do not silently add to `deny.toml` allowlist.

- [ ] **Step 4: Create the crate skeleton**

Create `crates/shared/config-reload/Cargo.toml`:

```toml
[package]
name = "uptrakit-config-reload"
version = "0.1.0"
edition = "2024"
license = "Apache-2.0"
publish = false

[lints]
workspace = true

[dependencies]
arc-swap = { workspace = true }
async-trait = { workspace = true }
futures = { workspace = true }
notify-debouncer-full = { workspace = true }
parking_lot = { workspace = true }
rootcause = { workspace = true }
serde = { workspace = true, features = ["derive"] }
thiserror = { workspace = true }
time = { workspace = true, features = ["serde"] }
tokio = { workspace = true, features = ["sync", "rt", "macros", "signal", "time"] }
toml = { workspace = true }
tracing = { workspace = true }
uptrakit-shared-macros = { workspace = true }
uuid = { workspace = true, features = ["serde"] }

[dev-dependencies]
tempfile = { workspace = true }
tokio = { workspace = true, features = ["test-util", "macros", "rt-multi-thread"] }
```

Create `crates/shared/config-reload/src/lib.rs`:

```rust
//! Graceful-reload runtime for the uptrakit Controller.
//!
//! See `docs/superpowers/specs/2026-05-12-graceful-reload-design.md`.

pub mod audit;
pub mod config;
pub mod coordinator;
pub mod defaults;
pub mod delta;
pub mod error;
pub mod loader;
pub mod reconciler;
pub mod reloadable;
pub mod triggers;

pub use audit::ReloadAuditEvent;
pub use config::{RuntimeConfig, Scope};
pub use coordinator::{
    CoordinatorState, DegradedInfo, ReloadCoordinator, ReloadCoordinatorHandle, ReloadPhase,
    ReloadRequest, ReloadSource,
};
pub use delta::RuntimeConfigDelta;
pub use error::ConfigReloadError;
pub use loader::TomlConfigLoader;
pub use reconciler::ConfigReconciler;
pub use reloadable::{Reloadable, ReloadableErased};
```

- [ ] **Step 5: Run `cargo check` to confirm crate parses**

```bash
cargo check -p uptrakit-config-reload
```

Expected: build fails because the module files do not yet exist. The error must be limited to `unresolved module
declarations`; if there are workspace-level errors, fix them before continuing.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml deny.toml crates/shared/config-reload/Cargo.toml crates/shared/config-reload/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(config-reload): scaffold new crate

Add uptrakit-config-reload to the workspace with skeleton lib.rs. New deps
notify-debouncer-full + toml registered in workspace.dependencies. License
check via cargo deny.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: `Scope` + `Validate` trait alias

**Files:**

- Create: `crates/shared/config-reload/src/config/scope.rs`
- Create: `crates/shared/config-reload/src/config/mod.rs` (placeholder)

`Scope` identifies whose `settings_version` row a counter belongs to — the global instance row or a per-tenant row.
`Validate` mirrors the existing trait in `uptrakit-web-api-types` (used on every HTTP request type per the snapshot
rule); we re-use the same shape on `RuntimeConfig` so call sites are uniform.

- [ ] **Step 1: Write failing test for `Scope`**

Append to `tests/coordinator.rs` (create the file if missing):

```rust
use uptrakit_config_reload::config::Scope;

#[test]
fn scope_equality_global() {
    assert_eq!(Scope::Global, Scope::Global);
}

#[test]
fn scope_equality_tenant() {
    let id = uuid::Uuid::nil();
    assert_eq!(Scope::Tenant(id), Scope::Tenant(id));
    assert_ne!(Scope::Tenant(id), Scope::Global);
}
```

- [ ] **Step 2: Confirm tests fail**

```bash
cargo test -p uptrakit-config-reload --test coordinator
```

Expected: compile error — `Scope` unresolved.

- [ ] **Step 3: Implement `Scope`**

Create `crates/shared/config-reload/src/config/scope.rs`:

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identifies a settings_version row.
///
/// Maps 1:1 to the rows the reconciler polls for bumps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    Global,
    Tenant(Uuid),
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Global => f.write_str("global"),
            Self::Tenant(id) => write!(f, "tenant:{id}"),
        }
    }
}
```

Create `crates/shared/config-reload/src/config/mod.rs`:

```rust
pub mod scope;

pub use scope::Scope;

// Section modules and RuntimeConfig follow in subsequent tasks.
```

- [ ] **Step 4: Confirm test passes**

```bash
cargo test -p uptrakit-config-reload --test coordinator -- scope
```

Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/config-reload/src/config/
git commit -m "feat(config-reload): add Scope enum

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: `ConfigReloadError` enum

**Files:**

- Create: `crates/shared/config-reload/src/error.rs`

Per the snapshot rule "use rootcause::Report + thiserror::Error for all error types at module boundaries". We declare
a `thiserror` enum and provide an explicit `From<ConfigReloadError> for rootcause::Report` impl. The workspace's
`impl_report_conversion!` macro is only for **cross-error conversions** (`From<SourceError> for TargetError` with a
specific variant) — it has no single-arg "wrap into Report" form, so we hand-write the conversion. Pattern is
established in other crates (see `crates/shared/wire/src/error.rs`).

- [ ] **Step 1: Write failing test**

Add to `tests/coordinator.rs`:

```rust
use rootcause::Report;
use uptrakit_config_reload::ConfigReloadError;

#[test]
fn error_into_report_carries_message() {
    let err = ConfigReloadError::TomlParse {
        path: "/etc/uptrakit/controller.toml".into(),
        source_msg: "expected `=` at line 3".into(),
    };
    let report: Report = err.into();
    let display = report.to_string();
    assert!(display.contains("controller.toml"));
    assert!(display.contains("line 3"));
}
```

- [ ] **Step 2: Confirm test fails**

```bash
cargo test -p uptrakit-config-reload --test coordinator -- error
```

Expected: `ConfigReloadError` unresolved.

- [ ] **Step 3: Implement error**

Create `crates/shared/config-reload/src/error.rs`:

```rust
use std::path::PathBuf;

use rootcause::Report;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigReloadError {
    #[error("failed to read TOML at {path}: {source_msg}")]
    TomlIo { path: PathBuf, source_msg: String },

    #[error("failed to parse TOML at {path}: {source_msg}")]
    TomlParse { path: PathBuf, source_msg: String },

    #[error("config validation failed: {0}")]
    Validate(String),

    #[error("apply phase failed for subsystem `{subsystem}`: {message}")]
    ApplyFailed { subsystem: String, message: String },

    #[error("revert failed for subsystem `{subsystem}`: {message}")]
    RevertFailed { subsystem: String, message: String },

    #[error("health check failed for subsystem `{subsystem}`: {message}")]
    HealthFailed { subsystem: String, message: String },

    #[error("watchdog timed out for subsystem `{subsystem}` after {ms} ms")]
    WatchdogTimeout { subsystem: String, ms: u128 },

    #[error("coordinator in Degraded state; failed subsystems: {failed:?}")]
    Degraded { failed: Vec<String> },

    #[error("reconciler DB query failed: {0}")]
    Reconciler(String),
}

impl From<ConfigReloadError> for Report {
    fn from(err: ConfigReloadError) -> Self {
        Report::new(err)
    }
}
```

- [ ] **Step 4: Confirm test passes**

```bash
cargo test -p uptrakit-config-reload --test coordinator -- error
```

Expected: 1 test passes.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/config-reload/src/error.rs
git commit -m "feat(config-reload): add ConfigReloadError + rootcause glue

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: `DbConfig` section + `Validate`

**Files:**

- Create: `crates/shared/config-reload/src/config/db.rs`
- Modify: `crates/shared/config-reload/src/config/mod.rs`

- [ ] **Step 1: Write failing test**

Add to `tests/coordinator.rs`:

```rust
use uptrakit_config_reload::config::db::DbConfig;

#[test]
fn db_config_rejects_zero_pool_size() {
    let bad = DbConfig {
        url: "sqlite://x".into(),
        pool_size: 0,
        acquire_timeout_ms: 5000,
        _extra: Default::default(),
    };
    assert!(bad.validate().is_err());
}

#[test]
fn db_config_rejects_zero_timeout() {
    let bad = DbConfig {
        url: "sqlite://x".into(),
        pool_size: 16,
        acquire_timeout_ms: 0,
        _extra: Default::default(),
    };
    assert!(bad.validate().is_err());
}

#[test]
fn db_config_accepts_valid_values() {
    let good = DbConfig {
        url: "sqlite://x".into(),
        pool_size: 16,
        acquire_timeout_ms: 5000,
        _extra: Default::default(),
    };
    assert!(good.validate().is_ok());
}

#[test]
fn db_config_deny_unknown_fields_at_parse() {
    let bad = r#"
url = "sqlite://x"
pool_size = 16
acquire_timeout_ms = 5000
unknown_key = "value"
"#;
    let parsed: Result<DbConfig, _> = toml::from_str(bad);
    // _extra captures unknown — must NOT error at parse time (escape hatch).
    let cfg = parsed.expect("parse should succeed; unknowns land in _extra");
    assert_eq!(cfg._extra.len(), 1);
    assert!(cfg._extra.contains_key("unknown_key"));
}
```

- [ ] **Step 2: Confirm tests fail**

```bash
cargo test -p uptrakit-config-reload --test coordinator -- db_config
```

Expected: compile error.

- [ ] **Step 3: Implement `DbConfig`**

Create `crates/shared/config-reload/src/config/db.rs`:

```rust
use std::collections::HashMap;

use rootcause::Report;
use serde::{Deserialize, Serialize};

use crate::error::ConfigReloadError;

/// `[db]` section.
///
/// `url` is in the irreversibly-bound set (spec §11.1) — changes trigger reexec.
/// `pool_size` and `acquire_timeout_ms` reload in-process via reconnect (spec §10.3).
///
/// **Construction:** `#[non_exhaustive]` blocks struct-literal construction from external crates.
/// Use `DbConfig::default()` then mutate the fields you need, or build via `serde::Deserialize`
/// from TOML. Inside the defining crate (tests in this file), struct literals are still valid.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct DbConfig {
    pub url: String,
    #[serde(default = "default_pool_size")]
    pub pool_size: u32,
    #[serde(default = "default_acquire_timeout")]
    pub acquire_timeout_ms: u64,

    /// Unknown keys captured for tolerant downgrade (spec §6.1). Warned, never errored.
    #[serde(flatten)]
    pub _extra: HashMap<String, toml::Value>,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            pool_size: default_pool_size(),
            acquire_timeout_ms: default_acquire_timeout(),
            _extra: HashMap::new(),
        }
    }
}

const fn default_pool_size() -> u32 { 16 }
const fn default_acquire_timeout() -> u64 { 5_000 }

impl DbConfig {
    pub fn validate(&self) -> Result<(), Report> {
        if self.url.is_empty() {
            return Err(ConfigReloadError::Validate("db.url is empty".into()).into());
        }
        if self.pool_size == 0 {
            return Err(ConfigReloadError::Validate("db.pool_size must be >= 1".into()).into());
        }
        if self.acquire_timeout_ms == 0 {
            return Err(
                ConfigReloadError::Validate("db.acquire_timeout_ms must be > 0".into()).into(),
            );
        }
        Ok(())
    }
}
```

Append `pub mod db;` and `pub use db::DbConfig;` to `crates/shared/config-reload/src/config/mod.rs`.

- [ ] **Step 4: Confirm tests pass**

```bash
cargo test -p uptrakit-config-reload --test coordinator -- db_config
```

Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/config-reload/src/config/db.rs crates/shared/config-reload/src/config/mod.rs
git commit -m "feat(config-reload): add DbConfig section

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Remaining section structs (`Network`, `Nats`, `Tls`, `Audit`, `Log`, `MasterKey`, `EmbeddedServices`, `Zeroconf`)

**Files:**

- Create: `crates/shared/config-reload/src/config/network.rs`
- Create: `crates/shared/config-reload/src/config/nats.rs`
- Create: `crates/shared/config-reload/src/config/tls.rs`
- Create: `crates/shared/config-reload/src/config/audit.rs`
- Create: `crates/shared/config-reload/src/config/log.rs`
- Create: `crates/shared/config-reload/src/config/master_key.rs`
- Create: `crates/shared/config-reload/src/config/embedded.rs`
- Create: `crates/shared/config-reload/src/config/zeroconf.rs`
- Modify: `crates/shared/config-reload/src/config/mod.rs`

Each section follows the `DbConfig` pattern. All structs are `#[non_exhaustive]`, derive
`Clone + Debug + Deserialize + Serialize + PartialEq`, carry `#[serde(flatten)] pub _extra:
HashMap<String, toml::Value>`, **explicitly implement `Default`** (so external crates can
construct them — `#[non_exhaustive]` forbids struct literals outside the defining crate), and
have a `validate(&self) -> Result<(), Report>` method.

- [ ] **Step 1: Write failing parse tests**

Add to `tests/loader.rs` (create if missing):

```rust
use uptrakit_config_reload::config::{
    AuditConfig, EmbeddedServicesConfig, LogConfig, MasterKeyConfig, NatsConfig, NetworkConfig,
    TlsConfig, ZeroconfConfig,
};

#[test]
fn network_parses_https_and_pki() {
    let toml = r#"
[https]
addr = "0.0.0.0:8443"
trusted_proxies = ["127.0.0.1/32"]
real_ip_header = "x-forwarded-for"
forwarded_client_cert_info_header = "x-fcc"
forwarded_client_cert_pem_header  = "x-fcc-pem"

[pki]
addr = "0.0.0.0:8444"
"#;
    let parsed: NetworkConfig = toml::from_str(toml).unwrap();
    assert_eq!(parsed.https.addr, "0.0.0.0:8443");
    assert_eq!(parsed.pki.addr, "0.0.0.0:8444");
    assert!(parsed.validate().is_ok());
}

#[test]
fn network_rejects_collision() {
    let toml = r#"
[https]
addr = "0.0.0.0:8443"
trusted_proxies = []
real_ip_header = "x-forwarded-for"
forwarded_client_cert_info_header = "x-fcc"
forwarded_client_cert_pem_header  = "x-fcc-pem"

[pki]
addr = "0.0.0.0:8443"
"#;
    let parsed: NetworkConfig = toml::from_str(toml).unwrap();
    assert!(parsed.validate().is_err(), "https and pki on same addr must fail");
}

#[test]
fn audit_rejects_unknown_filter() {
    let bad: AuditConfig = toml::from_str(r#"
filter = "weird"
retention_days = 90
"#).unwrap();
    assert!(bad.validate().is_err());
}

#[test]
fn audit_accepts_known_filters() {
    for filter in ["all", "mutations", "none"] {
        let cfg: AuditConfig =
            toml::from_str(&format!("filter = \"{filter}\"\nretention_days = 90\n")).unwrap();
        cfg.validate().unwrap();
    }
}

#[test]
fn nats_validates_url() {
    let good: NatsConfig = toml::from_str(r#"url = "nats://localhost:4222""#).unwrap();
    assert!(good.validate().is_ok());

    let bad: NatsConfig = toml::from_str(r#"url = """#).unwrap();
    assert!(bad.validate().is_err());
}

#[test]
fn tls_requires_both_paths() {
    let bad: TlsConfig = toml::from_str(r#"
cert_path = "/etc/tls/cert.pem"
key_path = ""
sans = []
"#).unwrap();
    assert!(bad.validate().is_err());
}

#[test]
fn embedded_services_accepts_all_false() {
    let cfg: EmbeddedServicesConfig = toml::from_str(r#"
agent = false
agent_ssh = false
mqtt = false
scheduler = false
"#).unwrap();
    cfg.validate().unwrap();
}
```

- [ ] **Step 2: Confirm tests fail**

```bash
cargo test -p uptrakit-config-reload --test loader
```

Expected: unresolved-module errors.

- [ ] **Step 3: Implement sections**

Create each file. Shape repeats per section; sample `network.rs`:

```rust
use std::collections::HashMap;
use std::net::SocketAddr;

use rootcause::Report;
use serde::{Deserialize, Serialize};

use crate::error::ConfigReloadError;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct NetworkConfig {
    pub https: HttpsConfig,
    pub pki: PkiConfig,
    #[serde(flatten)]
    pub _extra: HashMap<String, toml::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct HttpsConfig {
    pub addr: String,
    #[serde(default)]
    pub trusted_proxies: Vec<String>,
    #[serde(default = "default_real_ip")]
    pub real_ip_header: String,
    #[serde(default = "default_fcc_info")]
    pub forwarded_client_cert_info_header: String,
    #[serde(default = "default_fcc_pem")]
    pub forwarded_client_cert_pem_header: String,
    #[serde(flatten)]
    pub _extra: HashMap<String, toml::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct PkiConfig {
    pub addr: String,
    #[serde(flatten)]
    pub _extra: HashMap<String, toml::Value>,
}

fn default_real_ip() -> String { "x-forwarded-for".into() }
fn default_fcc_info() -> String { "x-forwarded-client-cert".into() }
fn default_fcc_pem() -> String { "x-forwarded-client-cert-pem".into() }

impl NetworkConfig {
    pub fn validate(&self) -> Result<(), Report> {
        let https_sa = self.https.addr.parse::<SocketAddr>().map_err(|e| {
            ConfigReloadError::Validate(format!("network.https.addr invalid: {e}"))
        })?;
        let pki_sa = self.pki.addr.parse::<SocketAddr>().map_err(|e| {
            ConfigReloadError::Validate(format!("network.pki.addr invalid: {e}"))
        })?;
        if https_sa == pki_sa {
            return Err(ConfigReloadError::Validate(
                "network.https.addr must differ from network.pki.addr".into(),
            )
            .into());
        }
        Ok(())
    }
}
```

Implement `NatsConfig`, `TlsConfig`, `AuditConfig`, `LogConfig`, `MasterKeyConfig`, `EmbeddedServicesConfig`,
`ZeroconfConfig` analogously. Each gets:

- `#[non_exhaustive]` + `#[serde(flatten)] pub _extra: HashMap<String, toml::Value>`
- Explicit defaults via `#[serde(default = "fn_name")]` where needed
- A `validate()` returning `Result<(), Report>`
- AuditConfig validates `filter ∈ {all, mutations, none}` and `retention_days ≥ 0`
- TlsConfig validates both paths non-empty (file existence is _not_ checked here — that's runtime concern)
- NatsConfig validates URL non-empty (full URL parsing happens at connect time in Plan 2)
- LogConfig validates `path` non-empty and `level` parseable as `tracing::Level`
- MasterKeyConfig validates `path` non-empty and absolute
- EmbeddedServicesConfig validates booleans (always ok; topology change detection lives in the coordinator)
- ZeroconfConfig validates `url`/`pki_addr` non-empty when `enabled = true`

Add `pub mod` lines + re-exports to `crates/shared/config-reload/src/config/mod.rs`:

```rust
pub mod audit;
pub mod db;
pub mod embedded;
pub mod log;
pub mod master_key;
pub mod nats;
pub mod network;
pub mod scope;
pub mod tls;
pub mod zeroconf;

pub use audit::AuditConfig;
pub use db::DbConfig;
pub use embedded::EmbeddedServicesConfig;
pub use log::LogConfig;
pub use master_key::MasterKeyConfig;
pub use nats::NatsConfig;
pub use network::{HttpsConfig, NetworkConfig, PkiConfig};
pub use scope::Scope;
pub use tls::TlsConfig;
pub use zeroconf::ZeroconfConfig;
```

- [ ] **Step 4: Confirm tests pass**

```bash
cargo test -p uptrakit-config-reload --test loader
```

Expected: all section tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/config-reload/src/config/
git commit -m "feat(config-reload): add section structs with Validate impls

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: `RuntimeConfig` root + cross-section validation + `warn_about_extras()`

**Files:**

- Modify: `crates/shared/config-reload/src/config/mod.rs`

- [ ] **Step 1: Write failing test**

Add to `tests/loader.rs`:

```rust
use uptrakit_config_reload::RuntimeConfig;

#[test]
fn runtime_config_full_round_trip() {
    let toml = r#"
[db]
url = "sqlite://var/lib/uptrakit/controller.db"
pool_size = 16
acquire_timeout_ms = 5000

[master_key]
path = "/etc/uptrakit/master.key"

[network.https]
addr = "0.0.0.0:8443"
trusted_proxies = []
real_ip_header = "x-forwarded-for"
forwarded_client_cert_info_header = "x-fcc"
forwarded_client_cert_pem_header  = "x-fcc-pem"

[network.pki]
addr = "0.0.0.0:8444"

[tls]
cert_path = "/etc/uptrakit/tls/cert.pem"
key_path  = "/etc/uptrakit/tls/key.pem"
sans      = []

[nats]
url = "nats://localhost:4222"

[audit]
filter = "all"
retention_days = 90

[log]
path  = "/var/log/uptrakit/controller.log"
level = "info"

[zeroconf]
enabled = true
url      = "https://controller.local:8443"
pki_addr = "controller.local:8444"

[embedded_services]
agent = false
agent_ssh = false
mqtt = false
scheduler = true
"#;
    let cfg: RuntimeConfig = toml::from_str(toml).unwrap();
    cfg.validate().expect("full TOML must validate");
    assert!(cfg.warn_about_extras().is_empty());
}

#[test]
fn runtime_config_captures_unknown_keys() {
    let toml = r#"
[db]
url = "sqlite://x"
pool_size = 16
acquire_timeout_ms = 5000
poool_size = 32  # typo, lands in _extra

[master_key]
path = "/etc/uptrakit/master.key"

[network.https]
addr = "0.0.0.0:8443"
trusted_proxies = []
real_ip_header = "x-forwarded-for"
forwarded_client_cert_info_header = "x-fcc"
forwarded_client_cert_pem_header  = "x-fcc-pem"

[network.pki]
addr = "0.0.0.0:8444"

[tls]
cert_path = "x"
key_path  = "y"
sans      = []

[nats]
url = "nats://x"

[audit]
filter = "all"
retention_days = 90

[log]
path  = "x"
level = "info"

[zeroconf]
enabled = false
url      = ""
pki_addr = ""

[embedded_services]
agent = false
agent_ssh = false
mqtt = false
scheduler = false
"#;
    let cfg: RuntimeConfig = toml::from_str(toml).unwrap();
    let warnings = cfg.warn_about_extras();
    assert!(warnings.iter().any(|w| w.contains("poool_size")));
}
```

- [ ] **Step 2: Confirm tests fail**

```bash
cargo test -p uptrakit-config-reload --test loader -- runtime_config
```

Expected: `RuntimeConfig` unresolved.

- [ ] **Step 3: Implement `RuntimeConfig`**

Append to `crates/shared/config-reload/src/config/mod.rs`:

```rust
use rootcause::Report;
use serde::{Deserialize, Serialize};

use crate::error::ConfigReloadError;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct RuntimeConfig {
    pub db: DbConfig,
    pub master_key: MasterKeyConfig,
    pub network: NetworkConfig,
    pub tls: TlsConfig,
    pub nats: NatsConfig,
    pub audit: AuditConfig,
    pub log: LogConfig,
    #[serde(default)]
    pub zeroconf: ZeroconfConfig,
    pub embedded_services: EmbeddedServicesConfig,
}

impl RuntimeConfig {
    /// Run all per-section validations + cross-section invariants.
    pub fn validate(&self) -> Result<(), Report> {
        self.db.validate()?;
        self.master_key.validate()?;
        self.network.validate()?;
        self.tls.validate()?;
        self.nats.validate()?;
        self.audit.validate()?;
        self.log.validate()?;
        self.zeroconf.validate()?;
        self.embedded_services.validate()?;

        // Cross-section: https vs pki addr collision is checked inside network.validate().
        // Add further cross-section invariants here as the spec evolves.
        let _ = ConfigReloadError::Validate("placeholder".into()); // keep import used
        Ok(())
    }

    /// Emit human-readable warnings for keys captured by `_extra` (unknown-field escape hatch).
    /// Caller emits these via `tracing::warn!` and writes a `system_alerts` Warning row.
    pub fn warn_about_extras(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (key, _) in &self.db._extra {
            out.push(format!("[db] unknown key `{key}` ignored"));
        }
        for (key, _) in &self.master_key._extra {
            out.push(format!("[master_key] unknown key `{key}` ignored"));
        }
        for (key, _) in &self.network._extra {
            out.push(format!("[network] unknown key `{key}` ignored"));
        }
        for (key, _) in &self.network.https._extra {
            out.push(format!("[network.https] unknown key `{key}` ignored"));
        }
        for (key, _) in &self.network.pki._extra {
            out.push(format!("[network.pki] unknown key `{key}` ignored"));
        }
        for (key, _) in &self.tls._extra {
            out.push(format!("[tls] unknown key `{key}` ignored"));
        }
        for (key, _) in &self.nats._extra {
            out.push(format!("[nats] unknown key `{key}` ignored"));
        }
        for (key, _) in &self.audit._extra {
            out.push(format!("[audit] unknown key `{key}` ignored"));
        }
        for (key, _) in &self.log._extra {
            out.push(format!("[log] unknown key `{key}` ignored"));
        }
        for (key, _) in &self.zeroconf._extra {
            out.push(format!("[zeroconf] unknown key `{key}` ignored"));
        }
        for (key, _) in &self.embedded_services._extra {
            out.push(format!("[embedded_services] unknown key `{key}` ignored"));
        }
        out
    }
}
```

- [ ] **Step 4: Confirm tests pass**

```bash
cargo test -p uptrakit-config-reload --test loader -- runtime_config
```

Expected: both tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/config-reload/src/config/mod.rs
git commit -m "feat(config-reload): RuntimeConfig root + warn_about_extras

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: `TomlConfigLoader` + `--check-config`

**Files:**

- Create: `crates/shared/config-reload/src/loader.rs`
- Modify: `crates/shared/config-reload/src/lib.rs` (already declares `pub mod loader;`)

- [ ] **Step 1: Write failing tests**

Add to `tests/loader.rs`:

```rust
use std::io::Write;
use tempfile::NamedTempFile;
use uptrakit_config_reload::TomlConfigLoader;

#[test]
fn loader_validate_only_passes_for_minimal_valid_toml() {
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "{}", minimal_toml()).unwrap();
    let result = TomlConfigLoader::validate_only(f.path());
    assert!(result.is_ok(), "validate_only failed: {result:?}");
}

#[test]
fn loader_validate_only_fails_for_bad_toml() {
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "not valid toml = =").unwrap();
    let result = TomlConfigLoader::validate_only(f.path());
    assert!(result.is_err());
}

#[test]
fn loader_load_emits_extras_warnings() {
    let mut f = NamedTempFile::new().unwrap();
    write!(f, "{}", minimal_toml_with_typo()).unwrap();
    let loaded = TomlConfigLoader::load(f.path()).unwrap();
    assert!(!loaded.warnings.is_empty(), "warnings should fire for the typo");
    assert!(loaded.warnings.iter().any(|w| w.contains("poool_size")));
    drop(loaded);
}

fn minimal_toml() -> String { /* full TOML literal as in Task 6 round-trip test */ }
fn minimal_toml_with_typo() -> String { /* same, with poool_size = 32 in [db] */ }
```

- [ ] **Step 2: Confirm tests fail**

```bash
cargo test -p uptrakit-config-reload --test loader -- loader_
```

Expected: `TomlConfigLoader` unresolved.

- [ ] **Step 3: Implement loader**

Create `crates/shared/config-reload/src/loader.rs`:

```rust
use std::path::{Path, PathBuf};

use rootcause::Report;

use crate::config::RuntimeConfig;
use crate::error::ConfigReloadError;

/// Result of loading + validating a TOML config file.
///
/// Returned as a named struct (not a tuple) so call sites stay readable:
/// `loaded.config.network.https.addr` vs `loaded.0.network.https.addr`.
#[non_exhaustive]
pub struct LoadedConfig {
    pub config: RuntimeConfig,
    pub warnings: Vec<String>,
}

pub struct TomlConfigLoader;

impl TomlConfigLoader {
    /// Parse TOML + run per-section + cross-section validate. Returns the config plus the list of
    /// warning strings for `_extra` keys (caller decides how to surface them — `tracing::warn!`
    /// for runtime, stderr for `--check-config`).
    ///
    /// # Errors
    ///
    /// Returns `ConfigReloadError::TomlIo` if the file can't be read, `TomlParse` on bad TOML,
    /// or `Validate` if cross-section invariants fail.
    pub fn load(path: impl AsRef<Path>) -> Result<LoadedConfig, Report> {
        let path = path.as_ref();
        let bytes = std::fs::read_to_string(path).map_err(|e| ConfigReloadError::TomlIo {
            path: path.to_path_buf(),
            source_msg: e.to_string(),
        })?;
        let config: RuntimeConfig =
            toml::from_str(&bytes).map_err(|e| ConfigReloadError::TomlParse {
                path: path.to_path_buf(),
                source_msg: e.to_string(),
            })?;
        config.validate()?;
        let warnings = config.warn_about_extras();
        Ok(LoadedConfig { config, warnings })
    }

    /// `--check-config` entry point: parse + validate, print warnings to stderr, return Result.
    /// Performs no network / DB / master-key file probes.
    ///
    /// # Errors
    ///
    /// Same as [`load`](Self::load).
    pub fn validate_only(path: impl AsRef<Path>) -> Result<(), Report> {
        let loaded = Self::load(path)?;
        for w in &loaded.warnings {
            eprintln!("warning: {w}");
        }
        Ok(())
    }
}
```

Re-export `LoadedConfig` from `lib.rs` alongside `TomlConfigLoader`.

- [ ] **Step 4: Confirm tests pass**

```bash
cargo test -p uptrakit-config-reload --test loader
```

Expected: all loader tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/config-reload/src/loader.rs
git commit -m "feat(config-reload): TomlConfigLoader::load + validate_only

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: `RuntimeConfigDelta` + defaults

**Files:**

- Create: `crates/shared/config-reload/src/delta.rs`
- Create: `crates/shared/config-reload/src/defaults.rs`

- [ ] **Step 1: Implement `RuntimeConfigDelta`**

Create `crates/shared/config-reload/src/delta.rs`:

```rust
use std::sync::Arc;

use crate::config::{
    AuditConfig, DbConfig, EmbeddedServicesConfig, NatsConfig, NetworkConfig, TlsConfig,
    ZeroconfConfig,
};

/// In-process delta carrying the new value for one Config Section.
///
/// Wire-incompatible by design: never serialised. Adding a section means: add a variant here,
/// write the matching `Reloadable` + `ReloadableErased` pair, register it in the coordinator.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum RuntimeConfigDelta {
    Db(Arc<DbConfig>),
    Network(Arc<NetworkConfig>),
    Nats(Arc<NatsConfig>),
    Tls(Arc<TlsConfig>),
    Audit(Arc<AuditConfig>),
    Zeroconf(Arc<ZeroconfConfig>),
    EmbeddedServices(Arc<EmbeddedServicesConfig>),
}
```

- [ ] **Step 2: Implement `defaults`**

Create `crates/shared/config-reload/src/defaults.rs`:

```rust
use std::time::Duration;

// Per spec §9.1. TODO: expose as TOML keys if Operator demand surfaces.
pub const WATCHDOG_DB_POOL:  Duration = Duration::from_secs(15);
pub const WATCHDOG_NATS:     Duration = Duration::from_secs(10);
pub const WATCHDOG_HTTPS:    Duration = Duration::from_secs(5);
pub const WATCHDOG_PKI:      Duration = Duration::from_secs(5);
pub const WATCHDOG_PLUGINS:  Duration = Duration::from_secs(30);
pub const WATCHDOG_AUDIT:    Duration = Duration::from_secs(5);
pub const WATCHDOG_ZEROCONF: Duration = Duration::from_secs(5);
pub const WATCHDOG_EMBEDDED: Duration = Duration::from_secs(30);

pub const HTTPS_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);
pub const PKI_DRAIN_TIMEOUT:   Duration = Duration::from_secs(5);

pub const FILE_WATCH_DEBOUNCE: Duration = Duration::from_millis(500);
pub const RECONCILER_POLL:     Duration = Duration::from_secs(2);
```

- [ ] **Step 3: Compile check**

```bash
cargo check -p uptrakit-config-reload
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/shared/config-reload/src/delta.rs crates/shared/config-reload/src/defaults.rs
git commit -m "feat(config-reload): RuntimeConfigDelta + watchdog defaults

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: `Reloadable` + `ReloadableErased` traits

**Files:**

- Create: `crates/shared/config-reload/src/reloadable/mod.rs`

- [ ] **Step 1: Write failing test**

Add to `tests/coordinator.rs`:

```rust
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rootcause::Report;
use uptrakit_config_reload::{
    config::DbConfig, defaults, Reloadable, ReloadableErased, RuntimeConfigDelta,
};

struct StubDb;

impl Reloadable for StubDb {
    type Config = DbConfig;
    fn name(&self) -> &'static str { "stub_db" }
    fn validate(&self, _: &DbConfig) -> Result<(), Report> { Ok(()) }
    async fn apply(&self, _: Arc<DbConfig>) -> Result<(), Report> { Ok(()) }
    async fn revert(&self) -> Result<(), Report> { Ok(()) }
    async fn health_check(&self) -> Result<(), Report> { Ok(()) }
    fn rollback_window(&self) -> Duration { defaults::WATCHDOG_DB_POOL }
}

struct StubDbErased(StubDb);

#[async_trait]
impl ReloadableErased for StubDbErased {
    fn name(&self) -> &'static str { self.0.name() }
    fn validate(&self, delta: &RuntimeConfigDelta) -> Result<(), Report> {
        if let RuntimeConfigDelta::Db(cfg) = delta {
            self.0.validate(cfg)
        } else { Ok(()) }
    }
    async fn apply(&self, delta: &RuntimeConfigDelta) -> Result<(), Report> {
        if let RuntimeConfigDelta::Db(cfg) = delta { self.0.apply(cfg.clone()).await } else { Ok(()) }
    }
    async fn revert(&self) -> Result<(), Report> { self.0.revert().await }
    async fn health_check(&self) -> Result<(), Report> { self.0.health_check().await }
    fn rollback_window(&self) -> Duration { self.0.rollback_window() }
}

#[tokio::test]
async fn reloadable_erased_dispatches() {
    let erased: Box<dyn ReloadableErased> = Box::new(StubDbErased(StubDb));
    erased.health_check().await.unwrap();
}
```

- [ ] **Step 2: Confirm test fails**

```bash
cargo test -p uptrakit-config-reload --test coordinator -- reloadable_erased
```

Expected: traits unresolved.

- [ ] **Step 3: Implement traits**

Create `crates/shared/config-reload/src/reloadable/mod.rs`:

````rust
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rootcause::Report;

use crate::delta::RuntimeConfigDelta;

/// Typed reload contract for a long-lived subsystem.
///
/// Implementors keep their pre-apply snapshot internally and restore it from `revert()`.
/// Not object-safe (associated type + async fn). Object-safe form is `ReloadableErased`.
pub trait Reloadable: Send + Sync {
    type Config: Send + Sync + 'static;

    fn name(&self) -> &'static str;

    fn validate(&self, new: &Self::Config) -> Result<(), Report>;

    /// Apply MUST internally snapshot enough pre-apply state for `revert()` to restore.
    fn apply(
        &self,
        new: Arc<Self::Config>,
    ) -> impl std::future::Future<Output = Result<(), Report>> + Send + '_;

    fn revert(&self) -> impl std::future::Future<Output = Result<(), Report>> + Send + '_;

    fn health_check(&self) -> impl std::future::Future<Output = Result<(), Report>> + Send + '_;

    fn rollback_window(&self) -> Duration;
}

/// Object-safe wrapper used by the coordinator's heterogeneous registry.
///
/// Each subsystem owns one impl that downcasts the relevant `RuntimeConfigDelta` variant.
/// Use `async_trait` so the `dyn`-compat path lives in one place.
#[async_trait]
pub trait ReloadableErased: Send + Sync {
    fn name(&self) -> &'static str;
    fn validate(&self, delta: &RuntimeConfigDelta) -> Result<(), Report>;
    async fn apply(&self, delta: &RuntimeConfigDelta) -> Result<(), Report>;
    async fn revert(&self) -> Result<(), Report>;
    async fn health_check(&self) -> Result<(), Report>;
    fn rollback_window(&self) -> Duration;
}

/// Generate the `#[async_trait] impl ReloadableErased for $struct` body that downcasts a single
/// `RuntimeConfigDelta` variant and forwards to the typed `Reloadable` impl.
///
/// Each subsystem in Plan 2 invokes this once instead of hand-writing 25 lines of forwarding
/// boilerplate per Reloadable. The variant name is the `RuntimeConfigDelta::Xxx` arm; the macro
/// extracts the payload via `if let` and forwards the `Arc<Config>` to the typed methods.
///
/// Example:
///
/// ```ignore
/// reloadable_erased_impl!(DbPoolReloadable, RuntimeConfigDelta::Db);
/// ```
#[macro_export]
macro_rules! reloadable_erased_impl {
    ($struct:ty, $variant:path) => {
        #[::async_trait::async_trait]
        impl $crate::reloadable::ReloadableErased for $struct {
            fn name(&self) -> &'static str {
                <Self as $crate::reloadable::Reloadable>::name(self)
            }
            fn validate(
                &self,
                delta: &$crate::delta::RuntimeConfigDelta,
            ) -> ::std::result::Result<(), ::rootcause::Report> {
                if let $variant(cfg) = delta {
                    <Self as $crate::reloadable::Reloadable>::validate(self, cfg)
                } else {
                    Ok(())
                }
            }
            async fn apply(
                &self,
                delta: &$crate::delta::RuntimeConfigDelta,
            ) -> ::std::result::Result<(), ::rootcause::Report> {
                if let $variant(cfg) = delta {
                    <Self as $crate::reloadable::Reloadable>::apply(self, cfg.clone()).await
                } else {
                    Ok(())
                }
            }
            async fn revert(&self) -> ::std::result::Result<(), ::rootcause::Report> {
                <Self as $crate::reloadable::Reloadable>::revert(self).await
            }
            async fn health_check(&self) -> ::std::result::Result<(), ::rootcause::Report> {
                <Self as $crate::reloadable::Reloadable>::health_check(self).await
            }
            fn rollback_window(&self) -> ::std::time::Duration {
                <Self as $crate::reloadable::Reloadable>::rollback_window(self)
            }
        }
    };
}
````

Plan 2's subsystem tasks call `reloadable_erased_impl!(MyReloadable, RuntimeConfigDelta::MyVariant);`
instead of repeating the forwarding boilerplate. The macro is `#[macro_export]`-ed so the controller-runtime
crate's `Reloadable` implementors can reach it.

- [ ] **Step 4: Confirm tests pass**

```bash
cargo test -p uptrakit-config-reload --test coordinator -- reloadable_erased
```

Expected: 1 test passes.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/config-reload/src/reloadable/
git commit -m "feat(config-reload): Reloadable + ReloadableErased traits

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 10: `ReloadSource`, `ReloadPhase`, `ReloadRequest`, `CoordinatorState`, `DegradedInfo`

**Files:**

- Create: `crates/shared/config-reload/src/coordinator/mod.rs`

- [ ] **Step 1: Write failing test**

Add to `tests/coordinator.rs`:

```rust
use uptrakit_config_reload::{
    CoordinatorState, DegradedInfo, ReloadPhase, ReloadRequest, ReloadSource,
};

#[test]
fn reload_source_serde_roundtrip_includes_other_catch_all() {
    let s = ReloadSource::Other("future trigger".into());
    let json = serde_json::to_string(&s).unwrap();
    let back: ReloadSource = serde_json::from_str(&json).unwrap();
    matches!(back, ReloadSource::Other(ref msg) if msg == "future trigger");
}

#[test]
fn reload_phase_serde_roundtrip_includes_other() {
    let p = ReloadPhase::Other("future phase".into());
    let json = serde_json::to_string(&p).unwrap();
    let back: ReloadPhase = serde_json::from_str(&json).unwrap();
    matches!(back, ReloadPhase::Other(ref msg) if msg == "future phase");
}

#[test]
fn coordinator_state_degraded_carries_info() {
    let info = DegradedInfo {
        since: time::OffsetDateTime::now_utc(),
        failed_subsystems: vec!["nats".into()],
        reason: "revert returned Err on nats".into(),
    };
    let state = CoordinatorState::Degraded(info.clone());
    matches!(state, CoordinatorState::Degraded(_));
}
```

- [ ] **Step 2: Confirm tests fail**

```bash
cargo test -p uptrakit-config-reload --test coordinator -- reload_source
```

Expected: types unresolved.

- [ ] **Step 3: Implement coordinator types**

Create `crates/shared/config-reload/src/coordinator/mod.rs`:

```rust
mod state_machine;

pub use state_machine::ReloadCoordinator;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::config::Scope;

/// Trigger source for a reload request. Wire-exposed via audit-log JSON; carries `Other(String)`
/// per the snapshot rule.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReloadSource {
    Sighup,
    FileWatch { path: PathBuf },
    DbBump { scope: Scope, sections: Vec<String> },
    Boot,
    Other(String),
}

/// Coordinator phase recorded in `ConfigReloadFailed` audit events. Wire-exposed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum ReloadPhase {
    Validate,
    Apply,
    Watchdog,
    Reexec,
    Other(String),
}

#[derive(Clone, Debug)]
pub struct ReloadRequest {
    pub source: ReloadSource,
    pub timestamp: OffsetDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoordinatorState {
    Idle,
    Reloading,
    Degraded(DegradedInfo),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct DegradedInfo {
    pub since: OffsetDateTime,
    pub failed_subsystems: Vec<String>,
    pub reason: String,
}

/// Handle for external introspection (used by `GET /api/v1/instance/config-state` in Plan 3).
#[derive(Clone)]
pub struct ReloadCoordinatorHandle {
    pub(crate) state: std::sync::Arc<arc_swap::ArcSwap<CoordinatorState>>,
    pub(crate) tx: tokio::sync::mpsc::Sender<ReloadRequest>,
}

impl ReloadCoordinatorHandle {
    pub fn state(&self) -> CoordinatorState {
        (**self.state.load()).clone()
    }

    pub async fn enqueue(&self, request: ReloadRequest) -> Result<(), tokio::sync::mpsc::error::SendError<ReloadRequest>> {
        self.tx.send(request).await
    }
}
```

- [ ] **Step 4: Confirm tests pass**

```bash
cargo test -p uptrakit-config-reload --test coordinator -- reload_source
```

Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/config-reload/src/coordinator/mod.rs
git commit -m "feat(config-reload): ReloadSource/Phase/State + Coordinator handle

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 11: `ReloadCoordinator` state-machine helpers (validate / apply / watchdog / revert)

**Files:**

- Create: `crates/shared/config-reload/src/coordinator/state_machine.rs`

These helpers are split into small `async fn`s so individual futures stay below the
`clippy::large_futures = "deny"` threshold (snapshot rule).

- [ ] **Step 1: Write failing test**

Add to `tests/coordinator.rs`:

```rust
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use rootcause::Report;
use uptrakit_config_reload::{
    config::DbConfig, ReloadCoordinator, ReloadRequest, ReloadSource, ReloadableErased,
    RuntimeConfigDelta, CoordinatorState,
};

#[derive(Default)]
struct CountingReloadable {
    validated: AtomicUsize,
    applied: AtomicUsize,
    reverted: AtomicUsize,
    healthy: bool,
}

#[async_trait]
impl ReloadableErased for CountingReloadable {
    fn name(&self) -> &'static str { "counter" }
    fn validate(&self, _delta: &RuntimeConfigDelta) -> Result<(), Report> {
        self.validated.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn apply(&self, _delta: &RuntimeConfigDelta) -> Result<(), Report> {
        self.applied.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn revert(&self) -> Result<(), Report> {
        self.reverted.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn health_check(&self) -> Result<(), Report> {
        if self.healthy { Ok(()) } else {
            Err(rootcause::report!("unhealthy"))
        }
    }
    fn rollback_window(&self) -> Duration { Duration::from_millis(200) }
}

#[tokio::test(start_paused = true)]
async fn happy_path_apply_commits() {
    let r = Arc::new(CountingReloadable { healthy: true, ..Default::default() });
    let coord = ReloadCoordinator::new_for_test(vec![r.clone()]);
    coord.enqueue_and_drain(test_delta()).await;

    assert_eq!(r.validated.load(Ordering::SeqCst), 1);
    assert_eq!(r.applied.load(Ordering::SeqCst), 1);
    assert_eq!(r.reverted.load(Ordering::SeqCst), 0);
    assert!(matches!(coord.state(), CoordinatorState::Idle));
}

#[tokio::test(start_paused = true)]
async fn unhealthy_subsystem_triggers_atomic_revert_all() {
    let healthy = Arc::new(CountingReloadable { healthy: true, ..Default::default() });
    let unhealthy = Arc::new(CountingReloadable { healthy: false, ..Default::default() });
    let coord = ReloadCoordinator::new_for_test(vec![healthy.clone(), unhealthy.clone()]);
    coord.enqueue_and_drain(test_delta()).await;

    // Atomic rule: both revert, even though only one was unhealthy.
    assert_eq!(healthy.reverted.load(Ordering::SeqCst), 1);
    assert_eq!(unhealthy.reverted.load(Ordering::SeqCst), 1);
    assert!(matches!(coord.state(), CoordinatorState::Idle));
}

fn test_delta() -> RuntimeConfigDelta {
    RuntimeConfigDelta::Db(Arc::new(DbConfig {
        url: "sqlite://x".into(),
        pool_size: 16,
        acquire_timeout_ms: 5_000,
        _extra: Default::default(),
    }))
}
```

- [ ] **Step 2: Confirm tests fail**

```bash
cargo test -p uptrakit-config-reload --test coordinator -- happy_path
```

Expected: `ReloadCoordinator::new_for_test` unresolved.

- [ ] **Step 3: Implement coordinator state machine**

Create `crates/shared/config-reload/src/coordinator/state_machine.rs`:

```rust
use std::sync::Arc;

use arc_swap::ArcSwap;
use futures::stream::{FuturesUnordered, StreamExt};
use rootcause::Report;
use time::OffsetDateTime;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::audit::ReloadAuditEvent;
use crate::coordinator::{
    CoordinatorState, DegradedInfo, ReloadCoordinatorHandle, ReloadPhase, ReloadRequest,
};
use crate::delta::RuntimeConfigDelta;
use crate::reloadable::ReloadableErased;

pub struct ReloadCoordinator {
    state: Arc<ArcSwap<CoordinatorState>>,
    reloadables: Vec<Arc<dyn ReloadableErased>>,
    rx: mpsc::Receiver<ReloadRequest>,
    handle: ReloadCoordinatorHandle,
    audit_tx: tokio::sync::mpsc::UnboundedSender<ReloadAuditEvent>,
}

impl ReloadCoordinator {
    pub fn new(
        reloadables: Vec<Arc<dyn ReloadableErased>>,
        audit_tx: tokio::sync::mpsc::UnboundedSender<ReloadAuditEvent>,
    ) -> (Self, ReloadCoordinatorHandle) {
        let state = Arc::new(ArcSwap::new(Arc::new(CoordinatorState::Idle)));
        let (tx, rx) = mpsc::channel(64);
        let handle = ReloadCoordinatorHandle { state: state.clone(), tx };
        let coord = Self {
            state,
            reloadables,
            rx,
            handle: handle.clone(),
            audit_tx,
        };
        (coord, handle)
    }

    pub fn handle(&self) -> ReloadCoordinatorHandle {
        self.handle.clone()
    }

    pub fn state(&self) -> CoordinatorState {
        (**self.state.load()).clone()
    }

    /// Background task: drain the queue, run the state machine per request.
    pub async fn run(mut self) {
        while let Some(req) = self.rx.recv().await {
            // Degraded state: refuse further reloads until cleared via Plan-3 endpoint.
            if let CoordinatorState::Degraded(_) = **self.state.load() {
                warn!(?req, "ignoring reload request while coordinator is Degraded");
                let _ = self.audit_tx.send(ReloadAuditEvent::Refused {
                    source: req.source,
                    reason: "coordinator is in Degraded state".into(),
                });
                continue;
            }
            self.state.store(Arc::new(CoordinatorState::Reloading));
            // Plan 2 wires the actual TOML reload + DB section reload into the coordinator;
            // for now the test-only entry point passes a single delta directly.
            // Production callers convert TOML diffs into a Vec<RuntimeConfigDelta> and call
            // `run_cycle`.
            // Placeholder no-op so the task remains compilable until Plan 2.
            self.state.store(Arc::new(CoordinatorState::Idle));
            let _ = req;
        }
    }

    /// Test-only one-shot driver. Production code calls `run` and pushes through `handle()`.
    #[cfg(any(test, feature = "testing"))]
    pub fn new_for_test(reloadables: Vec<Arc<dyn ReloadableErased>>) -> Self {
        let (audit_tx, _audit_rx) = tokio::sync::mpsc::unbounded_channel();
        let state = Arc::new(ArcSwap::new(Arc::new(CoordinatorState::Idle)));
        let (tx, rx) = mpsc::channel(64);
        let handle = ReloadCoordinatorHandle { state: state.clone(), tx };
        Self { state, reloadables, rx, handle, audit_tx }
    }

    #[cfg(any(test, feature = "testing"))]
    pub async fn enqueue_and_drain(&self, delta: RuntimeConfigDelta) {
        let _ = self.run_cycle(vec![delta]).await;
    }

    /// Execute one validate/apply/watchdog/revert cycle for a single delta batch.
    ///
    /// Returns `Ok(per_subsystem_ms)` on commit; `Err(Report)` on revert. Phase helpers return
    /// owned values rather than mutating shared accumulators by reference — owning the
    /// applied-list + timing-map at each phase boundary avoids borrow-checker contortions and
    /// keeps each helper's signature self-contained.
    pub async fn run_cycle(
        &self,
        deltas: Vec<RuntimeConfigDelta>,
    ) -> Result<std::collections::BTreeMap<String, u64>, Report> {
        // Phase 1: validate-all.
        self.run_validate_phase(&deltas)?;

        // Phase 2: apply-sequenced. Returns the list of subsystems we successfully applied
        // (in apply order) and their per-subsystem apply timings.
        let (applied, mut per_subsystem_ms) = match self.run_apply_phase(&deltas).await {
            Ok(out) => out,
            Err((partial_applied, e)) => {
                self.run_revert_phase(&partial_applied).await;
                return Err(e);
            }
        };

        // Phase 3: watchdog (concurrent, FuturesUnordered). Augments the timing map with
        // health-check durations and returns it merged.
        match self.run_watchdog_phase(&applied, per_subsystem_ms).await {
            Ok(merged) => Ok(merged),
            Err(e) => {
                // Atomic revert-all rule (spec §8.3, §12).
                self.run_revert_phase(&applied).await;
                Err(e)
            }
        }
    }

    fn run_validate_phase(&self, deltas: &[RuntimeConfigDelta]) -> Result<(), Report> {
        for r in &self.reloadables {
            for d in deltas {
                r.validate(d)?;
            }
        }
        Ok(())
    }

    /// Apply each reloadable's delta in order. On error returns `(applied_so_far, error)` so
    /// the coordinator can revert just the subsystems that did apply.
    async fn run_apply_phase(
        &self,
        deltas: &[RuntimeConfigDelta],
    ) -> Result<
        (Vec<Arc<dyn ReloadableErased>>, std::collections::BTreeMap<String, u64>),
        (Vec<Arc<dyn ReloadableErased>>, Report),
    > {
        let mut applied: Vec<Arc<dyn ReloadableErased>> = Vec::new();
        let mut per_subsystem_ms = std::collections::BTreeMap::new();
        for r in &self.reloadables {
            for d in deltas {
                let start = std::time::Instant::now();
                if let Err(e) = r.apply(d).await {
                    return Err((applied, e));
                }
                let took = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                per_subsystem_ms.insert(r.name().to_string(), took);
                applied.push(r.clone());
            }
        }
        Ok((applied, per_subsystem_ms))
    }

    async fn run_watchdog_phase(
        &self,
        applied: &[Arc<dyn ReloadableErased>],
        mut per_subsystem_ms: std::collections::BTreeMap<String, u64>,
    ) -> Result<std::collections::BTreeMap<String, u64>, Report> {
        let mut unordered = FuturesUnordered::new();
        for r in applied {
            let r_cloned = r.clone();
            let window = r.rollback_window();
            unordered.push(async move {
                let start = std::time::Instant::now();
                let outcome = tokio::time::timeout(window, r_cloned.health_check()).await;
                (r_cloned.name(), outcome, start.elapsed())
            });
        }
        while let Some((name, outcome, elapsed)) = unordered.next().await {
            let took = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
            per_subsystem_ms.entry(name.to_string()).and_modify(|v| *v += took);
            match outcome {
                Ok(Ok(())) => continue,
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err(rootcause::report!("watchdog timed out for {name}")),
            }
        }
        Ok(per_subsystem_ms)
    }

    async fn run_revert_phase(&self, applied: &[Arc<dyn ReloadableErased>]) {
        for r in applied.iter().rev() {
            if let Err(e) = r.revert().await {
                error!(subsystem = r.name(), error = %e, "revert failed; entering Degraded");
                self.state.store(Arc::new(CoordinatorState::Degraded(DegradedInfo {
                    since: OffsetDateTime::now_utc(),
                    failed_subsystems: vec![r.name().to_string()],
                    reason: format!("revert returned Err on {}: {e}", r.name()),
                })));
            }
        }
    }
}
```

Add a minimal `audit.rs` placeholder to satisfy the import:

```rust
// crates/shared/config-reload/src/audit.rs
use crate::coordinator::ReloadSource;

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ReloadAuditEvent {
    Refused { source: ReloadSource, reason: String },
    Requested { source: ReloadSource },
    Applied { sections: Vec<String>, per_subsystem_ms: std::collections::BTreeMap<String, u64> },
    Failed { phase: crate::coordinator::ReloadPhase, subsystem: Option<String>, error: String },
    Reverted { subsystem: String, reason: String },
}
```

- [ ] **Step 4: Confirm tests pass**

```bash
cargo test -p uptrakit-config-reload --test coordinator -- happy_path
cargo test -p uptrakit-config-reload --test coordinator -- unhealthy_subsystem
```

Expected: 2 tests pass.

- [ ] **Step 5: Run clippy with `large_futures = deny`**

```bash
cargo clippy -p uptrakit-config-reload --all-targets -- -D warnings
```

Expected: no warnings. If any helper triggers `large_futures`, split further.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/config-reload/src/coordinator/state_machine.rs crates/shared/config-reload/src/audit.rs
git commit -m "feat(config-reload): coordinator state machine with atomic revert

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 12: Degraded entry on revert failure + clear-degraded API surface

**Files:**

- Modify: `crates/shared/config-reload/src/coordinator/state_machine.rs`

- [ ] **Step 1: Write failing test**

Add to `tests/coordinator.rs`:

```rust
#[derive(Default)]
struct RevertFailingReloadable;

#[async_trait]
impl ReloadableErased for RevertFailingReloadable {
    fn name(&self) -> &'static str { "rev_fail" }
    fn validate(&self, _: &RuntimeConfigDelta) -> Result<(), Report> { Ok(()) }
    async fn apply(&self, _: &RuntimeConfigDelta) -> Result<(), Report> { Ok(()) }
    async fn revert(&self) -> Result<(), Report> { Err(rootcause::report!("revert is broken")) }
    async fn health_check(&self) -> Result<(), Report> { Err(rootcause::report!("force revert")) }
    fn rollback_window(&self) -> Duration { Duration::from_millis(100) }
}

#[tokio::test(start_paused = true)]
async fn coordinator_enters_degraded_when_revert_fails() {
    let r = Arc::new(RevertFailingReloadable);
    let coord = ReloadCoordinator::new_for_test(vec![r]);
    coord.enqueue_and_drain(test_delta()).await;
    matches!(coord.state(), CoordinatorState::Degraded(_));
}

#[tokio::test(start_paused = true)]
async fn coordinator_refuses_reloads_in_degraded() {
    let r = Arc::new(RevertFailingReloadable);
    let coord = ReloadCoordinator::new_for_test(vec![r.clone()]);
    coord.enqueue_and_drain(test_delta()).await;
    let starting_state = coord.state();
    // Second enqueue must be ignored.
    coord.enqueue_and_drain(test_delta()).await;
    assert_eq!(coord.state(), starting_state);
}
```

- [ ] **Step 2: Confirm tests fail / pass**

```bash
cargo test -p uptrakit-config-reload --test coordinator -- degraded
```

Expected: first passes (already implemented in Task 11), second may fail because `enqueue_and_drain` bypasses the
queue-refusal check. Adjust `enqueue_and_drain` to call into the `run` loop's gating logic.

- [ ] **Step 3: Implement clear-degraded helper**

Append to `state_machine.rs`:

```rust
impl ReloadCoordinator {
    /// Re-run `health_check` on every Reloadable. If all succeed, return to Idle and accept
    /// further requests. If any fail, stay in Degraded with the updated failure list.
    pub async fn clear_degraded(&self) -> Result<(), Report> {
        let mut still_failing: Vec<String> = Vec::new();
        let mut last_err: Option<Report> = None;
        for r in &self.reloadables {
            if let Err(e) = r.health_check().await {
                still_failing.push(r.name().to_string());
                last_err = Some(e);
            }
        }
        if still_failing.is_empty() {
            self.state.store(Arc::new(CoordinatorState::Idle));
            info!("Degraded state cleared; coordinator returning to Idle");
            Ok(())
        } else {
            self.state.store(Arc::new(CoordinatorState::Degraded(DegradedInfo {
                since: OffsetDateTime::now_utc(),
                failed_subsystems: still_failing.clone(),
                reason: format!(
                    "clear-degraded failed: subsystems still unhealthy: {}",
                    still_failing.join(", ")
                ),
            })));
            Err(last_err.unwrap_or_else(|| rootcause::report!("subsystems still failing")))
        }
    }
}
```

- [ ] **Step 4: Add to handle**

Modify `crates/shared/config-reload/src/coordinator/mod.rs`:

```rust
impl ReloadCoordinatorHandle {
    /// Stub until Plan 3 wires the control channel. Returns `Err` so any premature integration
    /// fails loudly rather than silently no-op'ing — Plan 3 replaces this with a real
    /// `ControlMessage::ClearDegraded` dispatch through the coordinator's run loop.
    pub async fn clear_degraded(&self) -> Result<(), rootcause::Report> {
        Err(rootcause::report!(
            "clear_degraded is not yet wired (Plan 3 follow-up); coordinator stuck in Degraded \
             must be cleared via restart until that lands"
        ))
    }
}
```

Mark the Plan-3 follow-up explicitly so reviewers do not get surprised.

- [ ] **Step 5: Run tests**

```bash
cargo test -p uptrakit-config-reload --test coordinator -- degraded
```

Expected: both tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/config-reload/src/coordinator/
git commit -m "feat(config-reload): Degraded state entry + clear_degraded helper

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 13: SIGHUP trigger task

**Files:**

- Create: `crates/shared/config-reload/src/triggers/mod.rs`
- Create: `crates/shared/config-reload/src/triggers/sighup.rs`

- [ ] **Step 1: Write failing test**

Add to `tests/triggers.rs` (new file):

```rust
use tokio::sync::mpsc;
use uptrakit_config_reload::triggers::sighup::spawn_sighup_task;
use uptrakit_config_reload::ReloadRequest;

#[tokio::test]
async fn sighup_task_forwards_signal_to_channel() {
    let (tx, mut rx) = mpsc::channel::<ReloadRequest>(8);
    let task = spawn_sighup_task(tx);
    // Send SIGHUP to ourselves via nix (test-only)
    nix::sys::signal::raise(nix::sys::signal::Signal::SIGHUP).unwrap();
    let req = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await.unwrap();
    assert!(req.is_some());
    drop(task);
}
```

Add `nix = { workspace = true, features = ["signal"] }` to dev-dependencies.

- [ ] **Step 2: Confirm test fails**

```bash
cargo test -p uptrakit-config-reload --test triggers -- sighup
```

Expected: trigger module unresolved.

- [ ] **Step 3: Implement SIGHUP task**

Create `crates/shared/config-reload/src/triggers/mod.rs`:

```rust
pub mod file_watch;
pub mod sighup;
```

Create `crates/shared/config-reload/src/triggers/sighup.rs`:

```rust
use time::OffsetDateTime;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{error, info};

use crate::coordinator::{ReloadRequest, ReloadSource};

pub fn spawn_sighup_task(tx: mpsc::Sender<ReloadRequest>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut sig = match signal(SignalKind::hangup()) {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, "failed to register SIGHUP handler");
                return;
            }
        };
        while sig.recv().await.is_some() {
            info!("SIGHUP received; enqueueing reload request");
            let req = ReloadRequest { source: ReloadSource::Sighup, timestamp: OffsetDateTime::now_utc() };
            if let Err(e) = tx.send(req).await {
                error!(error = %e, "failed to forward SIGHUP to coordinator");
                break;
            }
        }
    })
}
```

- [ ] **Step 4: Confirm test passes**

```bash
cargo test -p uptrakit-config-reload --test triggers -- sighup -- --test-threads=1
```

`--test-threads=1` because raising SIGHUP affects the whole process. Expected: 1 test passes.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/config-reload/src/triggers/
git commit -m "feat(config-reload): SIGHUP trigger task

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 14: File-watch trigger via `notify-debouncer-full`

**Files:**

- Create: `crates/shared/config-reload/src/triggers/file_watch.rs`

- [ ] **Step 1: Write failing test**

Add to `tests/triggers.rs`:

```rust
use std::io::Write;
use tempfile::NamedTempFile;
use tokio::sync::mpsc;
use uptrakit_config_reload::triggers::file_watch::spawn_file_watch_task;
use uptrakit_config_reload::ReloadRequest;

#[tokio::test]
async fn file_watch_emits_request_after_atomic_rename() {
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "initial").unwrap();
    let path = f.path().to_path_buf();
    let (tx, mut rx) = mpsc::channel::<ReloadRequest>(8);
    let _handle = spawn_file_watch_task(path.clone(), tx);

    // Atomic rename — simulate editor save.
    let mut other = NamedTempFile::new_in(path.parent().unwrap()).unwrap();
    writeln!(other, "updated").unwrap();
    std::fs::rename(other.path(), &path).unwrap();

    // Wait for debounce (500ms) + slack.
    let req = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await.unwrap();
    assert!(req.is_some());
}
```

- [ ] **Step 2: Confirm test fails**

```bash
cargo test -p uptrakit-config-reload --test triggers -- file_watch
```

- [ ] **Step 3: Implement file-watch task**

Create `crates/shared/config-reload/src/triggers/file_watch.rs`:

```rust
use std::path::PathBuf;

use notify_debouncer_full::{new_debouncer, DebouncedEvent, FileIdMap};
use time::OffsetDateTime;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{error, info};

use crate::coordinator::{ReloadRequest, ReloadSource};
use crate::defaults::FILE_WATCH_DEBOUNCE;

pub fn spawn_file_watch_task(
    config_path: PathBuf,
    tx: mpsc::Sender<ReloadRequest>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<Result<Vec<DebouncedEvent>, _>>();
        let mut debouncer = match new_debouncer(FILE_WATCH_DEBOUNCE, None, move |events| {
            let _ = notify_tx.send(events);
        }) {
            Ok(d) => d,
            Err(e) => {
                error!(error = ?e, "failed to start file-watch debouncer");
                return;
            }
        };
        let watch_dir = match config_path.parent() {
            Some(p) => p.to_path_buf(),
            None => {
                error!("config path has no parent directory");
                return;
            }
        };
        if let Err(e) = debouncer.watch(&watch_dir, notify::RecursiveMode::NonRecursive) {
            error!(error = ?e, "failed to watch config directory");
            return;
        }
        info!(path = %config_path.display(), "file-watch task started");
        while let Some(batch) = notify_rx.recv().await {
            let events = match batch {
                Ok(e) => e,
                Err(e) => {
                    error!(error = ?e, "file-watch event batch error");
                    continue;
                }
            };
            let touched = events.iter().any(|ev| ev.paths.iter().any(|p| p == &config_path));
            if !touched {
                continue;
            }
            let req = ReloadRequest {
                source: ReloadSource::FileWatch { path: config_path.clone() },
                timestamp: OffsetDateTime::now_utc(),
            };
            if let Err(e) = tx.send(req).await {
                error!(error = %e, "failed to forward file-watch event to coordinator");
                break;
            }
        }
    })
}
```

- [ ] **Step 4: Confirm test passes**

```bash
cargo test -p uptrakit-config-reload --test triggers -- file_watch
```

Expected: 1 test passes.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/config-reload/src/triggers/file_watch.rs
git commit -m "feat(config-reload): file-watch trigger via notify-debouncer-full

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 15: `ConfigReconciler` skeleton with `ArcSwap` counter cache

**Files:**

- Create: `crates/shared/config-reload/src/reconciler.rs`

- [ ] **Step 1: Write failing test**

Add to `tests/coordinator.rs`:

```rust
use std::sync::Arc;
use uptrakit_config_reload::config::Scope;
use uptrakit_config_reload::reconciler::SettingsVersionCache;

#[test]
fn settings_version_cache_loads_and_swaps() {
    let cache = SettingsVersionCache::new();
    cache.update(Scope::Global, 1);
    cache.update(Scope::Global, 2);
    assert_eq!(cache.get(Scope::Global), Some(2));
    let tid = uuid::Uuid::new_v4();
    cache.update(Scope::Tenant(tid), 7);
    assert_eq!(cache.get(Scope::Tenant(tid)), Some(7));
}
```

- [ ] **Step 2: Confirm test fails**

```bash
cargo test -p uptrakit-config-reload --test coordinator -- settings_version
```

- [ ] **Step 3: Implement cache + reconciler skeleton**

Create `crates/shared/config-reload/src/reconciler.rs`:

```rust
use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::config::Scope;

/// Lock-free read-often / write-rarely counter cache. Replaces the per-poll DB read for the
/// `IfMatch` extractor's current-ETag lookup (Plan 3).
#[derive(Clone)]
pub struct SettingsVersionCache {
    inner: Arc<ArcSwap<HashMap<Scope, u64>>>,
}

impl SettingsVersionCache {
    pub fn new() -> Self {
        Self { inner: Arc::new(ArcSwap::new(Arc::new(HashMap::new()))) }
    }

    pub fn get(&self, scope: Scope) -> Option<u64> {
        self.inner.load().get(&scope).copied()
    }

    pub fn update(&self, scope: Scope, version: u64) {
        let mut next: HashMap<Scope, u64> = (**self.inner.load()).clone();
        next.insert(scope, version);
        self.inner.store(Arc::new(next));
    }
}

impl Default for SettingsVersionCache {
    fn default() -> Self { Self::new() }
}

// The full ConfigReconciler task (polling + DB read + RuntimeConfigDelta emission) lands in
// Plan 2, where DB-section reload becomes a real path. The cache itself is part of the
// foundation because the IfMatch extractor in Plan 3 also reads from it.
```

- [ ] **Step 4: Confirm test passes**

```bash
cargo test -p uptrakit-config-reload --test coordinator -- settings_version
```

- [ ] **Step 5: Commit**

```bash
git add crates/shared/config-reload/src/reconciler.rs
git commit -m "feat(config-reload): SettingsVersionCache via ArcSwap

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 16: Per-section watch channels in `RuntimeConfigChannels`

**Files:**

- Create: `crates/shared/config-reload/src/channels.rs`
- Modify: `crates/shared/config-reload/src/lib.rs`

The coordinator's Reloadables (Plan 2) own their own `watch::Sender` for the slice of `RuntimeConfig` they
control. But the receivers must reach consumers (handlers, background tasks, embedded services) that exist
**before** Plan 2 lands. Solution: at boot, `RuntimeConfigChannels` constructs one `watch::Sender<Arc<…>>` per
Config Section seeded with the loaded TOML values; receivers fan out into `AppState`. When Plan 2 lands, each
Reloadable replaces the boot-time sender with its own (or, simpler, the Reloadable adopts the existing sender via
a shared `Arc<watch::Sender<…>>` design — but holding a `Sender` per Reloadable is the simpler ownership story
and what Plan 2 already specifies).

For Plan 1, the channels are **read-only after boot** because no Reloadables exist yet to publish updates. The
`AppState` consumers see only the boot value through `.borrow().clone()`. Plan 2 wires updates.

- [ ] **Step 1: Define the channel bundle**

```rust
// crates/shared/config-reload/src/channels.rs
use std::sync::Arc;
use tokio::sync::watch;

use crate::config::{
    AuditConfig, DbConfig, EmbeddedServicesConfig, LogConfig, MasterKeyConfig, NatsConfig,
    NetworkConfig, RuntimeConfig, TlsConfig, ZeroconfConfig,
};

/// Per-section `watch::Sender` bundle. The coordinator's Reloadables (Plan 2) borrow these
/// senders for live updates; the matching receivers fan out into `AppState`.
pub struct RuntimeConfigChannels {
    pub db: watch::Sender<Arc<DbConfig>>,
    pub network: watch::Sender<Arc<NetworkConfig>>,
    pub nats: watch::Sender<Arc<NatsConfig>>,
    pub tls: watch::Sender<Arc<TlsConfig>>,
    pub audit: watch::Sender<Arc<AuditConfig>>,
    pub log: watch::Sender<Arc<LogConfig>>,
    pub master_key: watch::Sender<Arc<MasterKeyConfig>>,
    pub embedded_services: watch::Sender<Arc<EmbeddedServicesConfig>>,
    pub zeroconf: watch::Sender<Arc<ZeroconfConfig>>,
}

pub struct RuntimeConfigReceivers {
    pub db: watch::Receiver<Arc<DbConfig>>,
    pub network: watch::Receiver<Arc<NetworkConfig>>,
    pub nats: watch::Receiver<Arc<NatsConfig>>,
    pub tls: watch::Receiver<Arc<TlsConfig>>,
    pub audit: watch::Receiver<Arc<AuditConfig>>,
    pub log: watch::Receiver<Arc<LogConfig>>,
    pub master_key: watch::Receiver<Arc<MasterKeyConfig>>,
    pub embedded_services: watch::Receiver<Arc<EmbeddedServicesConfig>>,
    pub zeroconf: watch::Receiver<Arc<ZeroconfConfig>>,
}

impl RuntimeConfigChannels {
    /// Build the channel set from the boot TOML. Each channel is seeded with the loaded value.
    pub fn from_runtime(runtime: &RuntimeConfig) -> (Self, RuntimeConfigReceivers) {
        let (db_tx, db_rx) = watch::channel(Arc::new(runtime.db.clone()));
        let (net_tx, net_rx) = watch::channel(Arc::new(runtime.network.clone()));
        let (nats_tx, nats_rx) = watch::channel(Arc::new(runtime.nats.clone()));
        let (tls_tx, tls_rx) = watch::channel(Arc::new(runtime.tls.clone()));
        let (audit_tx, audit_rx) = watch::channel(Arc::new(runtime.audit.clone()));
        let (log_tx, log_rx) = watch::channel(Arc::new(runtime.log.clone()));
        let (mk_tx, mk_rx) = watch::channel(Arc::new(runtime.master_key.clone()));
        let (emb_tx, emb_rx) = watch::channel(Arc::new(runtime.embedded_services.clone()));
        let (zc_tx, zc_rx) = watch::channel(Arc::new(runtime.zeroconf.clone()));

        let senders = Self {
            db: db_tx, network: net_tx, nats: nats_tx, tls: tls_tx, audit: audit_tx,
            log: log_tx, master_key: mk_tx, embedded_services: emb_tx, zeroconf: zc_tx,
        };
        let receivers = RuntimeConfigReceivers {
            db: db_rx, network: net_rx, nats: nats_rx, tls: tls_rx, audit: audit_rx,
            log: log_rx, master_key: mk_rx, embedded_services: emb_rx, zeroconf: zc_rx,
        };
        (senders, receivers)
    }
}
```

- [ ] **Step 2:** Re-export from `lib.rs`: `pub use channels::{RuntimeConfigChannels, RuntimeConfigReceivers};`
- [ ] **Step 3: Unit test** in `tests/coordinator.rs` confirms that an initial receiver `.borrow()` returns the
      seeded value.
- [ ] **Step 4: Commit** — `feat(config-reload): RuntimeConfigChannels boot-seeded watch fan-out`

---

## Task 17: Wire foundation into `controller-runtime/startup`

**Files:**

- Modify: `crates/core/controller-runtime/src/startup/mod.rs`
- Modify: `crates/core/controller-runtime/src/startup/settings.rs`
- Modify: `crates/core/controller-runtime/Cargo.toml` (depend on `uptrakit-config-reload`)

- [ ] **Step 1: Add dep**

In `crates/core/controller-runtime/Cargo.toml`, add to `[dependencies]`:

```toml
uptrakit-config-reload = { workspace = true }
```

Add to the workspace `[workspace.dependencies]`:

```toml
uptrakit-config-reload = { path = "crates/shared/config-reload" }
```

- [ ] **Step 2: Add boot helper**

Add to `crates/core/controller-runtime/src/startup/mod.rs`:

```rust
use std::path::PathBuf;
use uptrakit_config_reload::{
    ReloadCoordinator, ReloadCoordinatorHandle, TomlConfigLoader, RuntimeConfig,
};

pub struct BootedConfig {
    pub runtime: RuntimeConfig,
    pub coordinator_handle: ReloadCoordinatorHandle,
}

pub async fn boot_config(config_path: PathBuf) -> Result<BootedConfig, rootcause::Report> {
    let loaded = TomlConfigLoader::load(&config_path)?;
    for w in &loaded.warnings {
        tracing::warn!("config: {w}");
    }
    // Seed per-section watch channels from the boot TOML. Receivers fan into AppState; senders
    // stay with the coordinator until Plan 2's Reloadables claim them.
    let (channels, receivers) = RuntimeConfigChannels::from_runtime(&loaded.config);
    // Audit channel — Plan 3 wires this to AuditEmitter; placeholder unbounded here.
    let (audit_tx, _audit_rx) = tokio::sync::mpsc::unbounded_channel();
    let settings_version_cache = uptrakit_config_reload::reconciler::SettingsVersionCache::new();
    let reloadables = Vec::new(); // Plan 2 populates this.
    let (coordinator, handle) = ReloadCoordinator::new(reloadables, audit_tx);
    tokio::spawn(coordinator.run());
    // spawn_sighup_task / spawn_file_watch_task already call tokio::spawn internally and
    // return a JoinHandle<()>. Do NOT wrap them in another tokio::spawn — store the handles
    // so the supervisor can join them on shutdown if needed.
    let _sighup = uptrakit_config_reload::triggers::sighup::spawn_sighup_task(handle.sender());
    let _watch = uptrakit_config_reload::triggers::file_watch::spawn_file_watch_task(
        config_path.clone(),
        handle.sender(),
    );
    Ok(BootedConfig {
        runtime: loaded.config,
        coordinator_handle: handle,
        settings_version_cache,
        channels,
        receivers,
    })
}
```

`BootedConfig` carries the cache + channel set alongside the handle:

```rust
pub struct BootedConfig {
    pub runtime: RuntimeConfig,
    pub coordinator_handle: ReloadCoordinatorHandle,
    pub settings_version_cache: SettingsVersionCache,
    /// Senders the coordinator hands off to Plan 2 Reloadables. Plan 1 keeps the senders alive
    /// (one per section) so receivers don't see channel-closed.
    pub channels: RuntimeConfigChannels,
    pub receivers: RuntimeConfigReceivers,
}
```

Expose `sender()` on `ReloadCoordinatorHandle` as a `pub fn sender(&self) -> mpsc::Sender<ReloadRequest>` accessor
that clones the inner `tx`. The raw field stays `pub(crate)` in the config-reload crate.

- [ ] **Step 3: Wire into `AppState`**

In `crates/ui/web-api/src/app_state.rs`, add the coordinator handle, the version cache, and one receiver per
Config Section:

```rust
pub coordinator_handle: ReloadCoordinatorHandle,
pub settings_version_cache: uptrakit_config_reload::reconciler::SettingsVersionCache,
// Per-section receivers — handlers + background tasks read via `.borrow().clone()` per access.
pub db_config_rx: watch::Receiver<Arc<DbConfig>>,
pub network_config_rx: watch::Receiver<Arc<NetworkConfig>>,
pub nats_config_rx: watch::Receiver<Arc<NatsConfig>>,
pub tls_config_rx: watch::Receiver<Arc<TlsConfig>>,
pub audit_config_rx: watch::Receiver<Arc<AuditConfig>>,
pub log_config_rx: watch::Receiver<Arc<LogConfig>>,
pub master_key_config_rx: watch::Receiver<Arc<MasterKeyConfig>>,
pub embedded_services_config_rx: watch::Receiver<Arc<EmbeddedServicesConfig>>,
pub zeroconf_config_rx: watch::Receiver<Arc<ZeroconfConfig>>,
```

`settings_version_cache` is required by Plan 3's `IfMatch<SettingsVersion>` extractor (spec §14.3). The
section receivers exist so Plan 2 has somewhere to plug each Reloadable's publish-side without touching
`AppState` mid-Plan-2. Add the corresponding setters on `AppStateBuilder`. Set everything during boot from
`BootedConfig`.

- [ ] **Step 4: Compile + workspace check**

```bash
cargo check --all-features
cargo check --no-default-features --features db-sqlite
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/core/controller-runtime/src/startup/ crates/ui/web-api/src/app_state.rs Cargo.toml
git commit -m "feat(controller-runtime): boot config reload coordinator at startup

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 18: CLI `--config` + `--check-config` wiring

**Files:**

- Modify: `crates/core/controller/src/main.rs`
- Modify: `crates/core/controller-standalone/src/main.rs`

- [ ] **Step 1: Add flag**

Both binaries use `clap`. Add (sample for `controller/src/main.rs`):

```rust
#[derive(Parser)]
struct Cli {
    /// Path to controller TOML config.
    #[arg(long, env = "UPTRAKIT_CONFIG", default_value = "/etc/uptrakit/controller.toml")]
    config: PathBuf,

    /// Validate the TOML file and exit. No DB / network probes.
    #[arg(long)]
    check_config: bool,
    // (existing flags remain for this plan; CLI shrink lands in Plan 3.)
}

fn main() -> Result<(), anyhow::Error> {
    let cli = Cli::parse();
    if cli.check_config {
        return match uptrakit_config_reload::TomlConfigLoader::validate_only(&cli.config) {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("config check failed: {e:#}");
                std::process::exit(1);
            }
        };
    }
    // existing boot path
    Ok(())
}
```

- [ ] **Step 2: Manual smoke test**

```bash
cargo run --bin uptrakit-controller -- --check-config /nonexistent.toml
```

Expected: non-zero exit, message references the missing file.

```bash
cat > /tmp/test.toml <<EOF
[db]
url = "sqlite://x"
pool_size = 16
acquire_timeout_ms = 5000

[master_key]
path = "/etc/uptrakit/master.key"

[network.https]
addr = "0.0.0.0:8443"
trusted_proxies = []
real_ip_header = "x-forwarded-for"
forwarded_client_cert_info_header = "x-fcc"
forwarded_client_cert_pem_header  = "x-fcc-pem"

[network.pki]
addr = "0.0.0.0:8444"

[tls]
cert_path = "x"
key_path  = "y"
sans      = []

[nats]
url = "nats://x"

[audit]
filter = "all"
retention_days = 90

[log]
path  = "x"
level = "info"

[zeroconf]
enabled = false
url      = ""
pki_addr = ""

[embedded_services]
agent = false
agent_ssh = false
mqtt = false
scheduler = false
EOF
cargo run --bin uptrakit-controller -- --check-config /tmp/test.toml
```

Expected: exit 0, no stderr.

- [ ] **Step 3: Commit**

```bash
git add crates/core/controller/src/main.rs crates/core/controller-standalone/src/main.rs
git commit -m "feat(controller): --config + --check-config CLI flags

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 19: Quality gates + final verification

- [ ] **Step 1: Workspace gates**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
cargo deny check
cargo test --no-default-features --features db-sqlite
cargo test --all-features
```

Expected: all green. Address any lint warnings inline; do NOT silence them via `#[allow]` (use
`#[expect(... reason = "...")]` only if there's a legitimate need; default is to fix the root cause).

- [ ] **Step 2: Markdown gates for any docs touched**

```bash
npx prettier --write docs/superpowers/plans/2026-05-12-graceful-reload-1-foundation.md
npx markdownlint --config .markdownlint.json docs/superpowers/plans/2026-05-12-graceful-reload-1-foundation.md
```

- [ ] **Step 3: PR**

Open a PR with title:

```text
feat(config-reload): foundation crate + coordinator + check-config
```

Body: reference spec sections §4–§9, §13; note that subsystem `Reloadable` impls land in Plan 2; mention that the
`spawn_settings_reload` removal moves to Plan 2 as well so the foundation PR does not break running controllers
mid-rollout.

## Self-review

- Spec §4 (Domain Language) — terms appear in code comments + audit-event payloads (covered).
- Spec §5 (Architecture) — coordinator + per-section watch + reconciler cache landed.
- Spec §6 (Configuration sources) — TOML loader + section structs landed; section assignment table moves to Plan 4
  doc updates.
- Spec §7 (Reload triggers) — SIGHUP + file-watch + reconciler skeleton landed.
- Spec §8 (Coordinator) — state machine, Degraded sink, atomic revert-all, `FuturesUnordered` for watchdog (all
  covered).
- Spec §9 (Reloadable trait) — both typed and `#[async_trait]`-erased variants landed.
- Spec §13 (multi-controller) — `SettingsVersionCache` landed; full reconciler task lands in Plan 2.
- Snapshot rules: `rootcause::Report`, `parking_lot::Mutex` (none yet — first usage in Plan 2 subsystems),
  `#[non_exhaustive]` on every public type, `Other(String)` on every wire-exposed enum, no `unwrap()` in production,
  `FuturesUnordered` over `join_all`, BEGIN IMMEDIATE deferred to Plan 2 (reconciler DB reads).
- No `unwrap()` in production code (all in `#[cfg(test)]` or `tests/`).
- Workspace lints: confirmed via Task 18 Step 1 clippy gate.
