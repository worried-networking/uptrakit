# Backoff Guard API Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `Backoff::next_delay()` / `Backoff::reset()` with a `#[must_use]` consuming `AttemptGuard` (verbs: `reset` / `escalate`; warn-only
`Drop` with **no state mutation**). Fix the enrollment-loop reset-on-partial-success bug at `lifecycle.rs:331` via splitting the collapsed
`is_receive_closed_report || is_transient_network_report` arm into two predicate-specific arms. Publish `uptrakit-backoff` 0.0.1 → 0.1.0 to crates.io.
Drop the duplicate copy in `service-sdk`. Audit the other 6 backoff consumers for analogous bugs and migrate.

**Architecture:** Single sync std-only library crate (`uptrakit-backoff`). `Backoff` owns `current/base/max: Duration`;
`attempt(&mut self) -> AttemptGuard<'_>` returns a guard borrowing `&mut Backoff` so only one attempt can be live at a time. Guard verbs consume
`self` to advance/reset state. `Drop` impl emits `tracing::warn!` with **no state mutation** (avoids recreating the original symptom on unrelated `?`
paths). Three loudness layers in priority order: (1) workspace lints `unused_must_use`, `unused_variables`, `clippy::let_underscore_must_use = "deny"`
close the common forgot-to-resolve cases at compile time; (2) Task 4.8 syn-AST workspace test forbids `?` between `attempt()` and resolution; (3) the
`Drop` warn record is the runtime backstop. Per-call-site classification tests enforce verb-to-error mapping correctness.

**Tech Stack:** Rust 2024 / sync std-only (no async, no tokio runtime coupling for the backoff crate itself). Migration sites use tokio:
`tokio::time::sleep`, `tokio::select!`, `tracing`. Tests use `tokio::test` and `tracing-subscriber` with a non-reentrant subscriber for `Drop` warn
capture.

**Spec:** `docs/superpowers/specs/2026-06-10-backoff-guard-api-design.md` (commit `c4999a5e4`).

**Commit invariant:** **every commit on the branch must `cargo check --all-features` green.** Phases 1, 3, and 4 are tightly coupled because the
library API rewrite (Phase 1), the service-sdk dedup (Phase 3), and all 7 call-site migrations (Phase 4) break or restore the workspace's compile
state in lockstep — landing them as separate commits leaves intermediate non-compiling commits that break `git bisect` and reviewer checkout.
**Either** land Phases 1+3+4 as one atomic commit (recommended), **or** reorder so each commit individually keeps the workspace compiling. Phase 2
(publishing config) and Phase 5 (docs) can be separate commits since they don't affect compile.

## Phase 1 — Rewrite `uptrakit-backoff` library

### Task 1.1 — Replace `Backoff` API with guard pattern

- [ ] Open `crates/shared/backoff/src/lib.rs`.
- [ ] Add `#![warn(missing_docs)]` at the crate root (per snapshot rule: published crate must expose rustdoc).
- [ ] Add `#[non_exhaustive]` to the `Backoff` struct (snapshot Binding Rule: extensible public structs in shared crates).
- [ ] Rewrite `impl Backoff`:
  - Keep `pub fn new(base: Duration, max: Duration) -> Self`.
  - Add `pub fn attempt(&mut self) -> AttemptGuard<'_>`.
  - **Delete** `pub fn next_delay(&mut self) -> Duration`.
  - **Delete** `pub fn reset(&mut self)`.
  - **Do NOT add `Backoff::base()` accessor** (earlier draft proposed it; YAGNI — see Task 4.2 for the two-guard pattern that preserves jitter on
    `LoopOutcome::Disconnected` without needing a base getter).
- [ ] Add the `AttemptGuard<'a>` type:

  ```rust
  #[must_use = "AttemptGuard must be resolved via .reset() or .escalate()"]
  pub struct AttemptGuard<'a> {
      backoff: &'a mut Backoff,
      resolved: bool,
  }
  ```

- [ ] Implement guard methods (all consume `self`):
  - `pub fn sample_delay(&self) -> Duration` — read-only; samples `current + jitter`. Jitter re-sampled per call (document this; the `sample_` prefix
    is deliberate).
  - `pub fn reset(mut self)` — sets `resolved = true`, sets `self.backoff.current = self.backoff.base`. Caller picks `reset` whenever the cycle was
    healthy (Ok or Err-with-meaningful-progress).
  - `pub fn escalate(mut self)` — sets `resolved = true`, sets `self.backoff.current = (self.backoff.current * 2).min(self.backoff.max)`. Caller picks
    `escalate` when the cycle was unhealthy (fast-fail, no progress).
