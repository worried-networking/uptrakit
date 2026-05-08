# Scheduler Engine Merge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Merge `crates/shared/scheduler-engine` into `crates/core/scheduler-runtime`, making
`scheduler-runtime` the single crate that owns the scheduling framework and exports
`StandaloneSchedulerHandler` (the sole `ServiceHandler` impl).

**Architecture:** Move all 20 source files from `scheduler-engine/src/` directly into
`scheduler-runtime/src/`. Update `scheduler-runtime/Cargo.toml` to absorb all engine deps (some
become mandatory that were previously optional). Update all import sites across the workspace.
Delete the `scheduler-engine` crate.

**Tech Stack:** Rust, sea-orm, tokio, rootcause

---

## File Map

| Action | Path                                                                    |
| ------ | ----------------------------------------------------------------------- |
| Modify | `crates/core/scheduler-runtime/Cargo.toml`                              |
| Create | `crates/core/scheduler-runtime/src/ca_utils.rs` (copy from engine)      |
| Create | `crates/core/scheduler-runtime/src/claim.rs` (copy from engine)         |
| Create | `crates/core/scheduler-runtime/src/error.rs` (copy from engine)         |
| Create | `crates/core/scheduler-runtime/src/executor.rs` (copy from engine)      |
| Create | `crates/core/scheduler-runtime/src/interval.rs` (copy from engine)      |
| Create | `crates/core/scheduler-runtime/src/notifier.rs` (copy from engine)      |
| Create | `crates/core/scheduler-runtime/src/scheduler.rs` (copy from engine)     |
| Create | `crates/core/scheduler-runtime/src/tick_executor.rs` (copy from engine) |
| Create | `crates/core/scheduler-runtime/src/executors/` (copy from engine)       |
| Modify | `crates/core/scheduler-runtime/src/lib.rs`                              |
| Modify | `crates/core/scheduler-runtime/src/runtime.rs`                          |
| Modify | `crates/core/scheduler-runtime/src/standalone.rs`                       |
| Modify | `crates/core/controller-runtime/Cargo.toml`                             |
| Modify | `crates/core/controller-runtime/src/scheduler/mod.rs`                   |
| Modify | `crates/core/scheduler/Cargo.toml`                                      |
| Modify | `crates/ui/web-api-queries/Cargo.toml`                                  |
| Modify | `Cargo.toml` (workspace root)                                           |
| Delete | `crates/shared/scheduler-engine/` (entire directory)                    |

---

### Task 1: Absorb engine deps into scheduler-runtime/Cargo.toml

**Files:**

- Modify: `crates/core/scheduler-runtime/Cargo.toml`

- [ ] **Step 1: Replace the Cargo.toml**

Replace the entire file with the following (engine's deps become mandatory; `standalone` retains
its exclusive optional deps; `uptrakit-audit-log` and `uptrakit-wire` promote from optional to
mandatory because the engine executors require them unconditionally):

```toml
[package]
name = "uptrakit-scheduler-runtime"
description = "Uptrakit scheduler runtime: task scheduling and dispatch"
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
version = "0.0.2"

[features]
default = []
standalone = [
    "dep:uptrakit-crypto",
    "dep:uptrakit-nats",
    "dep:uptrakit-service-sdk",
    "dep:zeroize",
]
db-all = ["db-sqlite", "db-postgres"]
db-sqlite = ["sea-orm/sqlx-sqlite", "uptrakit-shared-db/db-sqlite"]
db-postgres = ["sea-orm/sqlx-postgres", "uptrakit-shared-db/db-postgres"]
oidc = []

[dependencies]
async-trait = { workspace = true }
rand = { workspace = true }
rootcause = { workspace = true }
sea-orm = { workspace = true }
serde_json = { workspace = true }
strum = { workspace = true }
thiserror = { workspace = true }
time = { workspace = true }
tokio = { workspace = true, features = ["rt", "sync", "time"] }
tokio-util = { workspace = true }
tracing = { workspace = true }
uptrakit-audit-log = { workspace = true }
uptrakit-command = { workspace = true }
uptrakit-config-merge = { workspace = true }
uptrakit-plugin-infrastructure-registry = { workspace = true }
uptrakit-shared-db = { workspace = true }
uptrakit-shared-macros = { workspace = true }
uptrakit-shared-types = { workspace = true }
uptrakit-wire = { workspace = true }
uuid = { workspace = true, features = ["v7"] }
x509-parser = { workspace = true }
uptrakit-crypto = { workspace = true, optional = true }
uptrakit-nats = { workspace = true, optional = true }
uptrakit-service-sdk = { workspace = true, optional = true }
zeroize = { workspace = true, optional = true }

[dev-dependencies]
parking_lot = { workspace = true }
tokio = { workspace = true, features = ["macros", "rt-multi-thread", "test-util"] }
uptrakit-crypto = { workspace = true, features = ["testing"] }
uptrakit-plugin-infrastructure-registry = { workspace = true, features = ["test-support"] }
uptrakit-shared-db = { workspace = true, features = ["db-sqlite", "migration"] }

[lints]
workspace = true
```

