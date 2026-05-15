# Graceful Reload Leftovers — Item C Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix `DbPoolReloadable` so that `apply()` actually delivers new pools to consumers, and migrate the config
reconciler (the primary live consumer) to receive pool updates via `watch::Receiver<Arc<DbConnHandle>>`.

**Architecture:** `DbPoolReloadable::new()` currently drops its only receiver (`_rx`), so `apply()` publishes to zero subscribers —
pool swaps are silently discarded.
A new `subscribe()` method on `DbPoolReloadable` creates additional receivers; `spawn_config_reconciler` is refactored to accept
`watch::Receiver<Arc<DbConnHandle>>` instead of a bare `DatabaseConnection` and re-reads the current handle at each poll tick.
All code stays within `controller-runtime` — no cross-crate type moves.
The five remaining long-lived DB holders (audit backend, enricher, notification dispatcher, surface proxy, AppState) are deferred to a follow-up:
they snapshot the initial pool at startup and that is acceptable for V1 since pool-size changes are rare in practice.

**Tech Stack:** Rust 2024 · `uptrakit-controller-runtime` · `tokio::sync::watch`

---

## File Map

| Action | Path                                                      |
| ------ | --------------------------------------------------------- |
| Modify | `crates/core/controller-runtime/src/reload/db_pool.rs`    |
| Modify | `crates/core/controller-runtime/src/reload/reconciler.rs` |
| Modify | `crates/core/controller-runtime/src/lib.rs`               |
| Modify | `docs/development/coding-standards.md`                    |

---

### Task 1: Add `subscribe()` to `DbPoolReloadable`

**Files:**

- Modify: `crates/core/controller-runtime/src/reload/db_pool.rs`

The `(tx, _rx)` line in `new()` drops the sole receiver immediately. Add `subscribe()` using `Sender::subscribe()`,
which creates new receivers from an existing sender. Callers must hold at least one receiver alive for `apply()` to propagate.
The `#[expect(clippy::let_underscore_must_use, ...)]` annotation on `let _ = self.tx.send(...)` in `apply()` and `revert()` must
remain unchanged — the lint fires on the syntax pattern regardless of whether receivers exist at runtime;
removing it would break `cargo clippy` (workspace `warnings = "deny"`).

- [ ] **Step 1: Add `subscribe()` method**

In `crates/core/controller-runtime/src/reload/db_pool.rs`, add after the closing brace of `new()` (still inside `impl DbPoolReloadable`):

```rust
    /// Subscribe to pool-handle updates.
    ///
    /// Returns a receiver that gets a new value every time [`apply`] opens a
    /// replacement pool.  Callers must hold the receiver alive for the watch
    /// channel to deliver updates.
    pub(crate) fn subscribe(&self) -> watch::Receiver<Arc<DbConnHandle>> {
        self.tx.subscribe()
    }
```

- [ ] **Step 2: Write the test**

Append to the `#[cfg(test)] mod tests` block:

```rust
    #[tokio::test(flavor = "current_thread")]
    async fn subscribe_receives_new_pool_after_apply() {
        let pool = build_test_pool().await;
        let reloadable = DbPoolReloadable::new(pool.clone(), TEST_URL.to_string());
        let mut rx = reloadable.subscribe();

        // No change yet.
        assert!(!rx.has_changed().unwrap());

        let new_cfg = std::sync::Arc::new(DbConfig::with_all(TEST_URL, 32, 6_000));
        reloadable.apply(new_cfg).await.unwrap();

        // apply() should have published a new handle.
        assert!(rx.has_changed().unwrap(), "subscriber did not receive new pool");
    }
```

- [ ] **Step 3: Run the test to verify it passes**

```bash
cargo test -p uptrakit-controller-runtime reload::db_pool::tests::subscribe_receives_new_pool_after_apply -- --nocapture
```

Expected: test passes.

- [ ] **Step 4: Commit**

```bash
git add crates/core/controller-runtime/src/reload/db_pool.rs
git commit -m "feat(db-pool): add subscribe() to DbPoolReloadable

Callers holding a Receiver<Arc<DbConnHandle>> now get new pool handles
delivered when apply() opens a replacement connection.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

### Task 2: Migrate `spawn_config_reconciler` to `watch::Receiver<Arc<DbConnHandle>>`

**Files:**

- Modify: `crates/core/controller-runtime/src/reload/reconciler.rs`

The reconciler's tight poll loop (every 2 s) is the primary live consumer that benefits from watch-driven pool delivery.
Change the signature to accept `watch::Receiver<Arc<DbConnHandle>>` and re-read the current handle at each `poll_once` tick.

- [ ] **Step 1: Write the failing test for the new signature**

In the `#[cfg(test)] mod tests` block of `reconciler.rs`, add the following import and test (does not compile yet — reconciler signature not changed):