- [ ] Implement `Drop`:

  ```rust
  impl Drop for AttemptGuard<'_> {
      fn drop(&mut self) {
          if !self.resolved && !std::thread::panicking() {
              tracing::warn!("backoff guard dropped unresolved (state unchanged); resolve before any ? or early-return between attempt() and resolution");
          }
      }
  }
  ```

  **No state mutation in release** — deliberate. Escalating on every unresolved drop would recreate the user-visible symptom (inflated delay after a
  healthy cycle) under any `?`-driven early-return that the lints miss.

  **No `debug_assert!` either** — the workspace no-`panic!` Binding Rule doesn't carve out for `debug_assert!`; even if it did, the assertion is
  redundant with Task 4.8's compile-time lint that catches the same pattern statically. Loudness lives in three places: (a) compile-time lints
  (`unused_must_use`, `let_underscore_must_use`); (b) Task 4.8 syn-AST test; (c) the dedicated unit test
  `dropping_unresolved_guard_warns_and_does_not_mutate_state` that asserts both the warn record and the no-mutation invariant.

### Task 1.2 — Rustdoc on every public item

- [ ] Document `Backoff` with: what it models (exponential backoff with jitter), construction example, link to `AttemptGuard`.
- [ ] Document `Backoff::new` and `Backoff::attempt`.
- [ ] Document `AttemptGuard` with: the contract ("must be resolved"), the two verbs and when to use each (`reset` = the cycle was healthy — work
      returned Ok OR work returned Err after reaching a meaningful application-level milestone; `escalate` = the cycle was unhealthy — no progress
      made before failure), and the Drop semantics.
- [ ] Document `sample_delay` warning: jitter re-sampled per call; store the result once. Example pattern.
- [ ] Document both resolution methods (`reset`, `escalate`) with one-paragraph rationale each.
- [ ] Run `cargo doc -p uptrakit-backoff` and inspect output; ensure `#![warn(missing_docs)]` produces no warnings.

### Task 1.3 — Replace library tests

- [ ] **Delete** existing tests in `crates/shared/backoff/src/lib.rs`:
  - `doubling_behaviour`, `max_cap`, `reset_returns_to_base`, `zero_base_does_not_panic`.
- [ ] Add new tests (spec §Tests list):
  - `attempt_reset_sets_current_to_base`.
  - `attempt_escalate_doubles_with_cap`.
  - `dropping_unresolved_guard_warns_and_does_not_mutate_state` — use `tracing-subscriber` with a non-reentrant subscriber (e.g.
    `tracing_subscriber::fmt::Layer` with a `MakeWriter` channeling to a `std::sync::mpsc` channel). Drop an unresolved guard. Assert state unchanged
    AND `warn!` record captured. With `debug_assert!` removed (Task 1.1), no `catch_unwind` is needed.
  - `dropping_unresolved_guard_during_panic_does_not_warn` — `std::panic::catch_unwind(|| { let g = b.attempt(); panic!("inner"); });` — verify outer
    catch_unwind returns Err AND no `warn` record was emitted (the `!std::thread::panicking()` guard suppresses logging during unwind).
  - `sample_delay_does_not_advance_state` — call multiple times, observe values in expected range, state unchanged after.
  - `bug_regression_reset_at_cap_returns_base` — repeated `escalate()` until `current == max`; then `reset()`; then a fresh `attempt().sample_delay()`
    returns in the base+jitter range.
  - `borrow_checker_prevents_concurrent_guards` — `compile_fail` doc test holding two `attempt()` guards simultaneously.
- [ ] Run `cargo test -p uptrakit-backoff` until green.

### Task 1.4 — Bump version and add `publish = true`

- [ ] In `crates/shared/backoff/Cargo.toml`:
  - Change `version = "0.0.1"` → `version = "0.1.0"`.
  - Add `publish = true` after the version line (matches `crates/shared/build-info/Cargo.toml:9`, `wire/Cargo.toml:9`, `service-sdk/Cargo.toml:9`).
- [ ] Verify with `cargo metadata --no-deps -p uptrakit-backoff --format-version 1 | jq '.packages[0] | {version, publish}'` — should show `"0.1.0"`
      and `null` (cargo represents `publish = true` as `null` in metadata).

## Phase 2 — Publishing configuration

### Task 2.1 — Update `release-plz.toml`

- [ ] Open `/Users/andreyyantsen/Development/uptrakit/release-plz.toml`.
- [ ] **Delete** the existing `uptrakit-backoff` entry at lines 76–78:

  ```toml
  [[package]]
  name = "uptrakit-backoff"
  release = false
  ```