- [ ] **Step 2: Verify it parses**

```bash
cargo metadata --manifest-path crates/core/scheduler-runtime/Cargo.toml --no-deps --format-version 1 > /dev/null
```

Expected: no output, exit 0.

- [ ] **Step 3: Commit**

```bash
git add crates/core/scheduler-runtime/Cargo.toml
git commit -m "build(scheduler-runtime): absorb engine deps ahead of merge"
```

---

### Task 2: Copy engine source files into scheduler-runtime/src/

**Files:**

- Create: `crates/core/scheduler-runtime/src/{ca_utils,claim,error,executor,interval,notifier,scheduler,tick_executor}.rs`
- Create: `crates/core/scheduler-runtime/src/executors/{mod,audit_log_cleanup,auth_cleanup,awaiting_restart,crl_renewal,detect_version,discover_software,fetch_releases,queries,service_cert_check,stale_lease_cleanup}.rs`

The engine files use `crate::` paths that resolve correctly after the move because all modules
land in the same crate root. No intra-engine imports need updating — `use crate::error::*` in
an engine file becomes `crate::error::*` inside `scheduler-runtime`, which is correct.

- [ ] **Step 1: Copy flat files**

```bash
cp crates/shared/scheduler-engine/src/ca_utils.rs crates/core/scheduler-runtime/src/ca_utils.rs
cp crates/shared/scheduler-engine/src/claim.rs crates/core/scheduler-runtime/src/claim.rs
cp crates/shared/scheduler-engine/src/error.rs crates/core/scheduler-runtime/src/error.rs
cp crates/shared/scheduler-engine/src/executor.rs crates/core/scheduler-runtime/src/executor.rs
cp crates/shared/scheduler-engine/src/interval.rs crates/core/scheduler-runtime/src/interval.rs
cp crates/shared/scheduler-engine/src/notifier.rs crates/core/scheduler-runtime/src/notifier.rs
cp crates/shared/scheduler-engine/src/scheduler.rs crates/core/scheduler-runtime/src/scheduler.rs
cp crates/shared/scheduler-engine/src/tick_executor.rs crates/core/scheduler-runtime/src/tick_executor.rs
```

- [ ] **Step 2: Copy executors directory**