```rust
    use crate::reload::db_pool::{DbConnHandle, DbPoolReloadable};
    use std::sync::Arc;
    use uptrakit_config_reload::config::DbConfig;

    #[tokio::test(flavor = "current_thread")]
    async fn reconciler_works_with_watch_receiver() {
        let db = build_test_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant_and_version(&db, tenant_id, 1, 1).await;

        // Build a DbPoolReloadable around the connection and subscribe.
        let reloadable = DbPoolReloadable::new(db, "sqlite::memory:".to_string());
        let db_rx = reloadable.subscribe();

        let (tx, mut rx) = mpsc::channel(8);
        let cache = SettingsVersionCache::new();
        let cancel = CancellationToken::new();

        let handle = spawn_config_reconciler(db_rx, tx, cache, cancel.clone());

        let req = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        assert!(matches!(
            req.source,
            ReloadSource::DbBump { scope: Scope::Global, .. }
        ));

        cancel.cancel();
        handle.await.expect("task panicked");
    }
```

- [ ] **Step 2: Run the test to confirm it fails to compile**

```bash
cargo test -p uptrakit-controller-runtime reload::reconciler::tests::reconciler_works_with_watch_receiver 2>&1 | head -20
```

Expected: compile error (wrong signature for `spawn_config_reconciler`).

- [ ] **Step 3: Update `spawn_config_reconciler` and `poll_once`**

Replace the full content of `crates/core/controller-runtime/src/reload/reconciler.rs`
(keeping the module doc and imports; replace only the two function bodies):

New imports to add at the top (replace the `use sea_orm::{DatabaseConnection, EntityTrait};` line):

```rust
use std::sync::Arc;

use sea_orm::EntityTrait;
use tokio::sync::watch;
```

New `spawn_config_reconciler` signature and body:

```rust
pub(crate) fn spawn_config_reconciler(
    db_rx: watch::Receiver<Arc<crate::reload::db_pool::DbConnHandle>>,
    tx: mpsc::Sender<ReloadRequest>,
    cache: SettingsVersionCache,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(RECONCILER_POLL);
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    debug!("config reconciler shutting down");
                    break;
                }
                _ = tick.tick() => {}
            }

            match poll_once(&db_rx, &tx, &cache).await {
                Ok(()) => {}
                Err(e) => {
                    warn!(error = %e, "config reconciler poll failed; retrying next tick");
                }
            }
        }
    })
}
```

New `poll_once` signature and body:

```rust
async fn poll_once(
    db_rx: &watch::Receiver<Arc<crate::reload::db_pool::DbConnHandle>>,
    tx: &mpsc::Sender<ReloadRequest>,
    cache: &SettingsVersionCache,
) -> Result<(), Report> {
    let handle = db_rx.borrow().clone();
    let rows = settings_version::Entity::find()
        .all(handle.conn())
        .await
        .map_err(|e| rootcause::report!(e))?;

    for row in rows {
        let tenant_scope = Scope::Tenant(row.tenant_id);
        let new_global = u64::try_from(row.global_version).unwrap_or(0);
        let new_tenant = u64::try_from(row.version).unwrap_or(0);

        let prior_global = cache.get(Scope::Global).unwrap_or(0);
        let prior_tenant = cache.get(tenant_scope).unwrap_or(0);

        if new_global > prior_global {
            cache.update(Scope::Global, new_global);
            let req = ReloadRequest {
                source: ReloadSource::DbBump {
                    scope: Scope::Global,
                    sections: vec!["audit".into(), "registration".into()],
                },
                timestamp: OffsetDateTime::now_utc(),
            };
            if tx.send(req).await.is_err() {
                debug!("coordinator channel closed; reconciler exiting");
                return Ok(());
            }
        }

        if new_tenant > prior_tenant {
            cache.update(tenant_scope, new_tenant);
            let req = ReloadRequest {
                source: ReloadSource::DbBump {
                    scope: tenant_scope,
                    sections: vec!["audit_log".into()],
                },
                timestamp: OffsetDateTime::now_utc(),
            };
            if tx.send(req).await.is_err() {
                debug!("coordinator channel closed; reconciler exiting");
                return Ok(());
            }
        }
    }

    Ok(())
}
```

- [ ] **Step 4: Update the four existing reconciler tests for the new signature**

The existing tests pass `db: DatabaseConnection` directly to `spawn_config_reconciler` and `poll_once`.
After Step 3's signature change they no longer compile. Replace all four in the `#[cfg(test)] mod tests` block:

```rust
    // Helper — wraps a DatabaseConnection in a watch channel for tests.
    // The DbPoolReloadable (and its Sender) is dropped after subscribe(); the
    // Receiver still works for borrow() reads. Tests that need apply() should
    // keep the reloadable alive separately (as reconciler_works_with_watch_receiver does).
    fn make_db_rx(db: sea_orm::DatabaseConnection) -> tokio::sync::watch::Receiver<Arc<crate::reload::db_pool::DbConnHandle>> {
        use crate::reload::db_pool::DbPoolReloadable;
        DbPoolReloadable::new(db, "sqlite::memory:".to_string()).subscribe()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reconciler_detects_global_bump() {
        let db = build_test_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant_and_version(&db, tenant_id, 1, 1).await;

        let db_rx = make_db_rx(db);
        let (tx, mut rx) = mpsc::channel(8);
        let cache = SettingsVersionCache::new();
        let cancel = CancellationToken::new();
        let handle = spawn_config_reconciler(db_rx, tx, cache.clone(), cancel.clone());

        let req = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        assert!(matches!(
            req.source,
            ReloadSource::DbBump { scope: Scope::Global, .. }
        ));

        cancel.cancel();
        handle.await.expect("task panicked");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reconciler_detects_tenant_bump() {
        let db = build_test_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant_and_version(&db, tenant_id, 1, 0).await;

        let db_rx = make_db_rx(db);
        let (tx, mut rx) = mpsc::channel(8);
        let cache = SettingsVersionCache::new();
        let cancel = CancellationToken::new();
        let handle = spawn_config_reconciler(db_rx, tx, cache.clone(), cancel.clone());

        let req = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        assert!(matches!(
            req.source,
            ReloadSource::DbBump { scope: Scope::Tenant(_), .. }
        ));

        cancel.cancel();
        handle.await.expect("task panicked");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reconciler_does_not_re_emit_on_unchanged_version() {
        let db = build_test_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant_and_version(&db, tenant_id, 1, 1).await;

        let db_rx = make_db_rx(db);
        let (tx, mut rx) = mpsc::channel(8);
        let cache = SettingsVersionCache::new();
        let cancel = CancellationToken::new();
        let handle = spawn_config_reconciler(db_rx, tx, cache.clone(), cancel.clone());

        loop {
            let got = tokio::time::timeout(
                RECONCILER_POLL + std::time::Duration::from_millis(300),
                rx.recv(),
            )
            .await;
            if got.is_err() {
                break;
            }
        }

        let nothing = tokio::time::timeout(RECONCILER_POLL * 2, rx.recv()).await;
        assert!(nothing.is_err(), "expected timeout, not a second bump");

        cancel.cancel();
        handle.await.expect("task panicked");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn poll_once_emits_nothing_on_empty_db() {
        let db = build_test_db().await;
        let db_rx = make_db_rx(db);
        let (tx, mut rx) = mpsc::channel(8);
        let cache = SettingsVersionCache::new();

        poll_once(&db_rx, &tx, &cache).await.unwrap();
        drop(tx);

        assert!(rx.recv().await.is_none(), "expected no messages");
    }
```

- [ ] **Step 5: Run the test to verify it passes**

```bash
cargo test -p uptrakit-controller-runtime reload::reconciler::tests -- --nocapture
```

Expected: all reconciler tests pass including `reconciler_works_with_watch_receiver`.

- [ ] **Step 6: Commit**

```bash
git add crates/core/controller-runtime/src/reload/reconciler.rs
git commit -m "refactor(reconciler): accept watch::Receiver<Arc<DbConnHandle>> instead of DatabaseConnection

The reconciler re-reads the current pool handle on every poll tick, so it
now automatically picks up replacement pools published by DbPoolReloadable::apply().

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

### Task 3: Wire `db_rx` in `run_server()`

**Files:**

- Modify: `crates/core/controller-runtime/src/lib.rs`

Extract `db_rx = db_reloadable.subscribe()` immediately after constructing `db_reloadable` (line ~666) but **before**
`Arc::new(db_reloadable)` moves the value. Update `spawn_config_reconciler` to receive `db_rx`.

- [ ] **Step 1: Update the coordinator wiring block**

In `crates/core/controller-runtime/src/lib.rs`, locate the line:

```rust
        let db_reloadable = reload::db_pool::DbPoolReloadable::new(db_conn.clone(), db_url.clone());
```

Add the `subscribe()` call immediately after it (before `Arc::new(db_reloadable)` in the `reloadables` vec):

```rust
        let db_reloadable = reload::db_pool::DbPoolReloadable::new(db_conn.clone(), db_url.clone());
        let db_rx = db_reloadable.subscribe();
```

- [ ] **Step 2: Update the reconciler call site**

Locate:

```rust
        let _reconciler = reload::reconciler::spawn_config_reconciler(
            db_conn.clone(),
            coordinator_handle.sender(),
            b.settings_version_cache.clone(),
            shutdown_token.clone(),
        );
