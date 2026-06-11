# Backoff Plain Methods Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Drop `AttemptGuard` + `Backoff::attempt()` ceremony + workspace syn-AST scanner. Ship 4 plain methods on `Backoff`: `new`,
`reset(&mut self)`, `escalate(&mut self) -> Duration` (with `#[must_use]` on return), `sample_base_jitter(&self) -> Duration`. Migrate 7 call sites to
the simpler shape. Revert local-only `0.1.0` to `0.0.1` (never released). Update docs. Single atomic commit; release-plz handles publishability.

**Architecture:** `Backoff` stays sync std-only. Public surface: 4 methods, all `&mut self` or `&self`, no builder/guard ceremony. `escalate()`
samples pre-escalation `current + jitter`, advances `current`, returns sampled `Duration` with `#[must_use]` so caller cannot silently drop the delay
(workspace `clippy::let_underscore_must_use = "deny"` closes the `let _` escape too). `reset()` returns `()` (success paths usually break out of loop,
no sleep). `sample_base_jitter()` retained for three call sites that read post-reset delay without state mutation.

**Tech Stack:** Rust 2024 / sync std-only library / tokio at call sites (`tokio::time::sleep`, `tokio::select!`, `tracing`). Test framework:
`cargo test` with `compile_fail` doctest for `#[must_use]` enforcement.

**Spec:** `docs/superpowers/specs/2026-06-11-backoff-plain-methods-design.md` (commit `f5b87ae3f`).

## Phase 1 — Rewrite the library

### Task 1.1 — Replace `Backoff` with plain methods

- [ ] Open `crates/shared/backoff/src/lib.rs`.
- [ ] Keep `#![warn(missing_docs)]` at crate root.
- [ ] Keep `#[non_exhaustive]` on `Backoff` (snapshot Binding Rule: extensible public structs in shared crates).
- [ ] Replace `impl Backoff` with **exactly four** methods:

  ```rust
  impl Backoff {
      pub fn new(base: Duration, max: Duration) -> Self;

      /// Set `current` to `base`. Call when the cycle was healthy.
      pub fn reset(&mut self);

      /// Sample a delay from the pre-escalation `current + jitter`,
      /// then advance `current` to `min(current * 2, max)`. Returns the
      /// sampled delay — caller should `sleep(delay).await` before next attempt.
      #[must_use = "the returned Duration is the delay to sleep before the next attempt"]
      pub fn escalate(&mut self) -> Duration;

      /// Sample `base + jitter` without changing state. Used by callers that
      /// just `reset()`-ed and need the post-reset delay.
      pub fn sample_base_jitter(&self) -> Duration;
  }
  ```

- [ ] Implementation detail: `escalate` captures `let delay = sample(self.current);` then sets `self.current = (self.current * 2).min(self.max);` then
      returns `delay`. **Pre-escalation snapshot, then mutate.** Spec §Public API doc on `escalate` calls this order out explicitly.
- [ ] Delete `AttemptGuard` struct, `Drop` impl, `Backoff::attempt`, and all guard methods.
- [ ] Delete the test-helper module that wraps `tracing_subscriber::fmt::Layer` + `mpsc::Sender` + `parking_lot::Mutex` (the `ChannelMakeWriter` /
      `ChannelWriter` / `make_channel_subscriber` helpers). No longer needed — new tests have no logging assertion.

### Task 1.2 — Replace library tests

