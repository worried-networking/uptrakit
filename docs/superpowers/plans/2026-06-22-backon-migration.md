# backon Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace all six `uptrakit_backoff` call sites with the `backon` crate and delete the in-house
`crates/shared/backoff/` crate entirely.

**Architecture:** Bounded retry-a-closure sites (version detection, NATS bootstrap, npm fetch) adopt backon's
`Retryable::retry().when().notify()` combinator, deleting hand-rolled `for` loops and two `last_err.expect(…)` guards.
Infinite reconnect loops (NATS consumer, MQTT client, service-SDK enrollment/reconnect) bind a backon
`ExponentialBuilder` (which is `Copy`), rebuild it to "reset", and advance with backon's native
`backoff.next().unwrap_or(cap)` (no `#[must_use]` wrapper). Migration is ordered so the workspace compiles and tests
pass at every commit.

**Tech Stack:** Rust, tokio (async runtime), `backon = 1.6.0`, `rootcause::Report` error handling, `cargo test` /
`cargo clippy` / `cargo deny`.

## Global Constraints

These apply to **every** task. Each task's requirements implicitly include this section.

- **Dependency pin:** `backon = { version = "1.6.0", default-features = false, features = ["std", "tokio-sleep"] }`
  (latest stable on crates.io; license `Apache-2.0`, already in `deny.toml` allow-list).
- **Attempt-count semantics (verified vs backon 1.6.0 source):** `(|| …).retry(builder.with_max_times(M))` performs
  **`M+1` total attempts** (1 initial call + M retries). To preserve a loop's existing total `T`, set
  `with_max_times(T-1)`. Attempt-count tests are the backstop — never trust the arithmetic alone.
- **`max_times` cliff:** backon's default `max_times = Some(3)`. Every _infinite_ reconnect builder MUST call
  `.without_max_times()`. A `nth(50).is_some()` guard test accompanies each infinite builder.
- **Jitter:** backon `with_jitter()` yields delay in `[current, 2·current)` (jitter `∈ [0, current)`, zero possible).
  Test bounds use `delay >= base` (closed lower bound), `delay < 2·base`.
- **Native backon idiom (no `#[must_use]` wrapper):** reconnect sites advance the backoff with
  `let delay = backoff.next().unwrap_or(<cap>);` directly — `<cap>` = the builder's `max_delay`. `.without_max_times()`
  guarantees `Some`, so `unwrap_or` is a panic-free defensive cap, never a real fallback. The old `escalate()`
  `#[must_use]` discipline is **dropped**, not reinvented.
- **Error handling (snapshot):** `rootcause::Report`, `report!`/`bail!`; no `unwrap()` in production code; prefer fixing
  root cause over `#[expect]`/`#[allow]` — deleted retry loops also delete their `#[expect(clippy::expect_used, …)]`
  attributes.
- **Locks:** `parking_lot` only in async code (not relevant here but do not introduce `std`/`tokio` mutexes).
- **`start_paused`:** add `#[tokio::test(start_paused = true)]` only to tests that drive a real `tokio::time::sleep`;
  tests that merely inspect returned `Duration` values, and the `nth(50)` cliff guard, must NOT set it.
- **Commit format:** Conventional Commits; end every commit message body with the trailer:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
- **Per-task verification (run before each commit):** `cargo fmt --all`, then the crate's
  `cargo clippy --all-targets --all-features -p <crate>` and `cargo test -p <crate> --all-features`. The final task runs
  the full workspace gate.

---

## File Structure

| File                                                                         | Responsibility                                                    | Touched in     |
| ---------------------------------------------------------------------------- | ----------------------------------------------------------------- | -------------- |
| `Cargo.toml` (root)                                                          | workspace dep: drop `uptrakit-backoff`, add `backon`              | Task 1, Task 8 |
| `crates/shared/service-sdk/src/lib.rs`                                       | add `reconnect_backoff_builder()`; later drop `Backoff` re-export | Task 1, Task 7 |
| `crates/shared/service-sdk/Cargo.toml`                                       | add `backon`; later drop `uptrakit-backoff`                       | Task 1, Task 7 |
| `crates/shared/agent-core/src/version_check.rs`                              | bounded retry → `.retry()`                                        | Task 2         |
| `crates/shared/nats/src/connection.rs`                                       | bounded bootstrap → `.retry()`                                    | Task 3         |
| `crates/plugins/package-managers/npm/src/releases.rs`                        | bounded fetch → `.retry().when()`                                 | Task 4         |
| `crates/ui/web-api/src/nats_transport.rs`                                    | infinite consumer loop → native backon iterator                   | Task 5         |
| `crates/core/mqtt-runtime/src/mqtt_client.rs`                                | infinite client loop → service-sdk builder                        | Task 6         |
| `crates/shared/service-sdk/src/lifecycle.rs`                                 | enrollment + reconnect loops → helpers; tests                     | Task 7         |
| `crates/shared/backoff/`                                                     | DELETE                                                            | Task 8         |
| `release-plz.toml`                                                           | drop `uptrakit-backoff` package + changelog entry                 | Task 8         |
| `docs/development/coding-standards.md`, `service-lifecycle.md`, `CONTEXT.md` | doc updates                                                       | Task 9         |

---

## Task 1: backon workspace dep + service-SDK reconnect builder

Adds the dependency and the shared `reconnect_backoff_builder()` factory (which encapsulates the easy-to-forget
`without_max_times()`), with unit tests. `uptrakit_backoff` stays fully intact — nothing else changes yet, so the
workspace still compiles.

**Files:**

- Modify: `Cargo.toml` (root, `[workspace.dependencies]`, line ~104)
- Modify: `crates/shared/service-sdk/Cargo.toml` (line ~29)
- Modify: `crates/shared/service-sdk/src/lib.rs` (line ~117)

**Interfaces:**