```

Replace with:

```rust
        let _reconciler = reload::reconciler::spawn_config_reconciler(
            db_rx,
            coordinator_handle.sender(),
            b.settings_version_cache.clone(),
            shutdown_token.clone(),
        );
```

- [ ] **Step 3: Run quality gates**

```bash
cargo check --no-default-features --features db-sqlite 2>&1 | tail -5
cargo check --all-features 2>&1 | tail -5
cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | tail -5
cargo clippy --all-targets --all-features 2>&1 | tail -5
```

Expected: no errors, no warnings.

- [ ] **Step 4: Run full test suite**

```bash
cargo test --all-features 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/core/controller-runtime/src/lib.rs
git commit -m "feat(controller-runtime): wire db_rx to reconciler via DbPoolReloadable::subscribe()

Pool-size changes applied via DbPoolReloadable::apply() are now delivered
to the config reconciler, which re-reads the current handle on every tick.
Five long-lived initial-handle sites (audit backend, enricher, notification
dispatcher, surface proxy, AppState) remain on direct clones; deferred to a
follow-up since pool-size changes are rare and those sites do not poll.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

### Task 4: Document DB pool migration patterns in coding-standards.md

**Files:**

- Modify: `docs/development/coding-standards.md`

Add a "Database Pool Migration" subsection so future contributors know the two patterns: the watch-driven re-read pattern
(for long-lived polling consumers) and the initial-handle pattern (for one-shot startup consumers that will be migrated later).

- [ ] **Step 1: Locate the database section**

Open `docs/development/coding-standards.md` and find the section header for database query patterns (search for `Database` or `db`).

- [ ] **Step 2: Append the subsection**

Add the following after the existing SQLite transaction rules:

````markdown
#### Database Pool Migration

`DbPoolReloadable` owns a `tokio::sync::watch` channel that publishes replacement
`Arc<DbConnHandle>` values when the pool is reloaded. Two patterns apply:

**Watch-driven re-read** (for long-lived polling consumers):

```rust
// Receive a watch::Receiver<Arc<DbConnHandle>> from DbPoolReloadable::subscribe().
// Re-read the current handle on every iteration — never clone it outside the loop.
let handle = db_rx.borrow().clone(); // Arc clone; releases read lock immediately
let rows = MyEntity::find().all(handle.conn()).await?;
```

**Initial-handle** (for startup components that construct once):

```rust
// Clone the connection from the initial handle. This site uses the boot-time
// pool until the process restarts. Annotate with a TODO for future migration.
// TODO: migrate to watch::Receiver<Arc<DbConnHandle>> for live pool updates.
let db = db_rx.borrow().conn().clone();
```

Never hold `db_rx.borrow()` across an `.await` point — the read lock blocks
`watch::Sender::send()`. Clone the `Arc<DbConnHandle>` first, then drop the borrow.
````

- [ ] **Step 3: Check markdownlint**

```bash
npx markdownlint-cli2 docs/development/coding-standards.md
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add docs/development/coding-standards.md
git commit -m "docs(coding-standards): document DB pool migration patterns

Adds watch-driven re-read pattern and initial-handle pattern so contributors
know how to migrate spawn sites from bare DatabaseConnection clones to
watch::Receiver<Arc<DbConnHandle>> incrementally.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
````

---

## Self-Review

**Spec coverage:**

| Spec requirement                                                            | Task                        |
| --------------------------------------------------------------------------- | --------------------------- |
| `DbPoolReloadable::subscribe()` method                                      | Task 1                      |
| Reconciler migrated to `watch::Receiver`                                    | Task 2                      |
| `db_rx` wired in `run_server()` before `Arc::new(db_reloadable)`            | Task 3                      |
| `docs/development/coding-standards.md` pool patterns                        | Task 4                      |
| AppState.db migration (537 sites)                                           | Explicitly deferred in spec |
| Sites 2–5 (audit backend, enricher, notification dispatcher, surface proxy) | Explicitly deferred         |

**Placeholder scan:** None found.

**Type consistency:**

- `DbConnHandle` — `pub(crate)`, lives in `controller-runtime/src/reload/db_pool.rs` throughout;
  `crate::reload::db_pool::DbConnHandle` used explicitly in reconciler to avoid ambiguity.
- `watch::Receiver<Arc<DbConnHandle>>` parameter name `db_rx` used consistently across Tasks 2 and 3.
- `handle.conn()` — returns `&DatabaseConnection`; `.clone()` on the return gives an owned `DatabaseConnection`.
  Consistent with `DbConnHandle::conn()` signature in `db_pool.rs`.