```bash
mkdir -p crates/core/scheduler-runtime/src/executors
cp crates/shared/scheduler-engine/src/executors/mod.rs crates/core/scheduler-runtime/src/executors/mod.rs
cp crates/shared/scheduler-engine/src/executors/audit_log_cleanup.rs crates/core/scheduler-runtime/src/executors/audit_log_cleanup.rs
cp crates/shared/scheduler-engine/src/executors/auth_cleanup.rs crates/core/scheduler-runtime/src/executors/auth_cleanup.rs
cp crates/shared/scheduler-engine/src/executors/awaiting_restart.rs crates/core/scheduler-runtime/src/executors/awaiting_restart.rs
cp crates/shared/scheduler-engine/src/executors/crl_renewal.rs crates/core/scheduler-runtime/src/executors/crl_renewal.rs
cp crates/shared/scheduler-engine/src/executors/detect_version.rs crates/core/scheduler-runtime/src/executors/detect_version.rs
cp crates/shared/scheduler-engine/src/executors/discover_software.rs crates/core/scheduler-runtime/src/executors/discover_software.rs
cp crates/shared/scheduler-engine/src/executors/fetch_releases.rs crates/core/scheduler-runtime/src/executors/fetch_releases.rs
cp crates/shared/scheduler-engine/src/executors/queries.rs crates/core/scheduler-runtime/src/executors/queries.rs
cp crates/shared/scheduler-engine/src/executors/service_cert_check.rs crates/core/scheduler-runtime/src/executors/service_cert_check.rs
cp crates/shared/scheduler-engine/src/executors/stale_lease_cleanup.rs crates/core/scheduler-runtime/src/executors/stale_lease_cleanup.rs
```

- [ ] **Step 3: Commit**

```bash
git add crates/core/scheduler-runtime/src/
git commit -m "feat(scheduler-runtime): copy engine source files (pre-integration)"
```

---

### Task 3: Rewrite scheduler-runtime/src/lib.rs

**Files:**

- Modify: `crates/core/scheduler-runtime/src/lib.rs`

- [ ] **Step 1: Replace lib.rs**

```rust
pub mod ca_utils;
pub mod claim;
pub mod error;
pub mod executor;
pub mod executors;
pub mod interval;
pub mod notifier;
pub mod scheduler;
pub mod tick_executor;

mod runtime;
#[cfg(feature = "standalone")]
mod standalone;

pub use error::{Result, SchedulerError};
pub use executor::TaskExecutor;
pub use notifier::SchedulerNotifier;
pub use scheduler::{Scheduler, SchedulerConfig, TASK_EXECUTION_TIMEOUT};
pub use tick_executor::TickExecutor;

pub use runtime::{ManagedSchedulerRuntime, SchedulerRunConfig, SchedulerStopMode, run_scheduler};
#[cfg(feature = "standalone")]
pub use standalone::{
    STANDALONE_SCHEDULER_APP_NAME, STANDALONE_SCHEDULER_DIR_NAME, STANDALONE_SCHEDULER_LABEL,
    StandaloneSchedulerHandler, standalone_scheduler_capabilities,
};
```

- [ ] **Step 2: Check the crate compiles (engine still present — conflicts expected)**

```bash
cargo check -p uptrakit-scheduler-runtime --features standalone,db-sqlite 2>&1 | head -30
```

Expected at this point: compile errors about unresolved `uptrakit_scheduler_engine` imports —
the engine dep was removed from Cargo.toml in Task 1, so `runtime.rs` and `standalone.rs` cannot
resolve it. This is expected. Skip this check and proceed to Task 4 to fix those imports.

- [ ] **Step 3: Commit**

```bash
git add crates/core/scheduler-runtime/src/lib.rs
git commit -m "feat(scheduler-runtime): declare engine modules in lib.rs"
```

---

### Task 4: Fix imports in runtime.rs and standalone.rs

**Files:**

- Modify: `crates/core/scheduler-runtime/src/runtime.rs`
- Modify: `crates/core/scheduler-runtime/src/standalone.rs`

- [ ] **Step 1: Update runtime.rs — replace engine imports with crate-local ones**

In `crates/core/scheduler-runtime/src/runtime.rs`, replace:

```rust
use uptrakit_scheduler_engine::executors::{
    auth_cleanup::AuthCleanupExecutor, detect_version::DetectVersionExecutor,
    discover_software::DiscoverSoftwareExecutor, fetch_releases::FetchReleasesExecutor,
    stale_lease_cleanup::StaleLeaseCleanupExecutor,
};
use uptrakit_scheduler_engine::{
    Scheduler, SchedulerConfig, SchedulerNotifier, TASK_EXECUTION_TIMEOUT,
};
```

with:

```rust
use crate::executors::{
    auth_cleanup::AuthCleanupExecutor, detect_version::DetectVersionExecutor,
    discover_software::DiscoverSoftwareExecutor, fetch_releases::FetchReleasesExecutor,
    stale_lease_cleanup::StaleLeaseCleanupExecutor,
};
use crate::{Scheduler, SchedulerConfig, SchedulerNotifier, TASK_EXECUTION_TIMEOUT};
```

- [ ] **Step 2: Update standalone.rs — replace all engine references**

In `crates/core/scheduler-runtime/src/standalone.rs`, replace every
`uptrakit_scheduler_engine::` reference with `crate::`. The file has two affected locations:

1. Top-level import (line ~9):

```rust
use uptrakit_scheduler_engine::SchedulerNotifier;
```

→

```rust
use crate::SchedulerNotifier;
```

1. Inline path in the constructor body (line ~146):

```rust
uptrakit_scheduler_engine::executors::audit_log_cleanup::AuditLogCleanupExecutor::new(
```

→

```rust
crate::executors::audit_log_cleanup::AuditLogCleanupExecutor::new(
```

You can apply both at once with:

```bash
sed -i 's/uptrakit_scheduler_engine::/crate::/g' crates/core/scheduler-runtime/src/standalone.rs
```

Verify no engine references remain:

```bash
grep -n "uptrakit_scheduler_engine" crates/core/scheduler-runtime/src/standalone.rs
```

Expected: no output.

- [ ] **Step 3: Verify crate compiles**

```bash
cargo check -p uptrakit-scheduler-runtime --features standalone,db-sqlite
```

Expected: no errors — all `uptrakit_scheduler_engine::` references in both `runtime.rs` and
`standalone.rs` are now resolved to `crate::` paths.

- [ ] **Step 4: Commit**

```bash
git add crates/core/scheduler-runtime/src/runtime.rs crates/core/scheduler-runtime/src/standalone.rs
git commit -m "fix(scheduler-runtime): switch engine imports to crate-local paths"
```

---

### Task 5: Rename StandaloneSchedulerHandler → SchedulerHandler

**Files:**

- Modify: `crates/core/scheduler-runtime/src/standalone.rs`
- Modify: `crates/core/scheduler-runtime/src/lib.rs`
- Modify: `crates/core/scheduler/src/main.rs`

The spec requires the public name be `SchedulerHandler`. The current name `StandaloneSchedulerHandler`
was a placeholder that pre-dates the binary/runtime boundary invariant.

- [ ] **Step 1: Write failing test**

```bash
grep -q "pub struct SchedulerHandler" crates/core/scheduler-runtime/src/standalone.rs && \
  echo "PASS" || echo "FAIL: SchedulerHandler not found yet"
```

Expected: `FAIL`.

- [ ] **Step 2: Rename in standalone.rs**

```bash
sed -i 's/StandaloneSchedulerHandler/SchedulerHandler/g' crates/core/scheduler-runtime/src/standalone.rs
```

Verify:

```bash
grep -n "StandaloneSchedulerHandler" crates/core/scheduler-runtime/src/standalone.rs
```

Expected: no output.

- [ ] **Step 3: Update lib.rs re-export**

In `crates/core/scheduler-runtime/src/lib.rs`, in the `#[cfg(feature = "standalone")]` re-export
block, replace:

```rust
    StandaloneSchedulerHandler, standalone_scheduler_capabilities,
```

with:

```rust
    SchedulerHandler, standalone_scheduler_capabilities,
```

- [ ] **Step 4: Update scheduler/src/main.rs construction site**

In `crates/core/scheduler/src/main.rs`, replace:

```rust
StandaloneSchedulerHandler::new(
```

with:

```rust
SchedulerHandler::new(
```

Also update the import at the top of `main.rs`. Find and replace any:

```rust
use uptrakit_scheduler_runtime::StandaloneSchedulerHandler;
```

with:

```rust
use uptrakit_scheduler_runtime::SchedulerHandler;
```

- [ ] **Step 5: Run the test**