- [ ] **Add** a new entry in the "Public-API library crates" section (around lines 165–193, alongside `uptrakit-build-info`):

  ```toml
  [[package]]
  name = "uptrakit-backoff"
  git_release_enable = false
  publish = true
  ```

- [ ] In the `uptrakit-service-sdk` entry's `changelog_include` array (lines 663–667), add `"uptrakit-backoff"`:

  ```toml
  changelog_include = [
    "uptrakit-wire",
    "uptrakit-shared-types",
    "uptrakit-surfaces",
    "uptrakit-backoff",
  ]
  ```

- [ ] Run `cargo test -p uptrakit-functional-tests release_config` (the release-plz self-consistency test) and confirm it still passes.

### Task 2.2 — Add crates.io README

- [ ] Create `crates/shared/backoff/README.md` (cargo auto-discovers; no `readme =` field needed in Cargo.toml).
- [ ] Content:
  - One-paragraph crate purpose ("Exponential backoff with jitter for reconnect loops; the guard pattern forces explicit resolution of each attempt so
    partial-success bugs become loud").
  - `Cargo.toml` snippet: `uptrakit-backoff = "0.1"`.
  - Three example snippets: (a) success-on-Ok pattern, (b) partial-progress pattern (the headline fix), (c) bounded retry pattern.
  - Link to `https://docs.rs/uptrakit-backoff` for full API.
  - MIT / Apache-2.0 license note (matches workspace).
- [ ] Verify `cargo package -p uptrakit-backoff --allow-dirty` succeeds and the produced tarball contains the README.

## Phase 3 — Drop the duplicate copy from `service-sdk`

### Task 3.1 — Delete in-tree copy and rewire

- [ ] Delete `crates/shared/service-sdk/src/backoff.rs`.
- [ ] In `crates/shared/service-sdk/src/lib.rs`:
  - Remove `pub mod backoff;`.
  - Replace `pub use backoff::Backoff;` with `pub use uptrakit_backoff::{Backoff, AttemptGuard};`.
- [ ] In `crates/shared/service-sdk/Cargo.toml`:
  - Add `uptrakit-backoff = { workspace = true }` under `[dependencies]` (workspace dep already exists per `Cargo.toml:104`).
  - Remove `rand = { workspace = true }` from `[dependencies]` — confirmed via grep that the deleted `backoff.rs` is the only `rand` consumer in
    service-sdk.
- [ ] Run `cargo check -p uptrakit-service-sdk` — should compile (re-export resolves through `uptrakit_backoff::Backoff`).

## Phase 4 — Migrate the 7 call sites + audit decisions

> **Migration rule** (applies to every call site below): construct the guard at the point of decision (after classifying the outcome), resolve before
> sleeping. Never hold a guard across a `?` or an early-return that would drop it unresolved. Cancellation arms inside `tokio::select!` fire AFTER the
> guard has been resolved, so no `cancel()` verb is needed.
>
> **Audit decisions baked in**: every non-enrollment site below has its `reset` vs `escalate` verb pre-decided with stated rationale. Tasks include
> the literal `// <verb> chosen: <reason>` inline comment to leave in the code. Mqtt and nats use `escalate()` uniformly — initial drafts proposed
> `had_connack`/`had_successful_fetch` flags, rejected because rumqttc/async_nats handle internal reconnect before surfacing Err and a reset-on-Err
> would stampede a recovering broker (same logic as the npm 5xx case). `reset` is reserved for sites with an explicit close-code signal that the cycle
> was healthy — only the enrollment loop and the reconnect loop's `ReceiveClosed` arm qualify.
>
> Task 4.8 adds a workspace lint that forbids `?` between `attempt()` and resolution — without this enforcement a future refactor could silently
> reintroduce the unresolved-drop bug class that release builds only `warn!` about.

### Task 4.1 — Bug fix at `lifecycle.rs:331` enrollment loop

- [ ] In `crates/shared/service-sdk/src/lifecycle.rs` around lines 330–376:
  - Replace the collapsed `if is_receive_closed_report(&e) || is_transient_network_report(&e)` arm with two separate arms (this is the actual bug
    fix).
  - `is_receive_closed_report(&e)` arm: `let guard = enrollment_backoff.attempt(); let delay = guard.sample_delay(); guard.reset();` then the
    interruptible sleep; `continue;`.
  - `is_transient_network_report(&e)` arm: identical structure but `guard.escalate();`.
  - `is_cancelled_report(&e)` arm: `return Ok(())` — no guard constructed.
  - Catch-all `Err(e)` arm (fatal): `return Err(e)` — no guard constructed.
- [ ] Verify with `cargo check -p uptrakit-service-sdk`.
- [ ] Add classification test. **Location rule** (applies to every Phase 4 classification test): if the target file already has an inline
      `#[cfg(test)] mod tests { ... }` block, add the test there. If not, create a sibling integration-test file under the crate's `tests/` directory
      following the existing `crates/shared/service-sdk/tests/no_workspace_db_deps.rs` pattern. Do not introduce a third pattern. For `lifecycle.rs`:
      inspect the file first; today it has an inline `#[cfg(test)] mod tests` block at the bottom — add the test there.
  - Construct an `EnrollmentError` of each relevant variant.
  - Assert that for `ReceiveClosed` the loop calls `reset` (observable via `enrollment_backoff.attempt().sample_delay()` returning base-range value).
  - Assert that for a pre-upgrade `TransientNetwork` variant the loop calls `escalate` (observable via cap-range value after escalation).
  - Note: if `do_enrollment` is hard to mock, factor the classification into a small
    `classify_enrollment_error(&Report<EnrollmentError>) -> EnrollmentOutcome` helper enum (`Succeeded` / `PartialProgress` / `Failed` / `Fatal`) and
    unit-test the helper. The loop body is then `match classify(...) { ... }`.

### Task 4.2 — Migrate `lifecycle.rs:481` reconnect loop

- [ ] In `crates/shared/service-sdk/src/lifecycle.rs` around lines 555–645:
  - **`Ok(outcome)` arm:** replace `reconnect_backoff.reset();` with `reconnect_backoff.attempt().reset();` on entry.
  - **`Err(e)` arm:** split `LoopError::TransientNetwork(_) | LoopError::ReceiveClosed` into two arms:
    - `LoopError::ReceiveClosed` → guard + `reset()`.
    - `LoopError::TransientNetwork(_)` → guard + `escalate()`.
  - **`LoopOutcome::Disconnected` arm** (line 625): replace `let delay = reconnect_backoff.next_delay();` with a two-guard pattern that preserves
    today's `base + jitter` semantics (today's `next_delay()` after `reset()` returns `base + jitter`; a bare `Backoff::base()` accessor would drop
    the jitter):

    ```rust
    let guard = reconnect_backoff.attempt();
    let delay = guard.sample_delay(); // base + jitter (current was reset above)
    guard.reset();                // keep state at base — no escalation for clean disconnect
    tokio::select! { … sleep(delay) … signal … }
    ```

  - **`LoopOutcome::Reconnect` and `LoopOutcome::Shutdown` and `LoopOutcome::Restart` arms:** unchanged (no backoff usage).