- [ ] Delete every existing test (the 7 guard-API tests + the AttemptGuard `compile_fail` doctest). Spec §Tests names what's gone.
- [ ] Add five unit tests in `crates/shared/backoff/src/lib.rs` `#[cfg(test)] mod tests`:
  - `reset_sets_current_to_base` — construct `Backoff::new(2s, 60s)`, call `escalate()` enough times to advance past base, call `reset()`, then assert
    internal state matches base via `sample_base_jitter()` returning a value in `[base, base + base/4]`.
  - `escalate_returns_current_plus_jitter_and_doubles_state` — assert returned `Duration` is in `[current, current + current/4]` at each step; assert
    subsequent `escalate()` returns roughly `2x` the prior delay (within jitter band).
  - `escalate_caps_at_max` — repeated `escalate()` until at cap; assert returned delay stays in `[max, max + max/4]` and never exceeds `max + max/4`.
  - `sample_base_jitter_samples_base_plus_jitter_without_state_change` — call N=20 times; assert all in `[base, base + base/4]`, not all equal (jitter
    re-samples), `current` unchanged before/after.
  - `bug_regression_reset_at_cap_returns_base` — escalate to cap, `reset()`, then `sample_base_jitter()` returns base-range. Locks in the
    user-reported scenario from `lifecycle.rs:331`.
- [ ] Add **one** `compile_fail` doc test on `escalate` rustdoc proving `#[must_use]` fires:

  ````rust
  /// ```compile_fail
  /// #![deny(unused_must_use)]
  /// use std::time::Duration;
  /// let mut b = uptrakit_backoff::Backoff::new(Duration::from_secs(1), Duration::from_secs(60));
  /// b.escalate(); // ERROR: unused `Duration` that must be used
  /// ```
  ````

  The inner `#![deny(unused_must_use)]` is required because workspace `warnings = "deny"` does not propagate to doc tests by default (spec §Tests
  calls this out). Doc test asserts a hard compile error.

- [ ] Add **one positive doctest** alongside the `compile_fail` to prove the API is callable when the return is bound (without this, `compile_fail`
      passes on any error, even an unrelated future signature change). One paragraph above the must_use rustdoc, document the canonical pattern:

  ````rust
  /// ```
  /// use std::time::Duration;
  /// let mut b = uptrakit_backoff::Backoff::new(Duration::from_secs(1), Duration::from_secs(60));
  /// let delay = b.escalate();  // bound; compiles fine
  /// assert!(delay >= Duration::from_secs(1));
  /// ```
  ````

### Task 1.3 — Revert library `Cargo.toml`

- [ ] `crates/shared/backoff/Cargo.toml`:
  - Revert `version = "0.1.0"` → `version = "0.0.1"` (spec §Version: the 0.1.0 bump was prep for the now-discarded guard API; release-plz computes
    next publishable version from commit history and `Conventional Commits`).
  - Drop `[dev-dependencies]` entries for `parking_lot` AND `tracing-subscriber` (both consumed only by deleted guard Drop test fixture).

### Task 1.4 — Update workspace `Cargo.toml` reference

- [ ] Root `Cargo.toml` `[workspace.dependencies]`: change `uptrakit-backoff = { path = "crates/shared/backoff", version = "0.1.0" }` →
      `version = "0.0.1"` to match the crate revert.

### Task 1.5 — Verify backoff crate compiles + tests pass

- [ ] `cargo check -p uptrakit-backoff`.
- [ ] `cargo test -p uptrakit-backoff` — all 5 unit tests + 1 positive doctest + 1 compile_fail doctest pass.
- [ ] `cargo clippy --all-targets -p uptrakit-backoff` — clean.

## Phase 2 — Remove the syn-AST workspace lint test + its deps

### Task 2.1 — Delete the workspace lint test

- [ ] `rm crates/core/functional-tests/tests/backoff_guard_no_question_in_attempt_scope.rs` (the 470-line syn-AST scanner; no longer applicable
      post-guard).

### Task 2.2 — Drop functional-tests dev-deps

- [ ] `crates/core/functional-tests/Cargo.toml` `[dev-dependencies]`: drop the three entries added solely for the scanner:
  - `syn = { workspace = true }`
  - `cargo_metadata = { workspace = true }`
  - `proc-macro2 = { workspace = true }`

### Task 2.3 — Revert workspace `syn` feature + drop `proc-macro2` workspace dep