```bash
grep -q "pub struct SchedulerHandler" crates/core/scheduler-runtime/src/standalone.rs && \
  echo "PASS" || echo "FAIL"
```

Expected: `PASS`.

- [ ] **Step 6: Verify binary compiles**

```bash
cargo check -p uptrakit-scheduler --features standalone,db-sqlite
```

Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add crates/core/scheduler-runtime/src/standalone.rs crates/core/scheduler-runtime/src/lib.rs crates/core/scheduler/src/main.rs
git commit -m "refactor(scheduler-runtime): rename StandaloneSchedulerHandler to SchedulerHandler"
```

---

### Task 6: Fix imports in controller-runtime

**Files:**

- Modify: `crates/core/controller-runtime/src/scheduler/mod.rs`
- Modify: `crates/core/controller-runtime/Cargo.toml`

- [ ] **Step 1: Update scheduler/mod.rs — top-level imports**

In `crates/core/controller-runtime/src/scheduler/mod.rs`, replace:

```rust
use uptrakit_scheduler_engine::{SchedulerNotifier, TaskExecutor};
```

with:

```rust
use uptrakit_scheduler_runtime::{SchedulerNotifier, TaskExecutor};
```

- [ ] **Step 2: Update scheduler/mod.rs — body imports in run_embedded_scheduler**

In the `run_embedded_scheduler` function body, replace:

```rust
    use uptrakit_scheduler_engine::executors::{
        audit_log_cleanup, crl_renewal, service_cert_check,
    };
```

with:

```rust
    use uptrakit_scheduler_runtime::executors::{
        audit_log_cleanup, crl_renewal, service_cert_check,
    };
```

- [ ] **Step 3: Update scheduler/mod.rs — AwaitingRestartExecutor path**

Replace:

```rust
                Box::new(
                    uptrakit_scheduler_engine::executors::awaiting_restart::AwaitingRestartExecutor::new(
                        Arc::clone(&notifier_for_extras),
                    ),
                ),
```

with:

```rust
                Box::new(
                    uptrakit_scheduler_runtime::executors::awaiting_restart::AwaitingRestartExecutor::new(
                        Arc::clone(&notifier_for_extras),
                    ),
                ),