- Produces:
  - `uptrakit_service_sdk::reconnect_backoff_builder() -> backon::ExponentialBuilder` (2s base, 60s cap, jitter,
    `without_max_times`). Reconnect loops consume it natively: `let mut backoff = reconnect_backoff_builder().build();`,
    reset via `backoff = builder.build()`, advance via `backoff.next().unwrap_or(Duration::from_secs(60))`.

- [ ] **Step 1: Add `backon` to the workspace dependency table**

In root `Cargo.toml`, under `[workspace.dependencies]`, add (keep the existing `uptrakit-backoff` line for now):

```toml
backon = { version = "1.6.0", default-features = false, features = ["std", "tokio-sleep"] }
```

- [ ] **Step 2: Add `backon` to service-sdk**

In `crates/shared/service-sdk/Cargo.toml`, in `[dependencies]` (keep `uptrakit-backoff` for now):

```toml
backon = { workspace = true }
```

- [ ] **Step 3: Write the failing helper tests**

Append to the `#[cfg(test)] mod tests` block in `crates/shared/service-sdk/src/lib.rs` (create the module if absent):

```rust
#[cfg(test)]
mod backoff_helpers_tests {
    use super::reconnect_backoff_builder;
    use backon::BackoffBuilder;
    use std::time::Duration;

    #[test]
    fn builder_is_infinite_no_max_times_cliff() {
        // Guard the backon default `max_times = Some(3)` cliff: the reconnect
        // builder must never terminate.
        assert!(reconnect_backoff_builder().build().nth(50).is_some());
    }

    #[test]
    fn first_delay_in_base_band() {
        // First yield is min_delay + jitter ∈ [2s, 4s).
        let mut backoff = reconnect_backoff_builder().build();
        let d = backoff.next().expect("infinite iterator yields");
        assert!(d >= Duration::from_secs(2), "delay {d:?} below base");
        assert!(d < Duration::from_secs(4), "delay {d:?} above 2*base");
    }

    #[test]
    fn delay_escalates_and_caps_at_60s() {
        let mut backoff = reconnect_backoff_builder().build();
        // Pull many delays; the cap means none ever reaches 2*60s, and later
        // delays are >= the 60s cap floor.
        let mut last = Duration::ZERO;
        for _ in 0..20 {
            last = backoff.next().expect("infinite iterator yields");
            assert!(last < Duration::from_secs(120), "delay {last:?} exceeds 2*cap");
        }
        assert!(last >= Duration::from_secs(60), "capped delay {last:?} below cap floor");
    }
}
```

> Tests may call `.expect()` on `backoff.next()` (test code, panic is the desired failure mode). Production code uses
> `.unwrap_or(cap)` instead — see Global Constraints.

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p uptrakit-service-sdk backoff_helpers_tests 2>&1 | tail -20` Expected: FAIL —
`cannot find function reconnect_backoff_builder`.

- [ ] **Step 5: Implement the helpers**

In `crates/shared/service-sdk/src/lib.rs`, near the top-level items (alongside the existing
`pub use uptrakit_backoff::Backoff;` at line ~117 — leave that line in place for now), add:

```rust
use std::time::Duration;