- [ ] Verify `cargo check -p uptrakit-service-sdk`.
- [ ] Add classification test (same helper-enum trick if needed) for `LoopError::ReceiveClosed` → `reset`; `LoopError::TransientNetwork` → failed.

### Task 4.3 — Migrate `mqtt_client.rs:439, 478`

**Audit decision** (pre-decided; document inline at migration site):

- `escalate()` for the poll-Err arm. No `had_connack` flag.
- Reasoning: rumqttc retries connect/reconnect internally before surfacing an `Err` from `EventLoop::poll()`. By the time the Err reaches our arm,
  rumqttc has already given up on its internal retries. A "had ConnAck this session" flag can be many minutes stale — resetting backoff to base 2 s on
  the next attempt would stampede a recovering broker. Same logic as the npm 5xx case: broker/registry overload should accumulate the backoff hint,
  not reset it.

Tasks:

- [ ] Line 439 (`Packet::ConnAck` arm): replace `reconnect_backoff.reset();` with `reconnect_backoff.attempt().reset();`. ConnAck is the clean success
      signal; only the API form changes.
- [ ] Line 478 (poll-Err arm): replace `let delay = reconnect_backoff.next_delay();` with:

  ```rust
  let guard = reconnect_backoff.attempt();
  let delay = guard.sample_delay();
  // escalate chosen: rumqttc retries connect internally; a surfaced Err means it gave up.
  // Resetting backoff would stampede a recovering broker; preserve the accumulated escalation.
  guard.escalate();
  ```

- [ ] Verify `cargo check -p uptrakit-mqtt-runtime`.
- [ ] Add classification test in the mqtt-runtime test module asserting `escalate()` is the verb called on the poll-Err arm.

### Task 4.4 — Migrate `nats_transport.rs:201, 205`

**Audit decision** (pre-decided; document inline at migration site):

- `escalate()` for the fetch-Err arm. No `had_successful_fetch` flag.
- Reasoning: same shape as mqtt. async_nats handles JetStream connection internally; a fetch-Err surfaces after async_nats has exhausted its own retry
  budget. A "had successful fetch this session" flag is stale signal once we're seeing Errs, and resetting backoff to base 1 s would stampede the
  JetStream server on its way back up. Preserve the escalation hint.