- [ ] Root `Cargo.toml` `[workspace.dependencies]`:
  - `syn`: revert `features = ["full", "extra-traits", "visit"]` → `features = ["full", "extra-traits"]` (the `visit` feature was added for the
    scanner; no other consumer).
  - **Drop** `proc-macro2 = { version = "1", features = ["span-locations"] }` workspace dep entry entirely. The `span-locations` feature was added
    solely for the scanner; no production consumer references `proc-macro2` directly (the proc-macro crates pull it via their own deps). Workspace dep
    drop is safe.
- [ ] Verify the drop is safe: `grep -rn 'proc-macro2 ' crates/ 2>/dev/null | grep -v target` — only the scanner consumed it. If any production crate
      has `proc-macro2 = { workspace = true }`, leave the workspace entry (without the `span-locations` feature).

### Task 2.4 — Verify workspace builds without functional-tests scanner

- [ ] `cargo check --all-features` — clean.
- [ ] `cargo test -p uptrakit-functional-tests --no-run` — confirms functional-tests crate still compiles minus the deleted scanner.

## Phase 3 — Service-sdk re-export update (semver-breaking)

### Task 3.1 — Drop `AttemptGuard` from service-sdk re-export

- [ ] `crates/shared/service-sdk/src/lib.rs`:
  - Change `pub use uptrakit_backoff::{AttemptGuard, Backoff};` → `pub use uptrakit_backoff::Backoff;`.
- [ ] Verify with `cargo check -p uptrakit-service-sdk` after Phase 4 migrations.

> **Semver note for the commit footer**: removing this re-export is a breaking change to `uptrakit-service-sdk`'s public API. The same commit (Phase
> 6.3) carries `BREAKING CHANGE:` covering both `uptrakit-backoff` AND `uptrakit-service-sdk`. release-plz applies the `!` to both publishable crates
> on the next release run.

## Phase 4 — Migrate the 7 call sites + comment cleanup

> **Migration shape** (applies to every error-path site below): replace
>
> ```rust
> let guard = backoff.attempt();
> let delay = guard.sample_delay();
> guard.escalate();
> sleep(delay).await;
> ```
>
> with
>
> ```rust
> let delay = backoff.escalate();
> sleep(delay).await;
> ```
>
> Replace success-path `backoff.attempt().reset();` → `backoff.reset();`.
>
> Replace partial-progress shape (`is_receive_closed_report` / `LoopError::ReceiveClosed`):
>
> ```rust
> let guard = backoff.attempt();
> let delay = guard.sample_delay();
> guard.reset();
> sleep(delay).await;
> ```
>
> with
>
> ```rust
> backoff.reset();
> let delay = backoff.sample_base_jitter();
> sleep(delay).await;
> ```
>
> Inline `// reset chosen: <reason>` / `// escalate chosen: <reason>` comments stay at every call site (the audit log; spec Goal §4).

### Task 4.1 — `lifecycle.rs:331` enrollment loop

- [ ] In `crates/shared/service-sdk/src/lifecycle.rs` around line 331:
  - `Ok(())` arm: no backoff call, break.
  - `is_cancelled_report(&e)` arm: `return Ok(())`, no backoff call.
  - `is_receive_closed_report(&e)` arm: `enrollment_backoff.reset(); let delay = enrollment_backoff.sample_base_jitter();` + tokio::select!
    sleep/signal as today.
  - `is_transient_network_report(&e)` arm: `let delay = enrollment_backoff.escalate();` + tokio::select! sleep/signal.
  - Catch-all `Err(e)`: `return Err(e)`.
- [ ] **Update the enrollment regression test** (spec §Tests "small structural rework"). The existing test's load-bearing assertion is "current is at
      cap (60 s) BEFORE the reset arm fires" — that's the canary on the misclassification. Restructure preserving that assertion:
  1. Call `enrollment_backoff.escalate()` repeatedly until reaching cap; capture each returned `Duration`. The returned `Duration` reflects
     pre-escalation `current + jitter`, so the final at-cap call returns a value in `[60s, 75s]` — **explicitly assert this range** before any reset
     call. This IS the canary.
  2. Call `enrollment_backoff.reset()`.
  3. Call `enrollment_backoff.sample_base_jitter()` and assert it returns base-range `[2s, 2.5s]` — proves `reset()` worked.

  Both assertions are required. An implementer who only adds the post-reset base-range check skips the at-cap canary and weakens the regression.

