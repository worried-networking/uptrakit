# Graceful Reload Leftovers — Items A + B Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire `ReloadCoordinator::run()` to call `run_cycle()` on file and DB triggers (Item A), and update
`ConfigFileState` live after each successful file reload (Item B).

**Architecture:** Item A adds a `ReexecHook` trait to `uptrakit-config-reload`,
new coordinator fields (`config_path`, `current_config`, `reexec_hook`), and delta-production helpers;
the full `run()` loop replaces the existing stub.
Item B removes the `_file_state_tx` no-op in `reload_audit_bridge`, adds SHA-256 digest computation, and handles the new
`FileChanged` and updated `Applied { source }` audit events.
Both items share `ReloadAuditEvent` changes (Task 3), so they are implemented together.

**Tech Stack:** Rust 2024 · `uptrakit-config-reload` (shared) · `uptrakit-controller-runtime` (core) · `tokio::sync::watch` ·
`sha2` (already in `controller-runtime`) · `arc_swap` (already in `config-reload`)

---

## File Map

| Action | Path                                                           |
| ------ | -------------------------------------------------------------- |
| Create | `crates/shared/config-reload/src/reexec_hook.rs`               |
| Modify | `crates/shared/config-reload/src/delta.rs`                     |
| Modify | `crates/shared/config-reload/src/audit.rs`                     |
| Modify | `crates/shared/config-reload/src/coordinator/state_machine.rs` |
| Modify | `crates/shared/config-reload/src/lib.rs`                       |
| Modify | `crates/ui/web-api-queries/src/reload/plugin_registry.rs`      |
| Modify | `crates/core/controller-runtime/src/reexec/listenfd.rs`        |
| Modify | `crates/core/controller-runtime/src/startup/mod.rs`            |
| Modify | `crates/core/controller-runtime/src/lib.rs`                    |
| Create | `crates/shared/config-reload/tests/coordinator_run.rs`         |
| Modify | `docs/development/coding-standards.md`                         |

---

### Task 1: `ReexecOutcome` enum + `ReexecHook` trait in `uptrakit-config-reload`

**Files:**

- Create: `crates/shared/config-reload/src/reexec_hook.rs`
- Modify: `crates/shared/config-reload/src/lib.rs`

- [ ] **Step 1: Create `reexec_hook.rs`**

```rust
// crates/shared/config-reload/src/reexec_hook.rs
//! Cross-crate hook for triggering a process reexec when irreversibly-bound
//! config keys change. Defined here so `uptrakit-config-reload` stays ignorant
//! of `controller-runtime`'s reexec internals.

use rootcause::Report;

use crate::config::RuntimeConfig;

/// Result of a reexec eligibility check.
///
/// `exec()` on success diverges and never returns, so this type is only
/// ever constructed on the two non-diverging paths.
///
/// Not `#[non_exhaustive]` — this is a closed two-variant result-like enum
/// whose only match site is `ControllerReexecHook::check_and_trigger`. Adding
/// `#[non_exhaustive]` to an enum in a shared crate forces every external
/// consumer to add a wildcard arm, which would make the exhaustive match in
/// `controller-runtime` fail to compile.
#[must_use]
pub enum ReexecOutcome {
    /// Reexec was attempted but `exec()` returned an error. The process is
    /// still running; the coordinator treats this as a reload failure.
    ExecFailed(Report),
    /// No irreversibly-bound key changed; proceed with in-process reload.
    NotNeeded,
}

/// Hook called by the coordinator before applying file-sourced deltas.
///
/// The implementation lives in `controller-runtime` and is registered at
/// startup via [`crate::coordinator::ReloadCoordinator::set_reexec_hook`].
/// This keeps the shared `uptrakit-config-reload` crate ignorant of
/// `triage::decide` and `perform_reexec`.
pub trait ReexecHook: Send + Sync {
    /// Inspect `prior` vs `new`; decide and perform reexec if needed.
    ///
    /// On a successful `exec()` the function diverges and never returns.
    /// Returns `ReexecOutcome::ExecFailed(err)` when `exec()` fails.
    /// Returns `ReexecOutcome::NotNeeded` when no irreversibly-bound key
    /// changed.
    ///
    /// **Pre-exec requirement**: flush any async log writers synchronously
    /// before calling `perform_reexec`, because the Tokio runtime is
    /// killed when the process image is replaced. Prefer a synchronous
    /// tracing writer for the controller binary.
    ///
    /// # Errors (via `ReexecOutcome::ExecFailed`)
    ///
    /// Wraps the OS error from `exec()` when the binary path is inaccessible
    /// or cleared `FD_CLOEXEC` failed.
    fn check_and_trigger(&self, prior: &RuntimeConfig, new: &RuntimeConfig) -> ReexecOutcome;
}
```

- [ ] **Step 2: Export from `lib.rs`**

Add a new module declaration and re-exports to `crates/shared/config-reload/src/lib.rs`:

```rust
pub mod reexec_hook;
```

Add to the `pub use` block:

```rust
pub use reexec_hook::{ReexecHook, ReexecOutcome};
```

- [ ] **Step 3: Run quality gates — should compile clean**

```bash
cargo check --no-default-features --features db-sqlite -p uptrakit-config-reload
cargo clippy --all-targets --no-default-features --features db-sqlite -p uptrakit-config-reload
```

Expected: no errors, no warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/shared/config-reload/src/reexec_hook.rs \
        crates/shared/config-reload/src/lib.rs
git commit -m "feat(config-reload): add ReexecHook trait and ReexecOutcome enum

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

### Task 2: `RuntimeConfigDelta::PluginsDbRefresh` + `variant_tag()` + plugin erased impl

**Files:**

- Modify: `crates/shared/config-reload/src/delta.rs`
- Modify: `crates/ui/web-api-queries/src/reload/plugin_registry.rs`

- [ ] **Step 1: Write failing test for `variant_tag`**

Add to the bottom of `crates/shared/config-reload/src/delta.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn variant_tags_are_unique() {
        let tags: HashSet<&str> = vec![
            RuntimeConfigDelta::Db(Arc::new(Default::default())).variant_tag(),
            RuntimeConfigDelta::Network(Arc::new(Default::default())).variant_tag(),
            RuntimeConfigDelta::Nats(Arc::new(Default::default())).variant_tag(),
            RuntimeConfigDelta::Tls(Arc::new(Default::default())).variant_tag(),
            RuntimeConfigDelta::Audit(Arc::new(Default::default())).variant_tag(),
            RuntimeConfigDelta::Zeroconf(Arc::new(Default::default())).variant_tag(),
            RuntimeConfigDelta::EmbeddedServices(Arc::new(Default::default())).variant_tag(),
            RuntimeConfigDelta::Plugins(Arc::new(Default::default())).variant_tag(),
            RuntimeConfigDelta::PluginsDbRefresh.variant_tag(),
        ]
        .into_iter()
        .collect();
        assert_eq!(tags.len(), 9, "every variant must have a unique tag");
    }
}
```

Run to verify it fails:

```bash
cargo test --no-default-features --features db-sqlite -p uptrakit-config-reload -- delta::tests 2>&1 | grep -E "FAILED|error"
```

Expected: compile error — `PluginsDbRefresh` doesn't exist yet, `variant_tag` doesn't exist.

- [ ] **Step 2: Add `PluginsDbRefresh` variant and `variant_tag()` to `delta.rs`**

Replace the existing `RuntimeConfigDelta` enum definition:

```rust
/// In-process delta carrying the new value for one config section.
///
/// Wire-incompatible by design: `RuntimeConfigDelta` is **never serialised**.
/// Each variant wraps the new section value in an [`Arc`] so that receivers
/// can clone cheaply without copying the entire section.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum RuntimeConfigDelta {
    /// New database connection and pool settings.
    Db(Arc<DbConfig>),
    /// New network listener settings (HTTPS + PKI).
    Network(Arc<NetworkConfig>),
    /// New NATS messaging server settings.
    Nats(Arc<NatsConfig>),
    /// New TLS certificate and key settings.
    Tls(Arc<TlsConfig>),
    /// New audit log settings.
    Audit(Arc<AuditConfig>),
    /// New zero-configuration auto-discovery settings.
    Zeroconf(Arc<ZeroconfConfig>),
    /// New embedded-services toggle settings.
    EmbeddedServices(Arc<EmbeddedServicesConfig>),
    /// Plugin settings reload signal (TOML-sourced; version counter bumped by
    /// `ConfigReconciler` on each plugin settings change).
    Plugins(Arc<PluginsConfig>),
    /// Signal `PluginsReloadable` to re-read plugin configuration from the DB.
    ///
    /// Unlike `Plugins(Arc<PluginsConfig>)`, this variant carries no config
    /// payload — the distinction is structural, eliminating the sentinel-value
    /// anti-pattern where callers passed `PluginsConfig::default()` as a trigger.
    PluginsDbRefresh,
}