```

- [ ] **Step 4: Update scheduler/mod.rs — error::Result path in CaRotationCheckExecutor**

Replace:

```rust
    ) -> uptrakit_scheduler_engine::error::Result<()> {
```

with:

```rust
    ) -> uptrakit_scheduler_runtime::error::Result<()> {
```

- [ ] **Step 5: Update controller-runtime/Cargo.toml**

In `crates/core/controller-runtime/Cargo.toml`:

Replace the `embedded-scheduler` feature:

```toml
embedded-scheduler = ["dep:uptrakit-scheduler-engine", "dep:uptrakit-scheduler-runtime"]
```

with:

```toml
embedded-scheduler = ["dep:uptrakit-scheduler-runtime"]
```

Replace the `oidc` feature (remove `uptrakit-scheduler-engine?/oidc`):

```toml
oidc = ["uptrakit-web-api/oidc", "uptrakit-scheduler-engine?/oidc", "uptrakit-scheduler-runtime?/oidc"]
```

with:

```toml
oidc = ["uptrakit-web-api/oidc", "uptrakit-scheduler-runtime?/oidc"]
```

Remove the optional engine dep from `[dependencies]`:

```toml
uptrakit-scheduler-engine = { workspace = true, optional = true }
```

(Delete this line entirely.)

- [ ] **Step 6: Verify controller-runtime compiles**

```bash
cargo check -p uptrakit-controller-runtime --features embedded-scheduler,db-sqlite
```

Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add crates/core/controller-runtime/src/scheduler/mod.rs crates/core/controller-runtime/Cargo.toml
git commit -m "fix(controller-runtime): switch scheduler/mod.rs to uptrakit_scheduler_runtime"
```

---

### Task 7: Update scheduler binary and web-api-queries

**Files:**

- Modify: `crates/core/scheduler/Cargo.toml`
- Modify: `crates/ui/web-api-queries/Cargo.toml`
- Modify: `crates/ui/web-api-queries/src/queries/scheduled_tasks.rs`

- [ ] **Step 1: Update scheduler/Cargo.toml**

Replace the `oidc` feature:

```toml
oidc = ["uptrakit-scheduler-engine/oidc", "uptrakit-scheduler-runtime/oidc"]
```

with:

```toml
oidc = ["uptrakit-scheduler-runtime/oidc"]
```

Remove `uptrakit-scheduler-engine` from `[dependencies]`:

```toml
uptrakit-scheduler-engine = { workspace = true }
```

(Delete this line entirely.)

- [ ] **Step 2: Verify scheduler binary compiles**

```bash
cargo check -p uptrakit-scheduler --features db-sqlite
```

Expected: no errors.

- [ ] **Step 3: Update web-api-queries/Cargo.toml**

Remove from `[dependencies]`:

```toml
uptrakit-scheduler-engine = { workspace = true }
```

(Delete this line entirely.)

- [ ] **Step 4: Update comment in scheduled_tasks.rs**

In `crates/ui/web-api-queries/src/queries/scheduled_tasks.rs`, replace the comment:

```rust
/// Compute the next run time inline (same logic as `uptrakit_scheduler_engine::interval`
```

with:

```rust
/// Compute the next run time inline (same logic as `uptrakit_scheduler_runtime::interval`
```

- [ ] **Step 5: Verify web-api-queries compiles**

```bash
cargo check -p uptrakit-web-api-queries --features db-sqlite
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add crates/core/scheduler/Cargo.toml crates/ui/web-api-queries/Cargo.toml crates/ui/web-api-queries/src/queries/scheduled_tasks.rs
git commit -m "fix(scheduler,web-api-queries): remove dep on deleted scheduler-engine crate"
```

---

### Task 8: Delete scheduler-engine and remove workspace entry

**Files:**

- Modify: `Cargo.toml` (workspace root)
- Delete: `crates/shared/scheduler-engine/` (entire directory)

- [ ] **Step 1: Remove workspace dependency entry from root Cargo.toml**

In `Cargo.toml`, in the `[workspace.dependencies]` section, delete the line:

```toml
uptrakit-scheduler-engine = { path = "crates/shared/scheduler-engine", version = "0.0.1" }
```

The workspace `members` glob (`"crates/shared/*"`) auto-discovers crates; deleting the directory
is sufficient to remove it from the member list.

- [ ] **Step 2: Delete the scheduler-engine directory**

```bash
rm -rf crates/shared/scheduler-engine
```

- [ ] **Step 3: Verify full workspace compiles**

```bash
cargo check --all-features
```

Expected: no errors referencing `scheduler-engine`.

- [ ] **Step 4: Run scheduler-runtime tests**

```bash
cargo test -p uptrakit-scheduler-runtime --features standalone,db-sqlite
```

Expected: all tests pass. (The engine had tests in each file; they are now part of
`scheduler-runtime` and run here.)

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml
git rm -r crates/shared/scheduler-engine
git commit -m "feat(scheduler-runtime): complete engine merge — delete scheduler-engine crate"
```

---

### Task 9: Quality gates

- [ ] **Step 1: Format**

```bash
cargo fmt --all
```

- [ ] **Step 2: Clippy (no-default-features)**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep -E "^error"
```

Expected: no errors.

- [ ] **Step 3: Clippy (all features)**

```bash
cargo clippy --all-targets --all-features 2>&1 | grep -E "^error"
```

Expected: no errors.

- [ ] **Step 4: Full test suite**

```bash
cargo test --all-features
```

Expected: all tests pass.

- [ ] **Step 5: deny check**

```bash
cargo deny check
```

Expected: no bans or advisories related to removed crate.

- [ ] **Step 6: Commit formatting if needed**

```bash
git add -u && git diff --cached --quiet || git commit -m "style: cargo fmt after scheduler-engine merge"
```

---

**Plan complete and saved to `docs/superpowers/plans/2026-05-07-scheduler-engine-merge.md`.**