- [ ] `cargo check -p uptrakit-service-sdk`.

### Task 4.2 — `lifecycle.rs:481` reconnect loop

- [ ] In `crates/shared/service-sdk/src/lifecycle.rs` around line 481:
  - `Ok(outcome)` arm: replace `reconnect_backoff.attempt().reset();` with `reconnect_backoff.reset();` on entry.
  - `Err(e)` arm — split:
    - `LoopError::ReceiveClosed`: `reconnect_backoff.reset(); let delay = reconnect_backoff.sample_base_jitter();` + tokio::select!.
    - `LoopError::TransientNetwork(_)`: `let delay = reconnect_backoff.escalate();` + tokio::select!.
  - `LoopOutcome::Disconnected` arm: **no code change here.** The existing code already calls `reconnect_backoff.sample_base_jitter()` directly with
    no guard pattern; the upstream `Ok(outcome)` arm's `reset()` has already executed before this match reaches `Disconnected`. **Do NOT** add a
    spurious `reconnect_backoff.reset()` before the existing `sample_base_jitter()` call — that would be a double-reset and not necessary.
- [ ] **Inline comment cleanup** in `lifecycle.rs` (spec §Documentation deliverables): delete any narration referencing "the guard", "spinning a fake
      attempt() cycle", or `AttemptGuard`. Particularly around lines 685–687 and any in the enrollment loop block.
- [ ] `cargo check -p uptrakit-service-sdk`.

### Task 4.3 — `mqtt_client.rs:439, 478`

- [ ] In `crates/core/mqtt-runtime/src/mqtt_client.rs`:
  - Line 439 ConnAck arm: replace `reconnect_backoff.attempt().reset();` with `reconnect_backoff.reset();`.
  - Line 478 poll-Err arm: replace the guard pattern with `let delay = reconnect_backoff.escalate();` + the existing `tokio::select!` for
    sleep/shutdown.
- [ ] Inline `// escalate chosen: rumqttc internal retry exhausted by the time Err surfaces` comment stays.
- [ ] `cargo check -p uptrakit-mqtt-runtime`.

### Task 4.4 — `nats_transport.rs:201, 205`

- [ ] In `crates/ui/web-api/src/nats_transport.rs`:
  - Line 201 Ok branch: replace `backoff.attempt().reset();` with `backoff.reset();`.
  - Line 205 Err branch: replace the guard pattern with `let delay = backoff.escalate();` + existing cancellable sleep.
- [ ] Inline `// escalate chosen` comment stays.
- [ ] `cargo check -p uptrakit-web-api`.

### Task 4.5 — `nats/connection.rs:54` bounded startup loop

- [ ] In `crates/shared/nats/src/connection.rs` around the bounded `for attempt in 1..=MAX_ATTEMPTS` loop:
  - Drop the per-iteration `let guard = backoff.attempt();` line.
  - On `Ok(c)`: no backoff call, then `break 'connect c;` (success path; nothing to reset since loop exits).
  - On `Err(e)`: `let delay = backoff.escalate();` + `if attempt < MAX_ATTEMPTS { tokio::time::sleep(delay).await; }`.
- [ ] Inline `// escalate chosen: bounded MAX_ATTEMPTS=10` comment stays.
- [ ] `cargo check -p uptrakit-nats`.

### Task 4.6 — `npm/releases.rs` two retry branches

- [ ] In `crates/plugins/package-managers/npm/src/releases.rs`:
  - Line 24 (request-failure branch): replace guard with
    `let delay = backoff.escalate(); if attempt < FETCH_MAX_RETRIES { tokio::time::sleep(delay).await; }`.
  - Line 49 (5xx branch): same shape.
  - Success path (`return Ok(...)`): unchanged, no backoff call.