/// Builder for the standard service reconnect/enrollment backoff: 2s base, 60s
/// cap, jittered, never terminates (infinite reconnect). `without_max_times()`
/// is mandatory — backon defaults to `max_times = Some(3)`, which would silently
/// stop a reconnect loop after three attempts — so it is encapsulated here once
/// rather than repeated at every call site.
///
/// Consume natively: `let mut backoff = reconnect_backoff_builder().build();`,
/// reset with `backoff = builder.build();`, advance with
/// `backoff.next().unwrap_or(Duration::from_secs(60))` (the `unwrap_or` cap is a
/// panic-free guard; `without_max_times()` means `next()` is always `Some`).
pub fn reconnect_backoff_builder() -> backon::ExponentialBuilder {
    backon::ExponentialBuilder::default()
        .with_min_delay(Duration::from_secs(2))
        .with_max_delay(Duration::from_secs(60))
        .with_jitter()
        .without_max_times()
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p uptrakit-service-sdk backoff_helpers_tests 2>&1 | tail -20` Expected: PASS (3 tests).

- [ ] **Step 7: Verify the workspace still compiles**

Run: `cargo check --all-features 2>&1 | tail -5` Expected: success (both `uptrakit-backoff` and `backon` present is
fine).

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add Cargo.toml Cargo.lock crates/shared/service-sdk/Cargo.toml crates/shared/service-sdk/src/lib.rs
git commit -m "feat(service-sdk): add backon dep + reconnect_backoff_builder

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: agent-core `version_check.rs` → backon `.retry()`

Bounded retry-a-closure. Deletes the `for 0..=max_retries` loop, the `last_error` accumulator, and the
`#[expect(clippy::expect_used, …)]` guard.

**Files:**

- Modify: `crates/shared/agent-core/Cargo.toml` (line ~27)
- Modify: `crates/shared/agent-core/src/version_check.rs` (lines 3, 18–22, 524–569)
- Test: same file (`mod tests`)

**Interfaces:**

- Consumes: nothing new (uses `backon` directly, not the service-sdk helper — this is a bounded site).
- Produces: `run_with_retry` keeps its existing signature
  (`label: &'static str, max_retries: u32, op: impl FnMut() -> Pin<Box<dyn Future<Output = PluginResult<T>> + Send>>`) →
  `Result<T, String>`.

Current loop (`0..=max_retries` ⇒ `max_retries + 1` total attempts). backon `with_max_times(max_retries)` ⇒
`max_retries + 1` total — **count preserved**.

- [ ] **Step 1: Write the failing attempt-count test**

Add to `crates/shared/agent-core/src/version_check.rs` test module (define a local always-retryable error helper if the
module lacks one):

```rust
#[tokio::test(start_paused = true)]
async fn run_with_retry_makes_max_retries_plus_one_attempts() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    let calls = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&calls);
    let result: Result<(), String> = run_with_retry("test", 2, move || {
        let c = Arc::clone(&c);
        Box::pin(async move {
            c.fetch_add(1, Ordering::SeqCst);
            // PluginError::TimedOut is retryable (is_retryable() == true), so every
            // attempt is consumed. Verified variants: error.rs is_retryable() returns
            // true for CommandSpawn/CommandFailed/CommandWait/TimedOut/CaptureFailed/PluginInternal.
            Err::<(), _>(rootcause::report!(
                uptrakit_plugin_infrastructure_core::PluginError::TimedOut
            ))
        })
    })
    .await;
    assert!(result.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 3, "2 retries + 1 initial = 3");
}
```

> `start_paused = true` is correct here: backon sleeps via `tokio::time::sleep`, and with `RETRY_BASE_DELAY = 5s` a real
> clock would make this a 15s test. Under the paused clock tokio **auto-advances** past each backoff sleep because no
> other task blocks the runtime — the test completes instantly, no manual `tokio::time::advance()` needed.

- [ ] **Step 2: Run the test to verify it passes against the old loop (regression lock)**

Run: `cargo test -p uptrakit-agent-core run_with_retry_makes_max_retries_plus_one_attempts 2>&1 | tail -20` Expected:
compiles against the _old_ `run_with_retry` and passes (old loop is also `max_retries+1`). This test is a **regression
lock** — it must keep passing after the rewrite. If it fails now, the helper signature changed unexpectedly; stop and
reconcile.

- [ ] **Step 3: Swap the dependency**

In `crates/shared/agent-core/Cargo.toml`, replace line 27:

```toml
backon = { workspace = true }
```

- [ ] **Step 4: Rewrite `run_with_retry` over backon**

In `version_check.rs`: change the import on line 3 from `use uptrakit_backoff::Backoff;` to
`use backon::{ExponentialBuilder, Retryable};`. Delete the `#[expect(clippy::expect_used, …)]` attribute (lines
524–527). Replace the function body (lines 528–569) with:

```rust
async fn run_with_retry<'a, T>(
    label: &'static str,
    max_retries: u32,
    op: impl FnMut() -> std::pin::Pin<
        Box<dyn std::future::Future<Output = PluginResult<T>> + Send + 'a>,
    >,
) -> Result<T, String> {
    let builder = ExponentialBuilder::default()
        .with_min_delay(RETRY_BASE_DELAY)
        .with_max_delay(RETRY_MAX_DELAY)
        .with_jitter()
        // max_retries retries after the first attempt = max_retries + 1 total,
        // preserving the previous `for 0..=max_retries` count.
        .with_max_times(max_retries as usize);

    op.retry(builder)
        .when(|e: &rootcause::Report<PluginError>| e.current_context().is_retryable())
        .notify(|e: &rootcause::Report<PluginError>, delay: std::time::Duration| {
            tracing::debug!(
                delay_ms = delay.as_millis() as u64,
                error = %e,
                label,
                "transient error, retrying",
            );
        })
        .await
        .map_err(|e| format!("{label} failed: {e}"))
}
```

> `op` is already `FnMut() -> Pin<Box<dyn Future<…>>>`, and `Pin<Box<dyn Future>>` implements `Future`, so it satisfies
> backon's `Retryable` bound (`FnMut() -> impl Future<Output = Result<…>>`) directly. `Retryable::retry` takes the
> closure by value (`self`), so pass `op` straight in — no `move ||` wrapper, no `mut`. (The `'a` lifetime and the
> `op: impl FnMut()` bound stay; only the body and the now-unneeded `mut` change.)

- [ ] **Step 5: Run the regression test + full crate tests**

Run: `cargo test -p uptrakit-agent-core --all-features 2>&1 | tail -20` Expected: PASS, including
`run_with_retry_makes_max_retries_plus_one_attempts`.

- [ ] **Step 6: Clippy (no `expect_used` suppression should remain)**

Run: `cargo clippy --all-targets --all-features -p uptrakit-agent-core 2>&1 | tail -20` Expected: clean — the
`#[expect(clippy::expect_used)]` is gone and no longer needed (no `.expect()` in the rewritten code).

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/shared/agent-core/Cargo.toml crates/shared/agent-core/src/version_check.rs Cargo.lock
git commit -m "refactor(agent-core): migrate version_check retry to backon

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: nats `connection.rs` → backon `.retry()`

Bounded bootstrap (`1..=MAX_ATTEMPTS` ⇒ 10 total). backon `with_max_times(MAX_ATTEMPTS - 1)` = 9 retries ⇒ 10 total.
Deletes the `last_err` accumulator + `#[expect(clippy::expect_used, …)]`.

**Files:**

- Modify: `crates/shared/nats/Cargo.toml` (line ~19)
- Modify: `crates/shared/nats/src/connection.rs` (import + lines 50–90)

**Interfaces:**

- Consumes: `backon::{ExponentialBuilder, Retryable}`.

- [ ] **Step 1: Swap the dependency**

In `crates/shared/nats/Cargo.toml`, replace line 19:

```toml
backon = { workspace = true }
```

- [ ] **Step 2: Rewrite the connect loop**

In `connection.rs`, change the `use uptrakit_backoff::Backoff;` import to
`use backon::{ExponentialBuilder, Retryable};`. Replace the whole `let client = 'connect: { … };` block (lines 53–90)
with:

```rust
let client = (|| async_nats::connect(url))
    .retry(
        ExponentialBuilder::default()
            .with_min_delay(Duration::from_secs(1))
            .with_max_delay(Duration::from_secs(30))
            .with_jitter()
            // 9 retries after the first = 10 total attempts (preserves MAX_ATTEMPTS).
            .with_max_times(MAX_ATTEMPTS - 1),
    )
    .notify(|e, delay| {
        tracing::warn!(
            url,
            delay_ms = delay.as_millis(),
            error = %e,
            "NATS connection attempt failed; retrying"
        );
    })
    .await
    .context_to::<NatsError>()?;
```

Keep `const MAX_ATTEMPTS: u32 = 10;` but change its type to `usize` (`with_max_times` takes `usize`), or cast:
`(MAX_ATTEMPTS - 1) as usize`. Prefer changing the const to `usize`. Delete the now-unused
`#[expect(clippy::expect_used, …)]` attribute and the `'connect` label.

> `.context_to::<NatsError>()?` replaces the old `return Err(last_err…).context_to::<NatsError>()`. Confirm
> `Report<async_nats::ConnectError>` (or whatever `connect` returns on error) has the `context_to` extension in scope
> via `rootcause::prelude::*` already imported at the top of the file.

- [ ] **Step 3: Build + test the crate**

Run: `cargo test -p uptrakit-nats --all-features 2>&1 | tail -20` Expected: PASS (no behavior tests depend on the exact
attempt count here; if one exists, it must still pass).

- [ ] **Step 4: Clippy**

Run: `cargo clippy --all-targets --all-features -p uptrakit-nats 2>&1 | tail -20` Expected: clean.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/shared/nats/Cargo.toml crates/shared/nats/src/connection.rs Cargo.lock
git commit -m "refactor(nats): migrate connect bootstrap retry to backon

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: npm `releases.rs` → backon `.retry().when()`

Bounded fetch (`1..=FETCH_MAX_RETRIES` ⇒ 3 total). `with_max_times(FETCH_MAX_RETRIES - 1)` = 2 retries ⇒ 3 total. Mixed
control flow: 404 → `Ok(vec![])` (terminal), other 4xx → terminal error (no retry), 5xx/transport → retryable error.

**Files:**

- Modify: `crates/plugins/package-managers/npm/Cargo.toml` (line ~13)
- Modify: `crates/plugins/package-managers/npm/src/releases.rs` (whole `fetch_releases`)
- Test: same file (`mod tests`)

**Interfaces:**

- Consumes: `backon::{ExponentialBuilder, Retryable}`; existing `PluginError::is_retryable()`.

- [ ] **Step 1: Write the failing branch tests**

Add to `crates/plugins/package-managers/npm/src/releases.rs` a `#[cfg(test)] mod tests` using a mock HTTP server (the
crate already uses `reqwest`; use `wiremock` if it is a dev-dependency, else `httpmock` — check the crate's
`[dev-dependencies]` and match the existing test style in the npm crate):

```rust
#[cfg(test)]
mod fetch_branch_tests {
    // 404 → Ok(empty), no retry.
    #[tokio::test]
    async fn not_found_returns_empty_without_retry() { /* mock 404, assert Ok(vec![]) and exactly 1 request */ }

    // Non-404 4xx → terminal error, no retry.
    #[tokio::test]
    async fn client_error_is_terminal_no_retry() { /* mock 403, assert Err and exactly 1 request */ }

    // 5xx → retried up to 3 total attempts, then error.
    #[tokio::test]
    async fn server_error_retries_three_times() { /* mock always-500, assert Err and exactly 3 requests */ }
}
```

> Use `httpmock` (already in `[workspace.dependencies]` as `httpmock = "0.8"`; `crates/plugins/infrastructure/proxmox`
> uses it the same way) — match its established pattern. The assertions on **request count** are the attempt-count
> backstop. **Do NOT use `start_paused = true` here:** the mock is a real HTTP server on a real socket, so a paused
> clock cannot compress its round-trips and would only risk auto-advance races. The base delay is
> `FETCH_BACKOFF_BASE = 1s`, so `server_error_retries_three_times` takes ~3s of real backoff — acceptable for a single
> retry test.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p <npm-crate-name> fetch_branch_tests 2>&1 | tail -20` (Resolve `<npm-crate-name>` from
`crates/plugins/package-managers/npm/Cargo.toml` `name =`.) Expected: FAIL (tests not yet matching the new control flow
/ mocks unset).

- [ ] **Step 3: Swap the dependency + add the test mock dep**

In `crates/plugins/package-managers/npm/Cargo.toml`, replace line 13:

```toml
backon = { workspace = true }
```

Add `httpmock` to that crate's `[dev-dependencies]` (it is already pinned in the root `[workspace.dependencies]` as
`httpmock = "0.8"`, so no root change is needed):

```toml
[dev-dependencies]
httpmock = { workspace = true }
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
```

(Keep any existing `[dev-dependencies]` entries; add only what is missing.)

- [ ] **Step 4: Rewrite `fetch_releases`**

Replace the import on line 3 (`use uptrakit_backoff::Backoff;`) with `use backon::{ExponentialBuilder, Retryable};`.
Replace the body (lines 13–96, keeping the `#[tracing::instrument]` attribute and the guard/url lines 14–17) with a
single retried closure. Map status to a retryable-or-terminal `PluginError`:

```rust
let builder = ExponentialBuilder::default()
    .with_min_delay(FETCH_BACKOFF_BASE)
    .with_max_delay(FETCH_BACKOFF_MAX)
    .with_jitter()
    // 2 retries after the first = 3 total attempts (preserves FETCH_MAX_RETRIES).
    .with_max_times(FETCH_MAX_RETRIES - 1);

let fetch = || async {
    let response = self
        .client
        .get(&url)
        .send()
        .await
        // Pre-HTTP transport error (TCP/TLS/DNS): retryable → PluginInternal (is_retryable() == true).
        .map_err(|e| report!(PluginError::PluginInternal(format!("npm registry request failed: {e}"))))?;

    let status = response.status();

    // 404 is a permanent "no releases" condition — terminal success.
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }
    // 5xx: registry overload / rate-limit — retryable → PluginInternal (is_retryable() == true).
    if status.is_server_error() {
        bail!(PluginError::PluginInternal(format!("npm registry returned HTTP {status}")));
    }
    // Other non-success (4xx) — terminal, do NOT retry → Configuration (is_retryable() == false).
    if !status.is_success() {
        bail!(PluginError::Configuration(format!("npm registry returned HTTP {status}")));
    }

    // Malformed body on a 2xx — terminal, retrying won't help → Serialization (is_retryable() == false).
    let json: serde_json::Value = response.json().await.map_err(|e| {
        report!(PluginError::Serialization(format!(
            "failed to parse npm registry response: {e}"
        )))
    })?;
    Ok(self.parse_registry_response(&json, package_identifier))
};

fetch
    .retry(builder)
    .when(|e: &rootcause::Report<PluginError>| e.current_context().is_retryable())
    .notify(|e, delay| {
        tracing::warn!(
            package = %package_identifier,
            delay_ms = delay.as_millis(),
            error = %e,
            "transient npm registry error; retrying"
        );
    })
    .await
```

> **Critical (verified against `crates/plugins/infrastructure/core/src/error.rs`):** `is_retryable()` returns **true**
> for `CommandSpawn | CommandFailed | CommandWait | TimedOut | CaptureFailed | PluginInternal`, and **false** for all
> others (`Configuration`, `Serialization`, `VersionParse`, `MissingConfig`, `MissingReleaseInfo`, `UnsupportedShell`,
> `UnsupportedOperation`, `InstallFailed`). So: transport + 5xx → `PluginInternal` (retried); terminal 4xx →
> `Configuration` and malformed-body → `Serialization` (both short-circuit `.when()` → no retry, preserving the old
> immediate-`bail!`/`?` behaviour). There is **no** `transient` constructor — do not use one. The 404 → `Ok(Vec::new())`
> path never enters `.when()` at all.

- [ ] **Step 5: Run the branch tests**

Run: `cargo test -p <npm-crate-name> --all-features 2>&1 | tail -20` Expected: PASS — 404 = 1 request + empty, 403 = 1
request + error, 500 = 3 requests + error.

- [ ] **Step 6: Clippy + commit**

```bash
cargo clippy --all-targets --all-features -p <npm-crate-name> 2>&1 | tail -20
cargo fmt --all
git add crates/plugins/package-managers/npm/Cargo.toml crates/plugins/package-managers/npm/src/releases.rs Cargo.lock
git commit -m "refactor(npm): migrate release fetch retry to backon

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: web-api `nats_transport.rs` → native backon iterator

Infinite consumer reconnect loop. `web-api` does **not** depend on `uptrakit-service-sdk`, and the consumer backoff uses
different params (1s/30s) than `reconnect_backoff_builder()` (2s/60s), so it constructs its own builder inline with
`.without_max_times()` and advances with `backoff.next().unwrap_or(Duration::from_secs(30))`.

**Files:**

- Modify: `crates/ui/web-api/Cargo.toml` (line ~82)
- Modify: `crates/ui/web-api/src/nats_transport.rs` (import + lines 181, 200–225)
- Test: same file (`mod tests`)

**Interfaces:**

- Consumes: `backon::{BackoffBuilder, ExponentialBuilder}`.

- [ ] **Step 1: Write the failing cliff-guard test**

Add to `nats_transport.rs`:

```rust
#[cfg(test)]
mod backoff_tests {
    use backon::{BackoffBuilder, ExponentialBuilder};
    use std::time::Duration;

    fn consumer_backoff_builder() -> ExponentialBuilder {
        ExponentialBuilder::default()
            .with_min_delay(Duration::from_secs(1))
            .with_max_delay(Duration::from_secs(30))
            .with_jitter()
            .without_max_times()
    }

    #[test]
    fn consumer_backoff_is_infinite() {
        assert!(consumer_backoff_builder().build().nth(50).is_some());
    }
}
```

> Extract `consumer_backoff_builder()` as a module-private `fn` in the file (not just in the test) so production and
> test share one definition — that is what the test should call.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p uptrakit-web-api backoff_tests 2>&1 | tail -20` Expected: FAIL — `consumer_backoff_builder` not
defined in module scope.

- [ ] **Step 3: Swap the dependency**

In `crates/ui/web-api/Cargo.toml`, replace line 82:

```toml
backon = { workspace = true }
```

- [ ] **Step 4: Add the local helper + builder, rewrite the loop**

In `nats_transport.rs`: change `use uptrakit_backoff::Backoff;` to `use backon::{BackoffBuilder, ExponentialBuilder};`.
Add the module-private builder factory (shared by production and the test):

```rust
fn consumer_backoff_builder() -> ExponentialBuilder {
    ExponentialBuilder::default()
        .with_min_delay(Duration::from_secs(1))
        .with_max_delay(Duration::from_secs(30))
        .with_jitter()
        .without_max_times()
}
```

Replace line 181 (`let mut backoff = Backoff::new(…);`) with:

```rust
let builder = consumer_backoff_builder();
let mut backoff = builder.build();
```

In the fetch-Ok arm (lines 200–204), replace `backoff.reset();` with `backoff = builder.build();` (keep the
`// reset chosen:` comment, updated to mention rebuild). In the fetch-Err arm (line 209), replace
`let delay = backoff.escalate();` with `let delay = backoff.next().unwrap_or(Duration::from_secs(30));` (keep the
`// escalate chosen:` comment; `without_max_times()` guarantees `Some`, the `unwrap_or` cap is a panic-free guard).
Leave both `tokio::select!` cancellation blocks unchanged.

- [ ] **Step 5: Run the test + crate tests**

Run: `cargo test -p uptrakit-web-api backoff_tests 2>&1 | tail -20` → PASS Run:
`cargo test -p uptrakit-web-api --all-features 2>&1 | tail -20` → PASS

- [ ] **Step 6: Clippy + commit**

```bash
cargo clippy --all-targets --all-features -p uptrakit-web-api 2>&1 | tail -20
cargo fmt --all
git add crates/ui/web-api/Cargo.toml crates/ui/web-api/src/nats_transport.rs Cargo.lock
git commit -m "refactor(web-api): migrate NATS consumer backoff to backon

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: mqtt-runtime `mqtt_client.rs` → service-SDK reconnect builder

Infinite client reconnect loop. `reconnect_backoff` is a **function parameter** (passed from `start()` into
`run_event_loop`), not a struct field. Thread the `ExponentialBuilder` (`Copy`) instead, rebuild on `ConnAck`.
mqtt-runtime currently has **no** `uptrakit-backoff` dep (it used the service-sdk re-export) — add `backon`.

**Files:**

- Modify: `crates/core/mqtt-runtime/Cargo.toml` (add `backon`)
- Modify: `crates/core/mqtt-runtime/src/mqtt_client.rs` (import line ~10, lines 311–320, 388, 440, 482)
- Test: same file (`mod tests`)

**Interfaces:**

- Consumes: `uptrakit_service_sdk::reconnect_backoff_builder` (Task 1); `backon::{BackoffBuilder, ExponentialBuilder}`.

- [ ] **Step 1: Write the failing cliff-guard test**

Add to `mqtt_client.rs`:

```rust
#[cfg(test)]
mod backoff_tests {
    use backon::BackoffBuilder;
    use uptrakit_service_sdk::reconnect_backoff_builder;

    #[test]
    fn mqtt_reconnect_backoff_is_infinite() {
        assert!(reconnect_backoff_builder().build().nth(50).is_some());
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p uptrakit-mqtt-runtime backoff_tests 2>&1 | tail -20` Expected: FAIL — `reconnect_backoff_builder`
unresolved (no `backon` dep / import yet).

- [ ] **Step 3: Add `backon` dependency**

In `crates/core/mqtt-runtime/Cargo.toml` `[dependencies]`:

```toml
backon = { workspace = true }
```

(`uptrakit-service-sdk` is already a dependency since the code used its `Backoff` re-export; confirm it is present.)

- [ ] **Step 4: Rewrite construction + loop**

Change the import on line ~10 from `use uptrakit_service_sdk::Backoff;` to:

```rust
use backon::{BackoffBuilder, ExponentialBuilder};
use uptrakit_service_sdk::reconnect_backoff_builder;
```

Replace line 311:

```rust
let reconnect_backoff_builder = reconnect_backoff_builder();
```

Pass the builder into the spawned task (lines 312–320): change the argument `reconnect_backoff` →
`reconnect_backoff_builder`. Change `run_event_loop`'s parameter (line 388) from `mut reconnect_backoff: Backoff` to:

```rust
reconnect_backoff_builder: ExponentialBuilder,
```

At the top of `run_event_loop`, build the iterator once:

```rust
let mut reconnect_backoff = reconnect_backoff_builder.build();
```

In the `ConnAck` arm (line 440), replace `reconnect_backoff.reset();` with
`reconnect_backoff = reconnect_backoff_builder.build();` (keep the `// reset chosen:` comment, updated). In the `Err(e)`
arm (line 482), replace `let delay = reconnect_backoff.escalate();` with
`let delay = reconnect_backoff.next().unwrap_or(Duration::from_secs(60));` (keep the `// escalate chosen:` comment;
`without_max_times()` guarantees `Some`, the `unwrap_or` cap is a panic-free guard). Leave the `tokio::select!`
cancellation unchanged.

- [ ] **Step 5: Run the test + crate tests**

Run: `cargo test -p uptrakit-mqtt-runtime backoff_tests 2>&1 | tail -20` → PASS Run:
`cargo test -p uptrakit-mqtt-runtime --all-features 2>&1 | tail -20` → PASS

> Note: `uptrakit-mqtt-runtime` sets `[lib] doctest = false`; it is excluded from the workspace doctest gate (Task 10).

- [ ] **Step 6: Clippy + commit**

```bash
cargo clippy --all-targets --all-features -p uptrakit-mqtt-runtime 2>&1 | tail -20
cargo fmt --all
git add crates/core/mqtt-runtime/Cargo.toml crates/core/mqtt-runtime/src/mqtt_client.rs Cargo.lock
git commit -m "refactor(mqtt-runtime): migrate reconnect backoff to backon builder

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: service-sdk `lifecycle.rs` → helpers; rewrite tests; drop re-export + dep

Highest-care site: enrollment + reconnect loops with three verbs, plus the bug-canary regression tests. After this task
**nothing** references `uptrakit_backoff`.

**Files:**

- Modify: `crates/shared/service-sdk/src/lifecycle.rs` (import; lines 331, 361–362, 383, 598, 607–608, 630, 682; tests
  ~817 onward)
- Modify: `crates/shared/service-sdk/src/lib.rs` (remove line 117 re-export)
- Modify: `crates/shared/service-sdk/Cargo.toml` (remove `uptrakit-backoff` line)

**Interfaces:**

- Consumes: `crate::reconnect_backoff_builder` (Task 1); `backon::BackoffBuilder`.

**Verb → backon mapping for this file** (`CAP = Duration::from_secs(60)`, `BASE = Duration::from_secs(2)`; `unwrap_or`
never fires — `without_max_times()` guarantees `Some` — it is a panic-free cap):

| Site                               | Old                                                  | New                                                                          |
| ---------------------------------- | ---------------------------------------------------- | ---------------------------------------------------------------------------- |
| enrollment construct (331)         | `Backoff::new(2s,60s)`                               | `let b = reconnect_backoff_builder(); let mut bo = b.build();`               |
| enrollment ReceiveClosed (361–362) | `reset()` + `sample_base_jitter()`                   | `bo = b.build(); let delay = bo.next().unwrap_or(BASE);` (rebuild = reset)   |
| enrollment TransientNetwork (383)  | `escalate()`                                         | `let delay = bo.next().unwrap_or(CAP);`                                      |
| reconnect construct                | `Backoff::new(2s,60s)`                               | `let b = reconnect_backoff_builder(); let mut rbo = b.build();`              |
| Ok(outcome) (598)                  | `reset()`                                            | `rbo = b.build();`                                                           |
| ReceiveClosed (607–608)            | `reset()` + `sample_base_jitter()`                   | `rbo = b.build(); let delay = rbo.next().unwrap_or(BASE);`                   |
| TransientNetwork (630)             | `escalate()`                                         | `let delay = rbo.next().unwrap_or(CAP);`                                     |
| Disconnected (682)                 | `sample_base_jitter()` (rebuild already done at 598) | `let delay = rbo.next().unwrap_or(BASE);` (no rebuild — 598 already rebuilt) |

- [ ] **Step 1: Rewrite the two existing verb regression tests (they must fail against unported code first)**

Locate the existing tests `enrollment_receive_closed_maps_to_reset_verb` (~817) and
`enrollment_transient_network_maps_to_escalate_verb`. Rewrite them to assert delay _bands_ on the new iterator model
instead of calling `Backoff::escalate/reset`:

```rust
// Canary: a receive-closed (healthy cycle) rebuilds → next delay is base band,
// even after prior escalations. (Test code uses .expect(); production uses
// .unwrap_or(cap) — see Global Constraints.)
#[test]
fn receive_closed_resets_to_base_band() {
    use backon::BackoffBuilder;
    use std::time::Duration;
    let b = reconnect_backoff_builder();
    let mut bo = b.build();
    // Escalate a few times.
    for _ in 0..5 { let _ = bo.next().expect("infinite"); }
    // Receive-closed path: rebuild.
    bo = b.build();
    let delay = bo.next().expect("infinite");
    assert!(delay >= Duration::from_secs(2) && delay < Duration::from_secs(4),
        "post-reset delay {delay:?} not in base band");
}

// Transient network escalates: second pull is strictly larger band than the first.
#[test]
fn transient_network_escalates() {
    use backon::BackoffBuilder;
    use std::time::Duration;
    let b = reconnect_backoff_builder();
    let mut bo = b.build();
    let _first = bo.next().expect("infinite");          // base band [2,4)
    let second = bo.next().expect("infinite");          // 2*base band [4,8)
    assert!(second >= Duration::from_secs(4), "escalated delay {second:?} did not grow");
}
```

- [ ] **Step 2: Add the new behaviour tests**

```rust
// sample-base-jitter: after rebuild + one pull, a subsequent pull escalates
// from base, not from a consumed cursor.
#[test]
fn sample_then_escalate_from_base() {
    use backon::BackoffBuilder;
    use std::time::Duration;
    let b = reconnect_backoff_builder();
    let mut bo = b.build();
    let sampled = bo.next().expect("infinite");         // base band
    assert!(sampled < Duration::from_secs(4));
    let escalated = bo.next().expect("infinite");       // 2*base band
    assert!(escalated >= Duration::from_secs(4));
}

#[test]
fn reconnect_backoff_is_infinite() {
    use backon::BackoffBuilder;
    assert!(reconnect_backoff_builder().build().nth(50).is_some());
}
```

- [ ] **Step 3: Run the tests to verify they fail to compile (old `Backoff` API still imported)**

Run: `cargo test -p uptrakit-service-sdk receive_closed_resets_to_base_band 2>&1 | tail -20` Expected:
FAIL/compile-error until the loops + imports are ported (the test references the helpers, not `Backoff`).

- [ ] **Step 4: Port both loops per the mapping table**

In `lifecycle.rs`: replace the `Backoff` import with
`use crate::reconnect_backoff_builder; use backon::BackoffBuilder;`. Apply each row of the mapping table above. Update
each `// reset chosen:` / `// escalate chosen:` comment to describe the rebuild / `next().unwrap_or(cap)` mechanics
while keeping the rationale. For the `Disconnected` arm, **do not** rebuild — line 598's `rbo = b.build()` already did
(preserve that ordering exactly).

- [ ] **Step 5: Remove the re-export and the dependency**

In `crates/shared/service-sdk/src/lib.rs`, delete line 117 `pub use uptrakit_backoff::Backoff;`. In
`crates/shared/service-sdk/Cargo.toml`, delete the `uptrakit-backoff = { workspace = true }` line (keep `backon`).

- [ ] **Step 6: Run the full crate suite**

Run: `cargo test -p uptrakit-service-sdk --all-features 2>&1 | tail -25` Expected: PASS (all four new/rewritten tests +
existing suite).

- [ ] **Step 7: Confirm nothing else imports the re-export**

Run: `grep -rn "service_sdk::Backoff\|uptrakit_backoff" crates/ --include=*.rs` Expected: empty (every call site
ported).

- [ ] **Step 8: Clippy + commit**

```bash
cargo clippy --all-targets --all-features -p uptrakit-service-sdk 2>&1 | tail -20
cargo fmt --all
git add crates/shared/service-sdk/Cargo.toml crates/shared/service-sdk/src/lib.rs crates/shared/service-sdk/src/lifecycle.rs Cargo.lock
git commit -m "refactor(service-sdk): migrate lifecycle backoff to backon; drop Backoff re-export

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: Delete the `uptrakit-backoff` crate + wiring

With every call site ported, remove the crate and all references.

**Files:**

- Delete: `crates/shared/backoff/` (whole directory)
- Modify: `Cargo.toml` (root, remove line ~104)
- Modify: `release-plz.toml` (remove `[[package]] name = "uptrakit-backoff"` block + the `"uptrakit-backoff",`
  `changelog_include` entry)

- [ ] **Step 1: Delete the crate directory**

Run: `git rm -r crates/shared/backoff` (The workspace `members` use the `crates/shared/*` glob, so no `members` edit is
needed.)

- [ ] **Step 2: Remove the workspace dependency**

In root `Cargo.toml`, delete line ~104:

```toml
uptrakit-backoff = { path = "crates/shared/backoff", version = "0.0.1" }
```

- [ ] **Step 3: Remove the release-plz entries**

In `release-plz.toml`, delete the three-line `[[package]]` block with `name = "uptrakit-backoff"` and the
`"uptrakit-backoff",` line inside the `changelog_include = [ … ]` array.

- [ ] **Step 4: Verify no references remain anywhere**

```bash
grep -rn "uptrakit_backoff\|uptrakit-backoff" . \
  --exclude-dir=.git --exclude-dir=target --exclude-dir=.claude \
  --exclude-dir=.superpowers | grep -v 'docs/superpowers/'
```

Expected: empty (the only legitimate remaining hits are historical `docs/superpowers/{plans,specs}` artifacts,
intentionally untouched).

- [ ] **Step 5: Full workspace build + lockfile update**

Run: `cargo check --all-features 2>&1 | tail -5` Expected: success; `Cargo.lock` drops `uptrakit-backoff`.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "chore(backoff): delete uptrakit-backoff crate after backon migration

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: Documentation updates

**Files:**

- Modify: `docs/development/coding-standards.md` (the `service-reconnect-backoff` section)
- Modify: `docs/development/service-lifecycle.md` (if it documents reconnect verb behaviour)
- Modify: `CONTEXT.md` (only if it enumerates `crates/shared/backoff`)

- [ ] **Step 1: Rewrite the coding-standards backoff section**

In `docs/development/coding-standards.md`, find the `service-reconnect-backoff` section (it currently mandates
`uptrakit_backoff::Backoff` with reset/escalate verb comments and the `escalate()` `#[must_use]` rule). Replace it with
backon guidance:

- Bounded retries: `backon`'s `Retryable::retry().when(pred).notify(…)` combinator; remember `with_max_times(M)` = `M+1`
  total attempts.
- Reconnect loops: `uptrakit_service_sdk::reconnect_backoff_builder().build()`; "reset" = rebuild the iterator
  (`backoff = builder.build()`); advance = `backoff.next().unwrap_or(cap)`. No `#[must_use]` wrapper — the old
  `escalate()` `#[must_use]` rule is **dropped**.
- Infinite loops MUST use `.without_max_times()`; ship a `nth(N).is_some()` guard test.

- [ ] **Step 2: Update service-lifecycle doc**

Run: `grep -n "Backoff\|backoff\|escalate\|reset" docs/development/service-lifecycle.md`. **Required:** line ~641 has a
public-API table row for the `Backoff` symbol ("Configurable exponential backoff (no async)") describing the
`uptrakit_service_sdk::Backoff` re-export removed in Task 7 — **delete that row** (the symbol no longer exists). If any
other passage describes the reconnect verb behaviour as current state, update it to the iterator-rebuild model.

- [ ] **Step 3: CONTEXT.md conditional**

Run: `grep -n "backoff" CONTEXT.md` If `crates/shared/backoff` is listed in a crate map, remove that entry. Otherwise no
change.

- [ ] **Step 4: Lint the markdown**

Run:
`markdownlint --config .markdownlint.json docs/development/coding-standards.md docs/development/service-lifecycle.md CONTEXT.md 2>&1 | tail`
Expected: clean (use `npx prettier --write` on the files first if alignment issues arise, per project convention).

- [ ] **Step 5: Commit**

```bash
git add docs/development/coding-standards.md docs/development/service-lifecycle.md CONTEXT.md
git commit -m "docs(backoff): document backon usage; retire uptrakit_backoff guidance

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 10: Full quality-gate sweep

Final verification across the whole workspace (the per-task gates only covered single crates).

- [ ] **Step 1: Run the complete gate suite**

```bash
cargo fmt --all -- --check
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo test --no-default-features --features db-sqlite
cargo test --workspace --all-features --doc --exclude uptrakit-mqtt-runtime
cargo deny check
python3 ci/check_plugin_semantic_boundary.py
bash ci/verify_no_security_audit.sh
bash ci/verify_typed_audit_actions.sh
bash ci/verify_handler_state_contract.sh
python3 ci/verify_db_access_policy.py
markdownlint --config .markdownlint.json '**/*.md'
```

Expected: all pass. `cargo deny check` validates `backon` (`Apache-2.0`) and its transitive `fastrand`
(`Apache-2.0 OR MIT`) — both in the allow-list. The four `ci/verify_*` gates are part of the standard full suite
(`quality-gates.md`); this migration does not touch audit/handler/DB-access paths, so they should pass unchanged — run
them to confirm no incidental regression.

- [ ] **Step 2: Final reference sweep**

```bash
grep -rn "uptrakit_backoff\|uptrakit-backoff" . \
  --exclude-dir=.git --exclude-dir=target --exclude-dir=.claude \
  --exclude-dir=.superpowers | grep -v 'docs/superpowers/'
```

Expected: empty.

- [ ] **Step 3: If any gate produced fixes, commit them**

```bash
cargo fmt --all
git add -A
git commit -m "chore(backoff): quality-gate fixups after backon migration

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

(If no fixes were needed, skip this commit.)

---

## Self-Review

**Spec coverage:** all six call sites (Tasks 2–7), crate deletion + release-plz (Task 8), shared
`reconnect_backoff_builder` (Task 1), attempt-count preservation (Tasks 2–4 with tests), jitter band tests (Tasks 1, 7),
cliff guards (Tasks 1, 5, 6, 7), docs (Task 9), quality gates incl. deny/doctest/semantic-boundary (Task 10). The spec's
"no new ADR" decision is honoured (no ADR task). Decision 5 (no `#[must_use]` wrapper — native `next().unwrap_or(cap)`)
is reflected at every reconnect site. Deferred items (crates.io yank, policy changes) are out of scope and not tasked —
correct.

**Placeholder scan:** the npm mock-test bodies (Task 4 Step 1) and the `<npm-crate-name>` token are deliberate impl-time
lookups (mock framework + crate name vary); each is flagged with the exact file to resolve them from. The
`PluginError::transient` constructor name is flagged for verification against `error.rs:56`. No silent TODOs.

**Type consistency:** `reconnect_backoff_builder() -> ExponentialBuilder` is consumed identically in Tasks 5/6/7
(`build()` → `backoff.next().unwrap_or(cap)`); Task 5 builds its own 1s/30s `ExponentialBuilder` inline (different
params). `with_max_times` argument is `usize` throughout. The `Disconnected`-arm no-double-rebuild rule is stated in
both the mapping table and Task 7 Step 4.