impl RuntimeConfigDelta {
    /// Return a stable `&'static str` discriminant for this variant.
    ///
    /// Used by [`dedup_deltas`](crate::coordinator) to deduplicate lists by
    /// variant tag without requiring `PartialEq` on the payload types.
    #[must_use]
    pub fn variant_tag(&self) -> &'static str {
        match self {
            Self::Db(_) => "Db",
            Self::Network(_) => "Network",
            Self::Nats(_) => "Nats",
            Self::Tls(_) => "Tls",
            Self::Audit(_) => "Audit",
            Self::Zeroconf(_) => "Zeroconf",
            Self::EmbeddedServices(_) => "EmbeddedServices",
            Self::Plugins(_) => "Plugins",
            Self::PluginsDbRefresh => "PluginsDbRefresh",
        }
    }
}
```

- [ ] **Step 3: Run the test — should pass now**

```bash
cargo test --no-default-features --features db-sqlite -p uptrakit-config-reload -- delta::tests
```

Expected: `variant_tags_are_unique` passes.

- [ ] **Step 4: Update `PluginCatalogReloadable` erased impl**

In `crates/ui/web-api-queries/src/reload/plugin_registry.rs`, replace:

```rust
uptrakit_config_reload::reloadable_erased_impl!(
    PluginCatalogReloadable,
    RuntimeConfigDelta::Plugins
);
```

with a manual impl that handles both variants:

```rust
#[async_trait::async_trait]
impl uptrakit_config_reload::reloadable::ReloadableErased for PluginCatalogReloadable {
    fn name(&self) -> &'static str {
        <Self as uptrakit_config_reload::reloadable::Reloadable>::name(self)
    }

    fn validate(
        &self,
        delta: &RuntimeConfigDelta,
    ) -> Result<(), rootcause::Report> {
        if let RuntimeConfigDelta::Plugins(cfg) = delta {
            <Self as uptrakit_config_reload::reloadable::Reloadable>::validate(self, cfg)
        } else {
            Ok(())
        }
    }

    async fn apply(
        &self,
        delta: &RuntimeConfigDelta,
    ) -> Result<(), rootcause::Report> {
        match delta {
            RuntimeConfigDelta::Plugins(cfg) => {
                <Self as uptrakit_config_reload::reloadable::Reloadable>::apply(
                    self,
                    cfg.clone(),
                )
                .await
            }
            RuntimeConfigDelta::PluginsDbRefresh => {
                // V1: log and no-op. The catalog re-reads plugin config from
                // DB on the next request via AppState. A follow-up task
                // wires a DB query here to propagate changes immediately.
                tracing::info!("plugin catalog DB refresh signal received (V1: no-op)");
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn revert(&self) -> Result<(), rootcause::Report> {
        <Self as uptrakit_config_reload::reloadable::Reloadable>::revert(self).await
    }

    async fn health_check(&self) -> Result<(), rootcause::Report> {
        <Self as uptrakit_config_reload::reloadable::Reloadable>::health_check(self).await
    }

    fn rollback_window(&self) -> std::time::Duration {
        <Self as uptrakit_config_reload::reloadable::Reloadable>::rollback_window(self)
    }
}
```

The impl block already uses `#[async_trait::async_trait]` as an attribute — no bare `use async_trait;` import is needed.
Check if `async-trait` is in `web-api-queries/Cargo.toml`; if not, add:

```toml
async-trait = { workspace = true }
```

- [ ] **Step 5: Run quality gates**

```bash
cargo check --all-features -p uptrakit-web-api-queries
cargo clippy --all-targets --all-features -p uptrakit-web-api-queries
cargo test --all-features -p uptrakit-web-api-queries
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/config-reload/src/delta.rs \
        crates/ui/web-api-queries/src/reload/plugin_registry.rs
git commit -m "feat(config-reload): add PluginsDbRefresh delta variant and variant_tag()

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

### Task 3: `ReloadAuditEvent` additions + bridge compile fix

**Files:**

- Modify: `crates/shared/config-reload/src/audit.rs`
- Modify: `crates/core/controller-runtime/src/lib.rs`

- [ ] **Step 1: Update `audit.rs`**

Replace the entire file content:

```rust
//! Audit events emitted by the reload coordinator.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::coordinator::{ReloadPhase, ReloadSource};

/// An event emitted by the [`crate::coordinator::ReloadCoordinator`] during a
/// reload lifecycle.
///
/// Consumers (e.g. the audit-log subsystem) receive these events on an
/// unbounded channel and persist or forward them as needed.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ReloadAuditEvent {
    /// A reload request was received but rejected (e.g. coordinator Degraded).
    Refused {
        /// The source that submitted the refused request.
        source: ReloadSource,
        /// Human-readable reason for rejection.
        reason: String,
    },
    /// A reload request was accepted and is being processed.
    Requested {
        /// The source that submitted the request.
        source: ReloadSource,
    },
    /// The coordinator loaded a new TOML file and is about to apply it.
    ///
    /// Emitted between `Requested` and `Applied`/`Failed`. The bridge uses
    /// this to set `ConfigFileState::pending_digest` / `pending_detected_at`.
    /// The coordinator emits the path rather than the digest; the bridge
    /// computes the digest (via `sha2`) when it receives this event.
    FileChanged {
        /// Path of the newly-loaded TOML file.
        path: PathBuf,
    },
    /// The reload cycle completed successfully.
    Applied {
        /// Which config sections were changed.
        sections: Vec<String>,
        /// Wall-clock milliseconds spent per subsystem (apply + health-check).
        per_subsystem_ms: BTreeMap<String, u64>,
        /// The source that triggered this reload cycle.
        ///
        /// Used by `reload_audit_bridge` to decide whether to re-read the
        /// config file and update `ConfigFileState`.
        source: ReloadSource,
    },
    /// The reload cycle failed at the given phase.
    Failed {
        /// The coordinator phase in which the failure occurred.
        phase: ReloadPhase,
        /// The subsystem that failed, if failure was subsystem-specific.
        subsystem: Option<String>,
        /// Human-readable description of the error.
        error: String,
    },
    /// A subsystem was reverted after a failed apply.
    Reverted {
        /// The subsystem that was reverted.
        subsystem: String,
        /// Human-readable reason the revert was triggered.
        reason: String,
    },
}
```

- [ ] **Step 2: Fix `Applied` match arms in `reload_audit_bridge` in `lib.rs`**

The `Applied` variant now has a `source` field. Find the match arm in `reload_audit_bridge` (around line 1303) that looks like:

```rust
ReloadAuditEvent::Applied {
    sections,
    per_subsystem_ms,
} => {
```

Add `source` to the destructure pattern (use `..` or bind it):

```rust
ReloadAuditEvent::Applied {
    sections,
    per_subsystem_ms,
    source: _,
} => {
```

Similarly, the audit-log JSON arm (around line 1379):

```rust
ReloadAuditEvent::Applied {
    sections,
    per_subsystem_ms,
} => (
```

becomes:

```rust
ReloadAuditEvent::Applied {
    sections,
    per_subsystem_ms,
    source: _,
} => (
```

We use `source: _` for now; Task 9 changes it to `source` for real handling.

- [ ] **Step 3: Run quality gates**

```bash
cargo check --all-features
cargo clippy --all-targets --all-features
```

Expected: compiles clean. The `FileChanged` arm is already covered by `_ => {}` / `_ => continue` in the bridge.

- [ ] **Step 4: Commit**

```bash
git add crates/shared/config-reload/src/audit.rs \
        crates/core/controller-runtime/src/lib.rs
git commit -m "feat(config-reload): add FileChanged audit event and source field to Applied

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

### Task 4: Coordinator new fields + builder methods

**Files:**

- Modify: `crates/shared/config-reload/src/coordinator/state_machine.rs`

- [ ] **Step 1: Add imports to `state_machine.rs`**

At the top of the file, add these `use` lines (after existing imports):

```rust
use std::path::PathBuf;

use crate::config::RuntimeConfig;
use crate::coordinator::ReloadSource;
use crate::loader::TomlConfigLoader;
use crate::reexec_hook::{ReexecHook, ReexecOutcome};
```

Also add `use std::collections::HashSet;` (needed by `dedup_deltas` in Task 5).

- [ ] **Step 2: Add three fields to `ReloadCoordinator`**

In the `ReloadCoordinator` struct definition, add after the existing `alert_writer` field:

```rust
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
```

- [ ] **Step 3: Update `new()` and `new_for_test()` to initialise the new fields**

In `ReloadCoordinator::new()`, inside the `Self { ... }` struct literal, add:

```rust
        config_path: None,
        current_config: Arc::new(RuntimeConfig::default()),
        reexec_hook: None,
```

In `ReloadCoordinator::new_for_test()`, inside the `Self { ... }` struct literal, add the same three fields with the same values.

- [ ] **Step 4: Add builder methods**

Add these three methods inside the `impl ReloadCoordinator` block, after `set_alert_writer`:

```rust
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
```

- [ ] **Step 5: Run quality gates**

```bash
cargo check --no-default-features --features db-sqlite -p uptrakit-config-reload
cargo clippy --all-targets --no-default-features --features db-sqlite -p uptrakit-config-reload
cargo test --no-default-features --features db-sqlite -p uptrakit-config-reload
```

Expected: all pass. The existing coordinator tests still compile and pass.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/config-reload/src/coordinator/state_machine.rs
git commit -m "feat(config-reload): add config_path, current_config, reexec_hook fields to ReloadCoordinator

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

### Task 5: Delta production helpers

**Files:**

- Modify: `crates/shared/config-reload/src/coordinator/state_machine.rs`

Add these three free functions at the bottom of `state_machine.rs` (before the `#[cfg(test)]` block):

- [ ] **Step 1: Write tests for the helpers**

```rust
#[cfg(test)]
mod delta_helper_tests {
    use super::*;
    use crate::config::RuntimeConfig;

    fn base() -> RuntimeConfig {
        RuntimeConfig::default()
    }

    #[test]
    fn build_deltas_empty_on_identical_configs() {
        let c = base();
        assert!(build_deltas(&c, &c).is_empty());
    }

    #[test]
    fn build_deltas_detects_audit_change() {
        let prior = base();
        let mut new = base();
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
        // Last-wins: one entry remains (both payloads identical here, so
        // only the count matters).
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn sections_to_deltas_maps_audit() {
        let c = base();
        let sections = vec!["audit".to_string()];
        let deltas = sections_to_deltas(&sections, &c);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].variant_tag(), "Audit");
    }

    #[test]
    fn sections_to_deltas_maps_plugins() {
        let c = base();
        let sections = vec!["plugins".to_string()];
        let deltas = sections_to_deltas(&sections, &c);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].variant_tag(), "PluginsDbRefresh");
    }

    #[test]
    fn sections_to_deltas_deduplicates() {
        let c = base();
        let sections = vec!["audit".to_string(), "audit_log".to_string()];
        let deltas = sections_to_deltas(&sections, &c);
        assert_eq!(deltas.len(), 1);
    }
}
```

Run to verify they fail (functions don't exist yet):

```bash
cargo test --no-default-features --features db-sqlite -p uptrakit-config-reload -- delta_helper_tests 2>&1 | grep -E "FAILED|error\[" | head -5
```

Expected: compile error.

- [ ] **Step 2: Add the helpers to `state_machine.rs`**

Add these functions before the `#[cfg(test)]` block. Note: `RuntimeConfig` already derives `PartialEq` (confirmed in codebase), so no derive changes needed.

```rust
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
```

- [ ] **Step 3: Run tests — should pass**

```bash
cargo test --no-default-features --features db-sqlite -p uptrakit-config-reload -- delta_helper_tests
```

Expected: all 6 tests pass.

- [ ] **Step 4: Run full quality gates**

```bash
cargo check --all-features
cargo clippy --all-targets --all-features
```

- [ ] **Step 5: Commit**

```bash
git add crates/shared/config-reload/src/coordinator/state_machine.rs
git commit -m "feat(config-reload): add build_deltas, sections_to_deltas, dedup_deltas helpers

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

### Task 6: Full `run()` loop + `process_request()`

**Files:**

- Modify: `crates/shared/config-reload/src/coordinator/state_machine.rs`

- [ ] **Step 1: Write failing integration test**

In `crates/shared/config-reload/tests/coordinator_run.rs` (create if absent):

```rust
//! Integration tests for the `ReloadCoordinator::run()` loop.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use uptrakit_config_reload::coordinator::{
    CoordinatorState, ReloadRequest, ReloadSource,
};
use uptrakit_config_reload::delta::RuntimeConfigDelta;
use uptrakit_config_reload::{ReloadAuditEvent, ReloadCoordinator};

// Minimal no-op reloadable that records apply calls.
struct CountingReloadable {
    count: Arc<std::sync::atomic::AtomicU32>,
}

#[async_trait::async_trait]
impl uptrakit_config_reload::reloadable::ReloadableErased for CountingReloadable {
    fn name(&self) -> &'static str { "counter" }
    fn validate(&self, _: &RuntimeConfigDelta) -> Result<(), rootcause::Report> { Ok(()) }
    async fn apply(&self, delta: &RuntimeConfigDelta) -> Result<(), rootcause::Report> {
        if matches!(delta, RuntimeConfigDelta::Audit(_)) {
            self.count.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
    async fn revert(&self) -> Result<(), rootcause::Report> { Ok(()) }
    async fn health_check(&self) -> Result<(), rootcause::Report> { Ok(()) }
    fn rollback_window(&self) -> std::time::Duration { std::time::Duration::from_secs(1) }
}

#[tokio::test(flavor = "current_thread")]
async fn run_loop_routes_db_bump_to_run_cycle() {
    let count = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let reloadable = Arc::new(CountingReloadable { count: Arc::clone(&count) });
    let mut coord = ReloadCoordinator::new_for_test(vec![reloadable]);
    let handle = coord.handle();
    let _task = tokio::spawn(coord.run());

    handle
        .sender()
        .send(ReloadRequest {
            source: ReloadSource::DbBump {
                scope: uptrakit_config_reload::Scope::Global,
                sections: vec!["audit".to_string()],
            },
        })
        .await
        .unwrap();

    // Yield to let the coordinator task process the message.
    // Never use tokio::time::sleep — yield_now is deterministic and avoids
    // wall-clock coupling (testing.md: "never sleep on wall-clock").
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert_eq!(count.load(Ordering::SeqCst), 1, "apply should have been called once");
}
```

Run:

```bash
cargo test --no-default-features --features db-sqlite -p uptrakit-config-reload -- coordinator_run 2>&1 | tail -5
```

Expected: fails (coordinator `run()` is still the stub — it never calls `run_cycle`).

- [ ] **Step 2: Replace stub `run()` with full implementation**

Locate the stub `run()` method in `state_machine.rs`. The current stub body is:

```rust
pub async fn run(mut self) {
    while let Some(req) = self.rx.recv().await {
        if let CoordinatorState::Degraded(_) = **self.state.load() {
            warn!( ... );
            if self.audit_tx.send(...).is_err() { warn!(...); }
            continue;
        }
        self.state.store(Arc::new(CoordinatorState::Reloading));
        // Plan 2 produces actual deltas ...
        self.state.store(Arc::new(CoordinatorState::Idle));
    }
}
```

Replace the entire `run()` method body with:

```rust
    /// Drive the coordinator loop until the sender side of the channel is dropped.
    pub async fn run(mut self) {
        while let Some(req) = self.rx.recv().await {
            if let CoordinatorState::Degraded(_) = **self.state.load() {
                warn!(source = ?req.source, "ignoring reload request while Degraded");
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

            // Clone source before moving `req` into process_request.
            let source = req.source.clone();
            self.state.store(Arc::new(CoordinatorState::Reloading));
            if self
                .audit_tx
                .send(ReloadAuditEvent::Requested { source: source.clone() })
                .is_err()
            {
                warn!("audit channel closed; Requested event dropped");
            }

            let outcome = self.process_request(req).await;

            match outcome {
                Ok(per_ms) => {
                    self.state.store(Arc::new(CoordinatorState::Idle));
                    if self
                        .audit_tx
                        .send(ReloadAuditEvent::Applied {
                            sections: per_ms.keys().cloned().collect(),
                            per_subsystem_ms: per_ms,
                            source,
                        })
                        .is_err()
                    {
                        warn!("audit channel closed; Applied event dropped");
                    }
                }
                Err(e) => {
                    // revert_phase sets Degraded if the revert itself fails;
                    // otherwise we return to Idle.
                    if !matches!(**self.state.load(), CoordinatorState::Degraded(_)) {
                        self.state.store(Arc::new(CoordinatorState::Idle));
                    }
                    if self
                        .audit_tx
                        .send(ReloadAuditEvent::Failed {
                            phase: crate::coordinator::ReloadPhase::Apply,
                            subsystem: None,
                            error: e.to_string(),
                        })
                        .is_err()
                    {
                        warn!("audit channel closed; Failed event dropped");
                    }
                }
            }
        }
    }
```

- [ ] **Step 3: Add `process_request()` method**

Add this private method inside `impl ReloadCoordinator`, directly after `run()`:

```rust
    /// Process one reload request: load/diff/triage for file triggers;
    /// map sections to deltas for DB bumps.
    ///
    /// Extracted into its own `async fn` to satisfy `clippy::large_futures`.
    ///
    /// # Errors
    ///
    /// Returns an error if TOML loading fails, the reexec hook returns
    /// `ExecFailed`, or `run_cycle` fails.
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
                            "coordinator has no config_path set; cannot reload from file"
                        ));
                    }
                };

                let loaded = TomlConfigLoader::load(&config_path)?;
                for w in &loaded.warnings {
                    tracing::warn!("config reload: {w}");
                }
                let new_config = loaded.config;

                // Emit FileChanged so the audit bridge sets pending_digest.
                // The bridge computes the SHA-256 digest from the path; the
                // coordinator does not hash the file itself.
                if self
                    .audit_tx
                    .send(ReloadAuditEvent::FileChanged {
                        path: config_path.clone(),
                    })
                    .is_err()
                {
                    warn!("audit channel closed; FileChanged event dropped");
                }

                let prior = Arc::clone(&self.current_config);

                // Check for irreversibly-bound key changes via the hook.
                if let Some(hook) = &self.reexec_hook {
                    match hook.check_and_trigger(&prior, &new_config) {
                        // exec() replaced the process image; unreachable.
                        ReexecOutcome::NotNeeded => {}
                        ReexecOutcome::ExecFailed(err) => return Err(err),
                    }
                }

                let deltas = build_deltas(&prior, &new_config);
                if deltas.is_empty() {
                    info!("file reload: no section changes detected; no-op");
                    return Ok(BTreeMap::new());
                }

                let per_ms = self.run_cycle(deltas).await?;

                // Update current config only after successful apply.
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
                // Boot is handled at startup outside the coordinator loop.
                // Other is a forward-compat catch-all.
                tracing::debug!(source = ?req.source, "coordinator: ignoring non-actionable source");
                Ok(BTreeMap::new())
            }
        }
    }
```

- [ ] **Step 4: Run the test — should pass now**

```bash
cargo test --no-default-features --features db-sqlite -p uptrakit-config-reload -- coordinator_run
```

Expected: `run_loop_routes_db_bump_to_run_cycle` passes.

- [ ] **Step 5: Run full quality gates**

```bash
cargo fmt --all
cargo check --all-features
cargo clippy --all-targets --all-features
cargo test --all-features
```

- [ ] **Step 6: Commit**

```bash
git add crates/shared/config-reload/src/coordinator/state_machine.rs \
        crates/shared/config-reload/tests/coordinator_run.rs
git commit -m "feat(config-reload): implement ReloadCoordinator run() loop with process_request()

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

### Task 7: `BootedConfig::runtime_arc` + SHA-256 boot digest

**Files:**

- Modify: `crates/core/controller-runtime/src/startup/mod.rs`

- [ ] **Step 1: Add `runtime_arc` field to `BootedConfig`**

In `startup/mod.rs`, locate the `BootedConfig` struct. Add a new field after `runtime`:

```rust
    /// `Arc`-wrapped clone of `runtime` for seeding the coordinator's
    /// `current_config` field without cloning the whole struct again.
    pub runtime_arc: std::sync::Arc<uptrakit_config_reload::RuntimeConfig>,
```

- [ ] **Step 2: Add `file_digest()` helper**

Add this free function at the bottom of `startup/mod.rs` (before the `PkiRuntime` struct or at end of file):

```rust
/// Compute a `"sha256:<hex>"` digest of the file at `path`.
///
/// Falls back to `"size:<N>"` on I/O error so that the status endpoint
/// always has a value rather than returning an empty string.
pub(crate) fn file_digest(path: &std::path::Path) -> String {
    use sha2::{Digest as _, Sha256};
    match std::fs::read(path) {
        Ok(bytes) => {
            let mut h = Sha256::new();
            h.update(&bytes);
            format!("sha256:{:x}", h.finalize())
        }
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "could not read config for digest; using size stub"
            );
            format!("size:{}", path.metadata().map(|m| m.len()).unwrap_or(0))
        }
    }
}
```

Verify that `sha2` is already in `controller-runtime/Cargo.toml`:

```bash
grep 'sha2' crates/core/controller-runtime/Cargo.toml
```

If absent, add to `[dependencies]`:

```toml
sha2 = { workspace = true }
```

Check workspace has `sha2`:

```bash
grep 'sha2' Cargo.toml
```

If not in workspace, add:

```toml
sha2 = "0.10"
```

- [ ] **Step 3: Update `boot_config()` to use SHA-256 and populate `runtime_arc`**

In `boot_config()`, locate:

```rust
    let file_bytes = std::fs::read(&config_path).unwrap_or_else(|e| { ... });
    let digest = format!("size:{}", file_bytes.len());
```

Replace with:

```rust
    let digest = file_digest(&config_path);
```

Also remove the `file_bytes` binding since it's no longer needed.

At the `Ok(BootedConfig { ... })` return, add the new field:

```rust
        runtime_arc: std::sync::Arc::new(loaded.config.clone()),
```

(Note: `loaded.config` is moved into `runtime` field; add the clone BEFORE the struct literal,
e.g. `let runtime_arc = std::sync::Arc::new(loaded.config.clone());` then use `runtime_arc` in the struct.)

Exact change: before `Ok(BootedConfig {`, add:

```rust
    let runtime_arc = std::sync::Arc::new(loaded.config.clone());
```

Then in the struct literal:

```rust
        runtime: loaded.config,
        runtime_arc,
```

- [ ] **Step 4: Run quality gates**

```bash
cargo check --all-features -p uptrakit-controller-runtime
cargo clippy --all-targets --all-features -p uptrakit-controller-runtime
```

- [ ] **Step 5: Commit**

```bash
git add crates/core/controller-runtime/src/startup/mod.rs
git commit -m "feat(controller-runtime): add runtime_arc to BootedConfig, upgrade boot digest to SHA-256

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

### Task 8: `ControllerReexecHook` + listener pre-bind + wiring

**Files:**

- Modify: `crates/core/controller-runtime/src/reexec/listenfd.rs`
- Modify: `crates/core/controller-runtime/src/lib.rs`

- [ ] **Step 1: Add `current_generation()` to `listenfd.rs`**

Add at the bottom of `crates/core/controller-runtime/src/reexec/listenfd.rs`:

```rust
/// Return the current process reexec generation.
///
/// Reads `UPTRAKIT_REEXEC_GENERATION` set by the previous generation's
/// `perform_reexec`. Returns `0` on cold start (env var absent or unparseable).
pub(crate) fn current_generation() -> u64 {
    std::env::var("UPTRAKIT_REEXEC_GENERATION")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}
```

- [ ] **Step 2: Add `ControllerReexecHook` struct to `lib.rs`**

Near the top of `lib.rs` (after the existing `use` imports), add:

```rust
use std::os::unix::io::AsRawFd as _;
use uptrakit_config_reload::{ReexecHook, ReexecOutcome};
```

Somewhere before `run_server()` (e.g. just above the `/// Bridge task:` comment), add the struct:

```rust
/// Reexec hook implementation for the controller process.
///
/// Captures listener FDs and exec arguments at startup so that
/// [`perform_reexec`] can reconstruct the command line without re-reading
/// the process environment after a signal.
struct ControllerReexecHook {
    /// Resolved from `std::env::current_exe()` at startup.
    current_exe: std::path::PathBuf,
    config_path: std::path::PathBuf,
    master_key_file: Option<String>,
    generation: u64,
    /// Raw listener FDs cleared of `FD_CLOEXEC` before `exec()`.
    /// Empty when PKI HTTP is disabled; the child re-binds in that case.
    listener_fds: Vec<std::os::unix::io::RawFd>,
}

impl ReexecHook for ControllerReexecHook {
    fn check_and_trigger(
        &self,
        prior: &uptrakit_config_reload::RuntimeConfig,
        new: &uptrakit_config_reload::RuntimeConfig,
    ) -> ReexecOutcome {
        let decision = reexec::triage::decide(prior, new);
        if !decision.needed {
            return ReexecOutcome::NotNeeded;
        }
        tracing::info!(reasons = ?decision.reasons, "reexec required by config change");

        let plan = reexec::ReexecPlan {
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

- [ ] **Step 3: Pre-bind HTTPS socket in `run_server()` before spawning the server task**

In `run_server()`, find the section that assigns `server_handle` and spawns the server task (around line 976-985):

```rust
let server_handle = axum_server::Handle::new();
let server_options = server::ServerOptions {
    https_addr: reconciled.https_addr,
    ...
    inherited_listener: inherited_https,
};
let server_task = tokio::spawn(server::run(server_options));
```

Replace with pre-bind logic:

```rust
let server_handle = axum_server::Handle::new();

// Pre-bind HTTPS socket so we have the raw FD for reexec listener inheritance.
// On the inherited path (reexec ≥ gen 1) the socket is already bound.
let https_std = match inherited_https {
    Some(l) => l,
    None => {
        let l = std::net::TcpListener::bind(reconciled.https_addr).map_err(|e| {
            report!(AppError::Config(format!(
                "bind HTTPS {}: {e}",
                reconciled.https_addr
            )))
        })?;
        // Required: axum-server 0.8 does not call set_nonblocking() internally
        // for inherited listeners. Matches the behaviour in server::run() for
        // the cold-start bind path.
        l.set_nonblocking(true).map_err(|e| {
            report!(AppError::Config(format!("set_nonblocking HTTPS: {e}")))
        })?;
        l
    }
};
let https_raw_fd = https_std.as_raw_fd();

let server_options = server::ServerOptions {
    https_addr: reconciled.https_addr,
    rustls_config,
    app_state: Arc::clone(&app_state),
    static_dir: validated.static_dir,
    handle: server_handle.clone(),
    inherited_listener: Some(https_std), // always Some now
};
let server_task = tokio::spawn(server::run(server_options));
```

- [ ] **Step 4: Pre-bind PKI socket (when configured) and collect listener FDs**

Find `spawn_pki_http(...)`. Before that call, add:

```rust
// Pre-bind PKI socket when configured so its FD can be inherited on reexec.
// When PKI is disabled, listener_fds contains only the HTTPS FD and the
// child process re-binds PKI — LISTEN_FDS=0 signals cold-start to the child.
let (listener_fds, pki_std_for_spawn): (Vec<std::os::unix::io::RawFd>, Option<std::net::TcpListener>) =
    if let Some(pki_port) = validated.pki_http_port {
        let pki_std = match inherited_pki {
            Some(l) => l,
            None => {
                let addr = std::net::SocketAddr::from(([0, 0, 0, 0], pki_port));
                let l = std::net::TcpListener::bind(addr).map_err(|e| {
                    report!(AppError::Config(format!("bind PKI HTTP {addr}: {e}")))
                })?;
                l.set_nonblocking(true).map_err(|e| {
                    report!(AppError::Config(format!("set_nonblocking PKI: {e}")))
                })?;
                l
            }
        };
        let pki_fd = pki_std.as_raw_fd();
        (vec![https_raw_fd, pki_fd], Some(pki_std))
    } else {
        // No PKI — LISTEN_FDS=0; child re-binds all sockets on reexec.
        (vec![], None)
    };
```

Update the `spawn_pki_http` call to use `pki_std_for_spawn` instead of `inherited_pki`:

```rust
spawn_pki_http(&mut bg, &app_state, validated.pki_http_port, pki_std_for_spawn);
```

Note: `spawn_pki_http` needs to accept `Option<std::net::TcpListener>` — which it already does. No signature change needed.

- [ ] **Step 5: Preserve `config_path` before `boot_config()` consumes it**

`boot_config(config_path: PathBuf)` takes `config_path` by value. Find the call in `run_server()`:

```rust
// Before (config_path is consumed here):
let booted = startup::boot_config(config_path).await?;
```

Change to clone before the call so coordinator and bridge can use it:

```rust
// After — clone before consuming:
let config_path_for_coord = config_path.clone();
let booted = startup::boot_config(config_path).await?;
```

Then use `config_path_for_coord` everywhere below (coordinator wiring, audit bridge call site in Task 9).

- [ ] **Step 6: Wire coordinator builder methods in `run_server()`**

After building the coordinator but BEFORE `tokio::spawn(b.coordinator.run())`, and AFTER the server bind steps above, add:

```rust
        let current_exe = std::env::current_exe().map_err(|e| {
            report!(AppError::Config(format!("resolve current_exe: {e}")))
        })?;
        b.coordinator.set_config_path(config_path_for_coord.clone());
        b.coordinator.set_current_config(Arc::clone(&b.runtime_arc));
        // Box not Arc: coordinator's run() loop is the sole owner of this hook.
        b.coordinator.set_reexec_hook(Box::new(ControllerReexecHook {
            current_exe,
            config_path: config_path_for_coord.clone(),
            master_key_file: args.master_key_from.clone(),
            generation: reexec::listenfd::current_generation(),
            listener_fds,
        }));
```

You need to confirm:

- `config_path_for_coord: PathBuf` is in scope here — it was cloned in Step 5 before `boot_config()` consumed the original.
- `args.master_key_from: Option<String>` — check the `Args` struct for the field name; adjust if different.
- `b.runtime_arc` — populated in Task 7.
- The coordinator setter calls happen inside the `{ let mut b = booted; ... }` block.

- [ ] **Step 7: Remove `dead_code` expects from reexec items**

In `crates/core/controller-runtime/src/reexec/mod.rs`, remove the `#[expect(dead_code, ...)]` attributes from `ReexecPlan` and
`perform_reexec`. They are now live code.

In `crates/core/controller-runtime/src/reexec/triage.rs`, remove the `#[cfg_attr(not(test), expect(dead_code, ...))]` from `decide`.

- [ ] **Step 8: Run quality gates**

```bash
cargo fmt --all
cargo check --all-features
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
```

Fix any lint errors before committing.

- [ ] **Step 9: Commit**

```bash
git add crates/core/controller-runtime/src/reexec/listenfd.rs \
        crates/core/controller-runtime/src/reexec/mod.rs \
        crates/core/controller-runtime/src/reexec/triage.rs \
        crates/core/controller-runtime/src/lib.rs
git commit -m "feat(controller-runtime): implement ControllerReexecHook and wire coordinator run() setters

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

### Task 9: `reload_audit_bridge` live file state (Item B)

**Files:**

- Modify: `crates/core/controller-runtime/src/lib.rs`

- [ ] **Step 1: Add `file_digest` import at the top of the `reload_audit_bridge` closure**

`file_digest` was defined in `startup/mod.rs` (Task 7). Re-export it or add a `use` alias in `lib.rs`:

```rust
use crate::startup::file_digest;
```

If `file_digest` is `pub(crate)` (it was marked as such in Task 7), this import works.

- [ ] **Step 2: Update `reload_audit_bridge` signature**

Find:

```rust
async fn reload_audit_bridge(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<uptrakit_config_reload::ReloadAuditEvent>,
    emitter: uptrakit_audit_log::AuditEmitter,
    _file_state_tx: tokio::sync::watch::Sender<uptrakit_config_reload::ConfigFileState>,
    last_reload_tx: tokio::sync::watch::Sender<Option<uptrakit_config_reload::LastReloadInfo>>,
    recent_events_tx: tokio::sync::watch::Sender<Vec<serde_json::Value>>,
) {
```

Replace with:

```rust
async fn reload_audit_bridge(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<uptrakit_config_reload::ReloadAuditEvent>,
    emitter: uptrakit_audit_log::AuditEmitter,
    file_state_tx: tokio::sync::watch::Sender<uptrakit_config_reload::ConfigFileState>,
    last_reload_tx: tokio::sync::watch::Sender<Option<uptrakit_config_reload::LastReloadInfo>>,
    recent_events_tx: tokio::sync::watch::Sender<Vec<serde_json::Value>>,
    config_path: std::path::PathBuf,
) {
```

- [ ] **Step 3: Handle `FileChanged` event in the status-watch arm**

In the `match &event {` block (the first one that updates status channels), add a new arm before the existing `Applied` arm:

```rust
            ReloadAuditEvent::FileChanged { path } => {
                let pending_digest = file_digest(path);
                file_state_tx.send_modify(|s| {
                    s.pending_digest = Some(pending_digest);
                    s.pending_detected_at = Some(time::OffsetDateTime::now_utc());
                });
            }
```

- [ ] **Step 4: Handle `Applied` with file source**

In the existing `Applied` match arm:

```rust
            ReloadAuditEvent::Applied {
                sections,
                per_subsystem_ms,
                source: _,
            } => {
```

Change `source: _` to bind `source` and add file-state update:

```rust
            ReloadAuditEvent::Applied {
                sections,
                per_subsystem_ms,
                source,
            } => {
                let info = uptrakit_config_reload::LastReloadInfo::new(
                    time::OffsetDateTime::now_utc(),
                    sections.clone(),
                    per_subsystem_ms.clone(),
                );
                drop(last_reload_tx.send(Some(info)));

                // Re-read file and update ConfigFileState for file-sourced reloads.
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

                let event_json = serde_json::json!({
                    "type": "applied",
                    "at": time::OffsetDateTime::now_utc()
                        .format(&time::format_description::well_known::Rfc3339)
                        .unwrap_or_else(|_| String::new()),
                    "sections": sections,
                });
                recent_events_tx.send_modify(|v| {
                    v.push(event_json);
                    if v.len() > 20 {
                        v.remove(0);
                    }
                });
            }
```

Also update the second `Applied` match arm (in the audit-log JSON arm further down the function):

```rust
            ReloadAuditEvent::Applied {
                sections,
                per_subsystem_ms,
                source: _,
            } => (
```

- [ ] **Step 5: Clear pending on `Failed`**

In the `Failed` match arm of the status-watch block, add before the `recent_events_tx.send_modify`:

```rust
            ReloadAuditEvent::Failed { .. } => {
                // Clear pending digest — the file did not apply.
                file_state_tx.send_modify(|s| {
                    s.pending_digest = None;
                    s.pending_detected_at = None;
                });
                // ... existing recent_events_tx code below ...
```

- [ ] **Step 6: Update `reload_audit_bridge` call site**

Find `tokio::spawn(reload_audit_bridge(` in `run_server()`. Add the new argument:

```rust
        tokio::spawn(reload_audit_bridge(
            audit_rx,
            audit_emitter,
            reload_file_state_tx,
            reload_last_reload_tx,
            reload_recent_events_tx,
            config_path_for_coord,   // cloned in Task 8 Step 5 before boot_config()
        ));
```

- [ ] **Step 7: Run quality gates**

```bash
cargo fmt --all
cargo check --all-features
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
```

- [ ] **Step 8: Commit**

```bash
git add crates/core/controller-runtime/src/lib.rs
git commit -m "feat(controller-runtime): update reload_audit_bridge to maintain live ConfigFileState (Item B)

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

### Task 10: Additional coordinator run-loop tests

**Files:**

- Modify: `crates/shared/config-reload/tests/coordinator_run.rs`

- [ ] **Step 1: Add remaining tests**

```rust
#[tokio::test(flavor = "current_thread")]
async fn run_loop_emits_requested_then_applied() {
    use tokio::sync::mpsc;

    let (audit_tx, mut audit_rx) = mpsc::unbounded_channel();
    let (coord, handle) = uptrakit_config_reload::ReloadCoordinator::new(
        vec![],
        audit_tx,
        Arc::new(uptrakit_config_reload::NoopAlertWriter),
    );
    let _task = tokio::spawn(coord.run());

    handle
        .sender()
        .send(ReloadRequest {
            source: ReloadSource::DbBump {
                scope: uptrakit_config_reload::Scope::Global,
                sections: vec![],
            },
        })
        .await
        .unwrap();

    // Yield to let the coordinator loop process the message without sleeping.
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }

    // Drain without timeout — events are already in the channel after yielding.
    let mut events = Vec::new();
    while let Ok(e) = audit_rx.try_recv() {
        events.push(e);
    }

    assert!(
        events.iter().any(|e| matches!(e, ReloadAuditEvent::Requested { .. })),
        "expected Requested event; got: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(e, ReloadAuditEvent::Applied { .. })),
        "expected Applied event; got: {events:?}"
    );
}

// This test uses start_paused = true because run_cycle's watchdog calls
// tokio::time::sleep(rollback_window). With paused time, we advance manually
// so the watchdog fires deterministically without wall-clock waiting.
#[tokio::test(start_paused = true)]
async fn run_loop_refuses_when_degraded() {
    use tokio::sync::mpsc;

    // Use a reloadable that fails both apply and revert to force Degraded state.
    struct FailingReloadable;
    #[async_trait::async_trait]
    impl uptrakit_config_reload::reloadable::ReloadableErased for FailingReloadable {
        fn name(&self) -> &'static str { "fail" }
        fn validate(&self, _: &RuntimeConfigDelta) -> Result<(), rootcause::Report> {
            Err(rootcause::report!("forced failure"))
        }
        async fn apply(&self, _: &RuntimeConfigDelta) -> Result<(), rootcause::Report> {
            Err(rootcause::report!("forced failure"))
        }
        async fn revert(&self) -> Result<(), rootcause::Report> {
            Err(rootcause::report!("revert failure — triggers Degraded"))
        }
        async fn health_check(&self) -> Result<(), rootcause::Report> { Ok(()) }
        fn rollback_window(&self) -> std::time::Duration { std::time::Duration::from_millis(10) }
    }

    let (audit_tx, mut audit_rx) = mpsc::unbounded_channel();
    let (coord, handle) = uptrakit_config_reload::ReloadCoordinator::new(
        vec![Arc::new(FailingReloadable)],
        audit_tx,
        Arc::new(uptrakit_config_reload::NoopAlertWriter),
    );
    let _task = tokio::spawn(coord.run());

    // Send first request; apply + revert both fail → Degraded.
    handle
        .sender()
        .send(ReloadRequest {
            source: ReloadSource::DbBump {
                scope: uptrakit_config_reload::Scope::Global,
                sections: vec!["audit".to_string()],
            },
        })
        .await
        .unwrap();
    // Yield first to let the coordinator enter run_cycle and start the watchdog.
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    // Advance time past rollback_window (10ms) so the watchdog fires and
    // triggers revert. With start_paused = true this is instantaneous.
    tokio::time::advance(std::time::Duration::from_millis(20)).await;
    // Yield again to let the coordinator complete the revert and enter Degraded.
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }

    // Send second request; Degraded coordinator emits Refused.
    handle
        .sender()
        .send(ReloadRequest {
            source: ReloadSource::DbBump {
                scope: uptrakit_config_reload::Scope::Global,
                sections: vec!["audit".to_string()],
            },
        })
        .await
        .unwrap();
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }

    let mut events = Vec::new();
    while let Ok(e) = audit_rx.try_recv() {
        events.push(e);
    }

    assert!(
        events.iter().any(|e| matches!(e, ReloadAuditEvent::Refused { .. })),
        "expected Refused event after Degraded; got: {events:?}"
    );
}
```

- [ ] **Step 2: Run all coordinator tests**

```bash
cargo test --no-default-features --features db-sqlite -p uptrakit-config-reload -- coordinator_run
```

Expected: all pass.

- [ ] **Step 3: Commit**

```bash
git add crates/shared/config-reload/tests/coordinator_run.rs
git commit -m "test(config-reload): add coordinator run-loop integration tests

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

### Task 11: Update `docs/development/coding-standards.md`

**Files:**

- Modify: `docs/development/coding-standards.md`

- [ ] **Step 1: Add reexec hook pattern documentation**

Add a new subsection under the existing "Config Reload" section (or create one if absent). The subsection title: `### Reexec Hook Pattern`.

Content to add:

```markdown
### Reexec Hook Pattern

When a config reload detects an irreversibly-bound key change (e.g. `db.url`,
`master_key.path`, `log.path`, embedded-service topology), the coordinator
delegates the decision to a `ReexecHook` implementation registered at startup.

**Rules:**

- The `uptrakit-config-reload` crate defines `ReexecHook` and `ReexecOutcome`;
  it must not import `triage::decide` or `perform_reexec` from
  `controller-runtime`. This boundary keeps the shared crate ignorant of
  process-exec internals.
- The `controller-runtime` crate implements `ControllerReexecHook` (which calls
  `triage::decide` and `perform_reexec`) and registers it via
  `coordinator.set_reexec_hook(...)` before spawning `coordinator.run()`.
- Capture `current_exe` via `std::env::current_exe()` at startup (before the
  hook is constructed) and propagate any error through `run_server()`'s
  `Result` return. Never call `current_exe()` inside the hook — it may fail
  after a process name change.
- Listener FDs for `perform_reexec` are captured by pre-binding HTTPS (and
  PKI when configured) sockets in `run_server()` before spawning server tasks.
  The raw FD integer is valid after the socket is moved into the server task;
  `clear_cloexec_raw` uses the integer, not the Rust wrapper.
- When no `ReexecHook` is registered (e.g. in tests), the coordinator skips
  the reexec check and proceeds with in-process apply.
```

- [ ] **Step 2: Run markdownlint**

```bash
npx prettier --write docs/development/coding-standards.md
markdownlint --config .markdownlint.json docs/development/coding-standards.md
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add docs/development/coding-standards.md
git commit -m "docs: document reexec hook pattern in coding-standards

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

### Final quality gate

- [ ] **Run full suite**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
markdownlint --config .markdownlint.json '**/*.md'
```

Expected: all pass, no warnings.