- [ ] Inline `// escalate chosen` comments stay per branch.
- [ ] `cargo check -p uptrakit-plugin-package-manager-npm`.

### Task 4.7 — `version_check.rs:535` retry helper

- [ ] In `crates/shared/agent-core/src/version_check.rs`:
  - Replace `let guard = backoff.attempt(); let delay = guard.sample_delay(); guard.escalate(); tokio::time::sleep(delay).await;` (or equivalent) with
    `let delay = backoff.escalate(); tokio::time::sleep(delay).await;`.
- [ ] Inline `// escalate chosen: PluginError trait carries no partial-progress signal` comment stays.
- [ ] `cargo check -p uptrakit-agent-core`.

### Task 4.8 — Workspace compile + per-crate tests

- [ ] `cargo check --all-features` — workspace clean.
- [ ] `cargo test -p uptrakit-service-sdk` — enrollment regression test (restructured per Task 4.1) passes.
- [ ] `cargo test -p uptrakit-mqtt-runtime`, `-p uptrakit-web-api`, `-p uptrakit-nats`, `-p uptrakit-plugin-package-manager-npm`,
      `-p uptrakit-agent-core` — all green.

## Phase 5 — Documentation

### Task 5.1 — `crates/shared/backoff/src/lib.rs` rustdoc

- [ ] Module-level `//!` doc: one-paragraph crate purpose, one-line "API surface: 4 methods on `Backoff`".
- [ ] Per-item rustdoc on each public method matching spec §Public API. Particularly: `escalate` doc spells out "pre-escalation sample, then advance".
- [ ] Run `cargo doc -p uptrakit-backoff` — `#![warn(missing_docs)]` produces no warnings.

### Task 5.2 — Rewrite `crates/shared/backoff/README.md`