Tasks:

- [ ] Line 201 (Ok branch): replace `backoff.reset();` with `backoff.attempt().reset();`. Successful fetch is the clean success signal.
- [ ] Line 205 (Err branch): replace `let delay = backoff.next_delay();` with:

  ```rust
  let guard = backoff.attempt();
  let delay = guard.sample_delay();
  // escalate chosen: async_nats handles internal reconnect; a surfaced fetch-Err means it gave up.
  // Resetting backoff would stampede a recovering JetStream server.
  guard.escalate();
  ```

- [ ] Verify `cargo check -p uptrakit-web-api`.
- [ ] Add classification test asserting `escalate()` is called on the Err branch.

### Task 4.5 — Migrate `nats/connection.rs:54`

**Audit decision** (pre-decided; document inline at migration site):

- Bounded `MAX_ATTEMPTS = 10` startup loop. Each `async_nats::connect` either returns a `Client` or errors. Possible failure modes: DNS/TCP refused
  (pre-connection), TLS handshake fail (mid-connection), auth fail (post-connection). None of these distinctions matter behaviorally — the function
  exits after `MAX_ATTEMPTS` regardless of verb choice, and the next caller restart begins a fresh bounded loop. Within a 10-attempt window the
  partial-progress reset is academic.
- Use `escalate()` for every attempt. Single verb keeps the bounded loop's intent obvious.

Tasks:

- [ ] In `crates/shared/nats/src/connection.rs` at the bounded loop (lines 56–74), restructure so the guard is constructed once at the top of each
      iteration before the `match`, then resolved in both arms (today's code only touches backoff on Err — restructure needed):

  ```rust
  for attempt in 1..=MAX_ATTEMPTS {
      let guard = backoff.attempt();
      match async_nats::connect(url).await {
          Ok(c) => {
              guard.reset();      // resolve BEFORE the labeled break (footgun)
              break 'connect c;
          }
          Err(e) => {
              let delay = guard.sample_delay();
              guard.escalate();         // resolve before sleep
              tracing::warn!(url, attempt, max_attempts = MAX_ATTEMPTS, delay_ms = delay.as_millis(),
                  error = %e, "NATS connection attempt failed; retrying");
              if attempt < MAX_ATTEMPTS { tokio::time::sleep(delay).await; }
              last_err = Some(e);
          }
      }
  }
  ```

- [ ] Add a `// escalate chosen: bounded MAX_ATTEMPTS=10 retry; reset vs escalate has no behavioral difference here` comment.
- [ ] Verify `cargo check -p uptrakit-nats`.
- [ ] Add classification test asserting `escalate()` is the verb called on the error branch.

### Task 4.6 — Migrate `npm/releases.rs:18`

**Audit decision** (pre-decided; document inline at migration site):

- Two retry-eligible branches: request-failure at line 24 (TCP/TLS/DNS error from reqwest, pre-HTTP-response) and 5xx at line 49 (registry responded
  with server error).
- Both use `escalate()`. The 5xx case may seem partial-progress flavored (the registry was reachable enough to send headers), but registries serve the
  same package off the same backend; a 5xx burst means the backend is overloaded or rate-limiting. Resetting backoff on every 5xx defeats the
  rate-limiting signal and hammers a recovering registry. Escalate uniformly.

Tasks:

- [ ] In `crates/plugins/package-managers/npm/src/releases.rs`:
  - Line 24-38 (request-failure branch): wrap in guard pattern.
    `let guard = backoff.attempt(); let delay = guard.sample_delay(); guard.escalate(); if attempt < FETCH_MAX_RETRIES { sleep(delay).await; }`. Add
    `// escalate chosen: pre-HTTP transport error, fast-fail` comment.
  - Line 49-62 (5xx branch): same shape. Add `// escalate chosen: 5xx burst signals registry overload/rate-limit; reset would defeat the backoff hint`
    comment.
  - Success path (`return Ok(...)` on parsed response and `return Ok(vec![])` on 404): no guard touched. The function exits without resolving any
    backoff state — fresh `Backoff` per call so no held state escapes.
- [ ] Verify `cargo check -p uptrakit-plugin-package-manager-npm`.
- [ ] Add classification test asserting both branches call `escalate()`.

### Task 4.7 — Migrate `version_check.rs:535`

**Audit decision** (pre-decided; document inline at migration site):

- Generic `run_with_retry` helper over `PluginResult<T>`. `PluginError::is_retryable()` is the only signal. The trait does not expose a "made
  progress" distinction, and a generic retry helper should not make assumptions about the per-plugin semantic. Splitting `is_retryable()` into a
  richer enum is out-of-scope for this plan.
- Use `escalate()` for every retryable attempt.

Tasks:

- [ ] In `crates/shared/agent-core/src/version_check.rs:543–555` (the `if retryable` branch):
  - Replace `let delay = backoff.next_delay();` with the guard pattern:
    `let guard = backoff.attempt(); let delay = guard.sample_delay(); guard.escalate();`.
  - Sleep `delay` then `continue;`.
  - Success path (`Ok(v) => return Ok(v)`) unchanged — no guard.
  - Non-retryable Err path (`return Err(format!(...))`) unchanged — no guard.
- [ ] Add `// escalate chosen: PluginError trait surface carries no partial-progress signal` comment.
- [ ] Verify `cargo check -p uptrakit-agent-core`.
- [ ] Add classification test asserting `escalate()` is called on a retryable `PluginError`.

### Task 4.8 — Custom lint: forbid `?` between `attempt()` and resolution

**Rationale**: The compile-time net (`unused_must_use`, `let_underscore_must_use = "deny"`) catches `backoff.attempt();` and `let _g = ...` but
**not** the realistic refactor pattern `let guard = backoff.attempt(); let delay = guard.sample_delay(); some_fallible_call()?; guard.escalate();` —
the `?` between `attempt()` and resolution compiles cleanly because the guard is "used" later. In release the unresolved drop produces only a `warn!`
log and (per spec) **no state mutation**, so a CI grep nobody alerts on hides it. Spec stretch goal — promote to a required Phase 4 deliverable per
user request.

**Approach chosen**: syn-based AST walker, packaged as a regular `#[test]` in
`crates/core/functional-tests/tests/backoff_guard_no_question_in_attempt_scope.rs`. Same place as the existing `release_config_invariants.rs`
workspace-policy tests; runs in normal `cargo test`; no extra toolchain. A real custom clippy lint or `dylint` plugin would be more rigorous but
demands clippy nightly / dylint runtime — heavyweight for one rule on a single API.

Tasks:

- [ ] **Workspace dep prerequisites** (Binding Rule: workspace dependencies first):
  - Root `Cargo.toml`: extend the existing `syn = { version = "2", features = ["full", "extra-traits"] }` workspace entry to add `"visit"`:
    `syn = { version = "2", features = ["full", "extra-traits", "visit"] }`. The `visit` feature is required for `syn::visit::Visit` and is not in the
    existing pin. Verify `2.x` is still latest stable at plan-execute time (`cargo search syn`).
  - `crates/core/functional-tests/Cargo.toml` `[dev-dependencies]`: add `syn = { workspace = true }` and `cargo_metadata = { workspace = true }` —
    neither is currently a dev-dep of functional-tests; both are workspace-pinned. The same `cargo_metadata` pattern is used by
    `crates/shared/service-sdk/tests/no_workspace_db_deps.rs`.
- [ ] Create `crates/core/functional-tests/tests/backoff_guard_no_question_in_attempt_scope.rs`. Test body:
  - Use `cargo_metadata::MetadataCommand` to enumerate workspace member crate roots (same pattern as
    `crates/shared/service-sdk/tests/no_workspace_db_deps.rs`).
  - For each `*.rs` file under each member's `src/` and `tests/`, parse with `syn::parse_file`.
  - Implement `syn::visit::Visit` with **full recursive delegation** (call `syn::visit::visit_expr(self, e)` / `visit_block` / etc. from each
    overridden `visit_*` so the walker descends into nested blocks, match arms, closures, and inner `Local`s). Do NOT iterate statements manually —
    that misses nested scopes like `{ let g = b.attempt(); { some_fallible()?; g.escalate(); } }`.
  - State machine: when `visit_local` sees an init whose RHS is `<expr>.attempt()` (matched via `ExprMethodCall` with `method.ident == "attempt"`),
    push a `(binding_ident, span)` frame onto an internal stack. While the frame is open, any `ExprTry` encountered during continued traversal is a
    violation against that frame. Frame is closed when:
    - the visitor encounters a method call `<binding_ident>.{reset,escalate}(...)` — clean resolution;
    - OR a `drop(<binding_ident>)` call — treated as resolution for the lint (runtime still emits the warn; the lint does not double-flag);
    - OR traversal exits the lexical scope that contained the `Local` (track via block depth).
  - **`tokio::select!` blocks**: the macro body is opaque to `syn` (parses as `ExprMacro`; inner body not inspectable without macro expansion). The
    visitor cannot enforce the `?` rule inside `select!`. However the _presence_ of `select!` IS detectable. Add a complementary rule: an `ExprMacro`
    whose path ends in `select` (matches `select!`, `tokio::select!`, `futures::select!`) inside an open guard scope is a violation UNLESS the guard
    is resolved on the line immediately preceding the macro (strict: the resolution must be the last statement before the macro, same-line is fine).
    This catches the realistic footgun "future contributor holds a guard across a `select!` body containing `recv().await?`" without requiring macro
    expansion. Document this complementary rule in the test's module-level doc comment. Escape hatch is the same
    `// uptrakit-backoff: allow ? in attempt scope — <reason>` suppression comment.
  - Collect violations as `(file_path, line, message)` tuples; on non-empty, `panic!` with the formatted list.
  - Allowlist mechanism: a `// uptrakit-backoff: allow ? in attempt scope — <reason>` comment on the violating line OR the immediately preceding line
    suppresses the check. Implementation: pre-parse each file's lines into a `HashSet<usize>` of suppressed line numbers; skip violations whose line
    is in the set.
- [ ] Verify the test passes on the migrated workspace after Tasks 4.1–4.7.
- [ ] Verify the test FAILS when a deliberate violation is introduced (manual one-off check):
  - Temporarily add `let guard = backoff.attempt(); let _ = some_fallible()?; guard.escalate();` to one migrated site.
  - Run `cargo test -p uptrakit-functional-tests backoff_guard_no_question_in_attempt_scope` — must fail with the violation line cited.
  - Revert the deliberate violation; re-run; must pass.
- [ ] Verify the allowlist: add a synthetic violation with the suppression comment; confirm the test passes; then remove the synthetic case.
- [ ] Document the lint in `docs/development/coding-standards.md` §"Service Reconnect Backoff" (Task 5.1 below): explain the rule, the escape hatch,
      and the `tokio::select!` opacity gap.

## Phase 5 — Documentation

### Task 5.1 — Rewrite `coding-standards.md` §Service Reconnect Backoff

- [ ] Open `docs/development/coding-standards.md`, locate the §"Service Reconnect Backoff" section starting at line 959 (ends around line 999).
- [ ] Replace the `next_delay()` / `reset()` example with the guard-pattern equivalent. Show:
  - Reconnect-with-reset-on-success shape:

    ```rust
    let guard = backoff.attempt();
    let delay = guard.sample_delay();
    match work().await {
        Ok(_) => { guard.reset(); break; }
        Err(_) => { guard.escalate(); sleep(delay).await; }
    }
    ```

  - Partial-progress shape with the bug-fix framing: distinguish post-WS-upgrade close from pre-upgrade TCP refusal; use `reset()` for the former.
  - One-paragraph note: "Forgetting to resolve a guard is caught by workspace `unused_must_use` + `clippy::let_underscore_must_use = "deny"` for the
    common cases; held-across-`?` is forbidden by the workspace test
    `crates/core/functional-tests/tests/backoff_guard_no_question_in_attempt_scope.rs` (escape hatch:
    `// uptrakit-backoff: allow ? in attempt scope — <reason>` comment); the `Drop` impl's `debug_assert!` is a final test-time loudness backstop."

- [ ] Reference `https://docs.rs/uptrakit-backoff` for the full API.
- [ ] Run `npx prettier --write docs/development/coding-standards.md && markdownlint --config .markdownlint.json docs/development/coding-standards.md`
      — must pass clean.

### Task 5.2 — Verify no other docs reference the removed API

- [ ] `grep -rn 'next_delay\|backoff\.reset' docs/ --include='*.md'` — clean up any other matches that reference the old API by name (the
      `docs/development/service-lifecycle.md:641` mention of `Backoff` is generic and likely doesn't need change; confirm).

## Phase 6 — Quality gates

### Task 6.1 — Run the full gate suite

- [ ] `cargo fmt --all`.
- [ ] Per-crate tests (touched by the migration):
  - `cargo test -p uptrakit-backoff`.
  - `cargo test -p uptrakit-service-sdk`.
  - `cargo test -p uptrakit-web-api`.
  - `cargo test -p uptrakit-nats`.
  - `cargo test -p uptrakit-agent-core`.
  - `cargo test -p uptrakit-mqtt-runtime`.
  - `cargo test -p uptrakit-plugin-package-manager-npm`.
  - `cargo test -p uptrakit-functional-tests release_config`.
  - `cargo test -p uptrakit-functional-tests backoff_guard_no_question_in_attempt_scope` (the new Task 4.8 lint test).
- [ ] Compile-feature matrices (per snapshot Tooling Constraints):
  - `cargo check --no-default-features --features db-sqlite`.
  - `cargo check --all-features`.
  - `cargo clippy --all-targets --no-default-features --features db-sqlite`.
  - `cargo clippy --all-targets --all-features`.
- [ ] Workspace-wide tests: `cargo test --all-features`.
- [ ] `cargo deny check`.
- [ ] Publishability dry-runs: `cargo package -p uptrakit-backoff --allow-dirty` and `cargo package -p uptrakit-service-sdk --allow-dirty` — both must
      succeed.
- [ ] `markdownlint --config .markdownlint.json '**/*.md'`.

### Task 6.2 — End-to-end smoke test

- [ ] Start a controller and a service (`agent` or `mqtt`).
- [ ] Wait until service logs `WebSocket connected` and `waiting for approval...`.
- [ ] Force "superseded by new connection": start a second instance of the same `service_id`, or close the WS server-side.
- [ ] Confirm next reconnect delay near 2s (base), not ~60s. No `backoff guard dropped unresolved` warn in logs.
- [ ] Regression A: kill the controller entirely → repeated connection-refused failures → delay must double up to 60s (`escalate()` branch).
- [ ] Regression B: do step 3 (supersede), then step 5 (kill controller). Verify both branches behave correctly with no spurious warns.

### Task 6.3 — Commit conventions for release-plz

- [ ] Squash or organize commits so the version-bumping change carries a Conventional-Commit subject like
      `feat(backoff)!: rewrite API with consuming guard pattern` and a `BREAKING CHANGE:` footer in the body. This tells release-plz to bump minor on
      the next automated release (matching the manual `0.0.1 → 0.1.0` jump).
- [ ] `git push` after all gates green.

## Self-review (run before declaring done)

- [ ] Every `Backoff::next_delay` and `Backoff::reset` call site removed (grep `next_delay\|backoff\.reset` across the workspace — only the deleted
      `service-sdk/src/backoff.rs` and library tests should have come up before this PR).
- [ ] Every migrated call site has at least one classification test asserting the chosen verb for at least one error variant.
- [ ] Every audit decision is documented inline as a `//` comment at the migration site (`// <verb> chosen: <reason>`). All six non-enrollment audit
      decisions match the pre-decided verbs in this plan (mqtt: `escalate` uniformly; nats_transport: `escalate` uniformly; nats/connection:
      `escalate` uniformly; npm: `escalate` both branches; version_check: `escalate`; reconnect loop at lifecycle.rs:481: ReceiveClosed → `reset`,
      TransientNetwork → `escalate`). Only the enrollment loop (lifecycle.rs:331) and the reconnect loop's ReceiveClosed arm use `reset` — those are
      the sites with an explicit close-code-aware signal that the cycle was healthy.
- [ ] No `had_connack` / `had_successful_fetch` style session-state flags in mqtt or nats_transport — earlier drafts proposed them; final decision is
      `escalate()` to preserve broker/server-friendly escalation hints during outages.
- [ ] Task 4.8 lint test (`backoff_guard_no_question_in_attempt_scope.rs`) passes; verified by both green run on migrated tree AND a temporary
      deliberate-violation run that fails as expected.
- [ ] Root `Cargo.toml` workspace `syn` entry has `"visit"` feature (verify line in `[workspace.dependencies]`).
- [ ] `crates/core/functional-tests/Cargo.toml` `[dev-dependencies]` includes both `syn = { workspace = true }` and
      `cargo_metadata = { workspace = true }`.
- [ ] `Backoff::base()` accessor was NOT added (per YAGNI; jitter preserved via two-guard pattern at `LoopOutcome::Disconnected`).
- [ ] `debug_assert!` is NOT present in the production `Drop` impl (snapshot Binding Rule: no `panic!()` in production code; loudness lives at
      compile-time lints, Task 4.8 lint test, and unit tests).
- [ ] `release-plz.toml` no longer lists `uptrakit-backoff` under `release = false`; new entry in the publishable section is present;
      `uptrakit-service-sdk`'s `changelog_include` includes `uptrakit-backoff`.
- [ ] `crates/shared/backoff/README.md` exists; `cargo package -p uptrakit-backoff` includes it.
- [ ] `docs/development/coding-standards.md` §"Service Reconnect Backoff" rewritten with guard examples; no lingering `next_delay`/`reset` text.
- [ ] `#![warn(missing_docs)]` at `crates/shared/backoff/src/lib.rs` crate root; `cargo doc -p uptrakit-backoff` produces no warnings.
- [ ] All quality gates from Task 6.1 green.
- [ ] Snapshot Binding Rules check:
  - `#[non_exhaustive]` on `Backoff` struct ✓
  - No `unwrap()` / `expect()` / `panic!()` in production paths (`debug_assert!` is dev-only) ✓
  - Workspace deps used (`rand`, `tracing` via workspace) ✓
  - `tracing` (not `log`) for the warn record ✓
  - Quality gates run ✓