- [ ] Drop the entire guard-pattern narrative + the `// uptrakit-backoff: allow ? in attempt scope` escape-hatch reference.
- [ ] Four examples (snapshot Tooling Constraint: rust code blocks tagged ` ```rust ` for markdownlint MD040):
  - **Reset on success**: reconnect loop calls `backoff.reset()` then breaks; otherwise `let delay = backoff.escalate(); sleep(delay).await;`.
  - **Partial-progress**: split error classification — post-WS-upgrade close uses
    `backoff.reset(); let delay = backoff.sample_base_jitter(); sleep(delay).await;`; pre-upgrade transient uses
    `let delay = backoff.escalate(); sleep(delay).await;`. Cite this as the headline bug-fix shape.
  - **`LoopOutcome::Disconnected`**: `sleep(backoff.sample_base_jitter()).await;` after upstream `reset()`.
  - **Bounded retry**: `for attempt in 1..=N` with `escalate()` on retryable Err, `return Ok(...)` on success.
- [ ] Update the `Cargo.toml` install snippet. Show `uptrakit-backoff = "0.1"` — that's the first version release-plz will publish to crates.io after
      this commit lands (`!` + `BREAKING CHANGE:` on a pre-1.0 crate → minor bump from squat 0.0.1 → 0.1.0). Using `"0.1"` (semver-prefix) lets the
      snippet stay correct for the entire 0.1.x line without needing edits per patch release. Keep docs.rs link, MIT/Apache-2.0 license note.
- [ ] Run `npx prettier --print-width 150 --prose-wrap always --write crates/shared/backoff/README.md` +
      `markdownlint --config .markdownlint.json crates/shared/backoff/README.md` — clean.

### Task 5.3 — Rewrite `docs/development/coding-standards.md` §Service Reconnect Backoff

- [ ] Section currently runs ~120 lines describing the guard pattern + workspace functional test + escape-hatch comment + `tracing::warn!` enforcement
      table. **Replace wholesale**.
- [ ] Show three plain-method shapes (reset-on-success, partial-progress, escalate-on-failure) using the new API.
- [ ] Drop references to `AttemptGuard`, `attempt()`, `sample_delay()`, the syn-AST workspace lint, the `// uptrakit-backoff: allow ?` escape hatch,
      and the `Drop` warn.
- [ ] Document the single concrete safety property: `#[must_use]` on `escalate()`'s `Duration` return — caller must use the delay or explicitly
      discard via `let _ = backoff.escalate();` which workspace `clippy::let_underscore_must_use = "deny"` (Snapshot Tooling Constraint) catches.
- [ ] Keep the "base 2 s, cap 60 s" reconnect-mandate guidance.
- [ ] Reference `https://docs.rs/uptrakit-backoff` for full API.
- [ ] Run `npx prettier --print-width 150 --prose-wrap always --write docs/development/coding-standards.md` +
      `markdownlint --config .markdownlint.json docs/development/coding-standards.md` — clean.

### Task 5.4 — Sweep docs for stale references

- [ ] `grep -rn 'AttemptGuard\|backoff.attempt(\|sample_delay\|backoff_guard_no_question_in_attempt_scope\|uptrakit-backoff: allow ?' docs/`:
  - Any hit in `docs/superpowers/specs/` or `docs/superpowers/plans/` (the prior guard spec + this plan + the just-superseded plan) — leave
    (historical record).
  - Any hit in `docs/development/` or `crates/*/README.md` — fix.

## Phase 6 — Quality gates + commit

### Task 6.1 — Full gate suite

- [ ] `cargo fmt --all`.
- [ ] `cargo test -p uptrakit-backoff`.
- [ ] `cargo test -p uptrakit-service-sdk`.
- [ ] `cargo test -p uptrakit-web-api`.
- [ ] `cargo test -p uptrakit-nats`.
- [ ] `cargo test -p uptrakit-agent-core`.
- [ ] `cargo test -p uptrakit-mqtt-runtime`.
- [ ] `cargo test -p uptrakit-plugin-package-manager-npm`.
- [ ] `cargo test -p uptrakit-functional-tests release_config` — release-plz self-consistency stays green.
- [ ] `cargo check --no-default-features --features db-sqlite`.
- [ ] `cargo check --all-features`.
- [ ] `cargo clippy --all-targets --no-default-features --features db-sqlite`.
- [ ] `cargo clippy --all-targets --all-features`.
- [ ] `cargo test --all-features`.
- [ ] `cargo deny check`.
- [ ] `cargo package -p uptrakit-backoff --allow-dirty` — dry-run; tarball includes `README.md` + Cargo.toml at version 0.0.1.
- [ ] `markdownlint --config .markdownlint.json '**/*.md'`.

### Task 6.2 — End-to-end manual smoke

- [ ] Start a controller + a service (`agent`). Wait for `WebSocket connected` and `waiting for approval...`.
- [ ] Force `1008 superseded` (start a second service with same `service_id` OR close WS server-side).
- [ ] Confirm next reconnect delay is near 2 s base, not ~60 s — proves the bug-fix split at `lifecycle.rs:354` still works under the new API.
- [ ] Regression check: kill the controller entirely; service hits repeated connection-refused. Confirm delay doubles up to 60 s — `escalate()` path
      still escalates.

### Task 6.3 — Single atomic commit

- [ ] Stage changes explicitly via `git commit --only <file list>` (do NOT use `git add -A`; per repo feedback `feedback_commit_only_flag.md`, blanket
      staging risks pulling in unrelated work). File list covers everything in Phases 1–5: backoff lib.rs + Cargo.toml + README.md, root Cargo.toml,
      functional-tests/Cargo.toml, deletion of `crates/core/functional-tests/tests/backoff_guard_no_question_in_attempt_scope.rs`,
      service-sdk/src/lib.rs + lifecycle.rs, mqtt_client.rs, nats_transport.rs, nats/connection.rs, npm/releases.rs, version_check.rs,
      coding-standards.md, plus `Cargo.lock` from the dep changes.
- [ ] Commit with Conventional Commits subject + `BREAKING CHANGE:` footer covering BOTH publishable crates:

  ```text
  refactor(backoff)!: drop AttemptGuard, ship plain methods

  Drop AttemptGuard + attempt() ceremony + workspace syn-AST scanner +
  parking_lot dev-dep + proc-macro2 span-locations workspace feature.
  Ship 4 plain methods on Backoff: new, reset(&mut self),
  escalate(&mut self) -> Duration (must_use on return), sample_base_jitter.
  Migrate 7 call sites to the simpler shape. Revert local-only 0.1.0 to
  0.0.1 (never released to crates.io).

  Removes the `pub use AttemptGuard` re-export from service-sdk — breaking
  change for any downstream importing uptrakit_service_sdk::AttemptGuard.

  BREAKING CHANGE: uptrakit-backoff AttemptGuard removed.
  uptrakit-service-sdk re-export pub use uptrakit_backoff::AttemptGuard
  removed. Use Backoff::reset() / Backoff::escalate() -> Duration /
  Backoff::sample_base_jitter() directly.
  ```

  Append `Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>` footer per repo convention (`git log -5` confirms).

- [ ] Verify commit message via `git log -1 --format=full`. release-plz needs the `BREAKING CHANGE:` footer to apply minor bumps on both pre-1.0
      crates.

## Self-review (run before declaring done)

- [ ] No grep hit for `AttemptGuard`, `Backoff::attempt`, `guard.sample_delay`, `guard.reset`, `guard.escalate`, `guard.cancel`, or
      `backoff.attempt()` across `crates/` (only in `target/`).
- [ ] No grep hit for `backoff_guard_no_question_in_attempt_scope` across `crates/` (only in `docs/superpowers/specs/` and `docs/superpowers/plans/` —
      historical record).
- [ ] No grep hit for `proc-macro2 = { workspace = true }` in `crates/core/functional-tests/Cargo.toml`.
- [ ] `crates/shared/backoff/Cargo.toml` shows `version = "0.0.1"` AND no `parking_lot` AND no `tracing-subscriber` in `[dev-dependencies]`.
- [ ] Root `Cargo.toml` `[workspace.dependencies]`: `syn` lacks `"visit"` feature; `proc-macro2` workspace entry is gone; `uptrakit-backoff` line is
      at version `"0.0.1"`.
- [ ] `crates/shared/service-sdk/src/lib.rs` re-export is `pub use uptrakit_backoff::Backoff;` (no `AttemptGuard`).
- [ ] All 7 migrated sites have `// reset chosen / escalate chosen: <reason>` inline comments.
- [ ] Enrollment regression test in `lifecycle.rs` was restructured (Task 4.1), not just signature-swapped.
- [ ] `compile_fail` doctest on `Backoff::escalate` contains `#![deny(unused_must_use)]` inside the block. Positive doctest binding the return is
      present alongside (proves API is callable; guards against future signature-change masking).
- [ ] Enrollment regression test includes the **at-cap assertion** (escalate return in `[60s, 75s]`) AND the post-reset base-range assertion. Both
      required per Task 4.1; removing the at-cap canary silently weakens the regression.
- [ ] `LoopOutcome::Disconnected` arm in `lifecycle.rs:481` reconnect loop was NOT touched (no spurious `reset()` added before
      `sample_base_jitter()`).
- [ ] README + coding-standards.md drop the escape-hatch comment reference + the guard narrative.
- [ ] Commit message has both `!` on subject AND `BREAKING CHANGE:` footer covering both crates.
- [ ] All Phase 6 gates green.
- [ ] Snapshot Binding Rules check:
  - `#[non_exhaustive]` on `Backoff` ✓
  - No `unwrap()` / `expect()` / `panic!()` in production paths ✓
  - `tracing` (N/A — no logging in lib anymore) ✓
  - Workspace deps via `{ workspace = true }` ✓
  - Conventional Commits + `BREAKING CHANGE:` footer ✓
  - Quality gates run ✓
