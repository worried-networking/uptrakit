# Backoff Plain Methods — Drop AttemptGuard

**Date:** 2026-06-11 **Status:** Proposed **Supersedes (partially):** `docs/superpowers/specs/2026-06-10-backoff-guard-api-design.md` (the guard API;
the bug fix at the enrollment site stays)

## Problem

The `AttemptGuard` API landed in `uptrakit-backoff` 0.1.0 (on main, never released to crates.io) was over-engineered for the bug class it claimed to
prevent.

The actual user-reported bug at `crates/shared/service-sdk/src/lifecycle.rs:331` was a **misclassification**: the collapsed
`is_receive_closed_report || is_transient_network_report` arm treated both cases as escalate-eligible, so a 34-second WS-connected attempt that ended
in `1008 superseded` kept the backoff at the 60 s cap instead of resetting. The fix was splitting that arm into two predicate-specific arms — a
call-site change. The library API rewrite (guard + `#[must_use]` + workspace syn-AST lint + parking_lot dev-dep + `proc-macro2 span-locations`
workspace feature) was orthogonal to the actual fix.

The guard pattern's headline safety property — "compile-time enforcement that an attempt must be resolved" — protects against a mistake mode that
doesn't exist in practice. Every migrated site uses the guard in a 3-line window where the resolution call is on the line immediately following the
delay capture. The guard is short-lived; forgetting to resolve it requires unusual control flow that the workspace syn-AST lint then catches
separately.

What the guard pattern actually delivers:

1. Removed free `next_delay()`. Caller must commit to a verb per attempt.
2. Type-encoded "this is an attempt cycle" semantics.
3. Workspace syn-AST lint for `?`-between-attempt-and-resolve (narrow but real).

What it does NOT deliver:

1. **Misclassification protection.** Caller still picks `reset` vs `escalate` per error variant. The original bug was a misclassification; the guard
   does nothing about it.
2. **Forgot-to-resolve enforcement in practice.** Three-line resolution window makes the compile-time check defensive theater for a mistake mode that
   doesn't occur in our codebase.

Costs that don't pull weight:

- `AttemptGuard` type, `#[must_use]` ceremony, Drop impl with `tracing::warn!` test fixture (parking_lot dev-dep, `MakeWriter` newtype, mpsc channel
  test plumbing).
- `crates/core/functional-tests/tests/backoff_guard_no_question_in_attempt_scope.rs` — 470 lines of syn-AST workspace scanner.
- Workspace `syn` `visit` feature + workspace-wide `proc-macro2 span-locations` feature (added solely for the scanner).
- `attempt()` ceremony at every call site (10+ touched lines per site).

## Goals

1. **Keep the actual bug fix at the call site.** The enrollment-loop two-arm split at `lifecycle.rs:354` stays. That's the load-bearing change.
2. **Simplify the library API to plain methods on `Backoff`.** Drop `AttemptGuard` entirely. Drop the workspace syn-AST scanner. Drop the workspace
   feature additions made solely for the scanner.
3. **`#[must_use]` on `Backoff::escalate()`'s `Duration` return is cheap belt-and-suspenders** — no contributor in the current 7-site codebase has
   ever escalated without sleeping, so this is plain ergonomics + lint hygiene, not load-bearing safety. Keep it because the cost is zero; do not
   inflate its safety claim.
4. **Rely on inline `// reset chosen / escalate chosen` comments PLUS the named enrollment regression test** for verb-classification correctness.
   Comments alone rot; the regression test is the permanent canary on the specific misclassification that hit production. Comments + test, not
   comments alone. Type system cannot fix misclassification.

## Non-goals (YAGNI)

- **No type-system enforcement of "must reset"** — short-lived state across many function calls cannot be statically verified without a guard-style
  ceremony, which this spec is explicitly removing. Verb choice is a call-site responsibility documented inline.
- **No workspace AST lint to detect "called escalate without reset somewhere"** — the rule is hard to express precisely (reset may live in a different
  branch reachable from a different stack frame), and the false-positive risk is high.
- **No stability-window time-based auto-reset** — rejected in prior rounds; same reasons (silent semantics, hidden contract).
- **No deprecation of the guard API** — never released; hard delete in the same commit that ships the plain-methods API.

## Approach

### Public API

```rust
// crates/shared/backoff/src/lib.rs
#![warn(missing_docs)]

#[non_exhaustive]
pub struct Backoff {
    current: Duration,
    base: Duration,
    max: Duration,
}

impl Backoff {
    pub fn new(base: Duration, max: Duration) -> Self;

    /// Set `current` to `base`. Call when the backoff cycle was healthy —
    /// work returned Ok OR work returned Err after reaching a meaningful
    /// application-level milestone (e.g. WS upgrade completed before
    /// server-initiated close).
    pub fn reset(&mut self);

    /// Samples a delay from the **pre-escalation** `current + jitter`,
    /// **then** advances `current` to `min(current * 2, max)`. Returns
    /// the sampled delay — the caller should `sleep(delay).await` before
    /// the next attempt. Call when the backoff cycle was unhealthy —
    /// fast-fail, no meaningful milestone reached.
    #[must_use = "the returned Duration is the delay to sleep before the next attempt"]
    pub fn escalate(&mut self) -> Duration;

    /// Sample a `base + jitter` delay independent of `current`. Useful when
    /// a caller just `reset()`-ed and needs the post-reset delay without
    /// further state mutation. Three call sites use this today
    /// (`lifecycle.rs:331` enrollment `is_receive_closed_report` arm,
    /// `lifecycle.rs:481` reconnect `LoopError::ReceiveClosed` arm,
    /// `lifecycle.rs:481` `LoopOutcome::Disconnected`). Re-samples jitter
    /// per call.
    pub fn sample_base_jitter(&self) -> Duration;
}
```

That's it. Four methods + constructor.

`reset` returns `()` — most success paths break out of the loop without sleeping, so a `Duration` return would just be discarded.

`escalate` returns `Duration` AND is `#[must_use]`. This is ergonomics + lint hygiene, not load-bearing safety — none of the 7 current sites have ever
escalated without sleeping. The `let _ = backoff.escalate();` escape is caught by workspace `clippy::let_underscore_must_use = "deny"`. Cheap
guardrail; not the API's reason for existing.

`sample_base_jitter` stays because three call sites (enrollment `is_receive_closed_report`, reconnect `LoopError::ReceiveClosed`, reconnect
`LoopOutcome::Disconnected`) need to read the post-reset delay without further mutating state.

### What gets deleted

- `AttemptGuard` type and its Drop impl.
- `Backoff::attempt()` method.
- `AttemptGuard::sample_delay`, `reset`, `escalate` methods on the guard.
- `crates/core/functional-tests/tests/backoff_guard_no_question_in_attempt_scope.rs` (entire file).
- `crates/core/functional-tests/Cargo.toml` `[dev-dependencies]` entries for `syn`, `cargo_metadata`, `proc-macro2` (added solely for the scanner).
- Workspace `Cargo.toml`: revert `syn = { version = "2", features = ["full", "extra-traits", "visit"] }` →
  `syn = { version = "2", features = ["full", "extra-traits"] }`. Drop `proc-macro2 = { version = "1", features = ["span-locations"] }` workspace dep
  entirely (no other consumer).
- `crates/shared/backoff/Cargo.toml` `[dev-dependencies]`: drop `parking_lot` (added for the Drop warn test fixture) AND drop `tracing-subscriber`
  (only consumed by the `dropping_unresolved_guard_*` tests being deleted; the new test set has no logging assertion).
- `crates/shared/service-sdk/src/lib.rs`: change the re-export from `pub use uptrakit_backoff::{AttemptGuard, Backoff};` to
  `pub use uptrakit_backoff::Backoff;` (`AttemptGuard` no longer exists). **This is a breaking change to `uptrakit-service-sdk`'s public API.**
  service-sdk IS published to crates.io (per the prior spec); removing the re-export breaks any downstream consumer that imported
  `uptrakit_service_sdk::AttemptGuard`. The same commit that lands the rewrite must carry a `BREAKING CHANGE:` footer scoped to service-sdk so
  release-plz publishes service-sdk's next version as a minor bump (`!` on pre-1.0). Commit subject:
  `refactor(backoff)!: drop AttemptGuard, ship plain methods` carries both crate's breaking change in one footer.

### 7 call sites — simpler shape

For every error-path site:

**Before (guard pattern):**

```rust
let guard = backoff.attempt();
let delay = guard.sample_delay();
guard.escalate();
sleep(delay).await;
```

**After (plain methods):**

```rust
let delay = backoff.escalate();
sleep(delay).await;
```

For success paths that just want to reset and break/continue:

**Before:** `backoff.attempt().reset();` (forces a guard cycle just to read state). **After:** `backoff.reset();` (no ceremony).

For the enrollment partial-progress arm (`is_receive_closed_report` branch — the headline fix):

**Before:**

```rust
let guard = enrollment_backoff.attempt();
let delay = guard.sample_delay();   // capture pre-reset jitter
guard.reset();
sleep(delay).await;
```

**After:**

```rust
enrollment_backoff.reset();
let delay = enrollment_backoff.sample_base_jitter();
sleep(delay).await;
```

`sample_base_jitter()` already returns `base + jitter`, which is `current + jitter` post-reset. Same semantics, two-line shape instead of three with
no guard.

For `LoopOutcome::Disconnected` at `lifecycle.rs:481`:

```rust
reconnect_backoff.reset();
let delay = reconnect_backoff.sample_base_jitter();
sleep(delay).await;
```

Same shape as enrollment partial-progress.

### Per-site verb mapping (unchanged from guard-API spec)

| Site                                                        | Verb                                                      |
| ----------------------------------------------------------- | --------------------------------------------------------- |
| `lifecycle.rs:331` enrollment `Ok(())`                      | no backoff call, break                                    |
| `lifecycle.rs:331` enrollment `is_receive_closed_report`    | `reset()` + `sample_base_jitter()`                        |
| `lifecycle.rs:331` enrollment `is_transient_network_report` | `escalate()`                                              |
| `lifecycle.rs:481` reconnect `Ok(_)`                        | `reset()`                                                 |
| `lifecycle.rs:481` reconnect `LoopError::ReceiveClosed`     | `reset()` + `sample_base_jitter()`                        |
| `lifecycle.rs:481` reconnect `LoopError::TransientNetwork`  | `escalate()`                                              |
| `lifecycle.rs:481` `LoopOutcome::Disconnected`              | (already reset upstream by Ok arm) `sample_base_jitter()` |
| `mqtt_client.rs:439` ConnAck                                | `reset()`                                                 |
| `mqtt_client.rs:478` poll-Err                               | `escalate()`                                              |
| `nats_transport.rs:201` Ok fetch                            | `reset()`                                                 |
| `nats_transport.rs:205` Err fetch                           | `escalate()`                                              |
| `nats/connection.rs:54` startup connect Ok                  | `reset()` before `break 'connect c;`                      |
| `nats/connection.rs:54` startup connect Err                 | `escalate()`                                              |
| `npm/releases.rs:24` request-failure                        | `escalate()`                                              |
| `npm/releases.rs:49` 5xx                                    | `escalate()`                                              |
| `version_check.rs:535` retryable Err                        | `escalate()`                                              |

Inline `// reset chosen: <reason>` / `// escalate chosen: <reason>` comments stay at every call site (the audit log).

### Tests

Library tests in `crates/shared/backoff/src/lib.rs` collapse to the essentials:

- `reset_sets_current_to_base`.
- `escalate_returns_current_plus_jitter_and_doubles_state`.
- `escalate_caps_at_max`.
- `sample_base_jitter_samples_base_plus_jitter_without_state_change` (re-sample test stays).
- `bug_regression_reset_at_cap_returns_base` (escalate to cap → reset → sample → base-range).

Delete:

- All `dropping_unresolved_guard_*` tests.
- `attempt_*` tests.
- `compile_fail` doc test for two simultaneous guards.

Add `#[must_use]` enforcement test: a `compile_fail` doc test that `backoff.escalate();` (unbound) fails to compile. The doc block must include
`#![deny(unused_must_use)]` at the top so the test fails on the warning regardless of toolchain default lint level (workspace `warnings = "deny"`
applies to the crate's own build, not to doc tests by default). Single doc-test block.

**Enrollment regression test stays** in `crates/shared/service-sdk/src/lifecycle.rs`. The shape needs a small structural rework, not a bare signature
swap: the existing test currently calls `guard.sample_delay()` to read the **pre-reset** `current + jitter` (at the 60 s cap), then asserts. Under the
new API, `sample_delay()` is gone — the equivalent assertion uses `Backoff::escalate()` (which returns `current + jitter` pre-escalation) to walk up
to the cap, then `Backoff::reset()`, then `Backoff::sample_base_jitter()` returns base-range. Restructure accordingly.

Per-site classification tests at other 6 sites: NOT added. Inline comments + code review are the audit log per single-maintainer scope.

### Version

Revert the local-only 0.1.0 bump. Files to update:

- `crates/shared/backoff/Cargo.toml`: `version = "0.1.0"` → `version = "0.0.1"`.
- Root `Cargo.toml` `[workspace.dependencies]` line for `uptrakit-backoff` (currently
  `uptrakit-backoff = { path = "crates/shared/backoff", version = "0.1.0" }` → `version = "0.0.1"`).

The 0.1.0 bump was prep for the now-discarded guard API and was never published to crates.io. Commit subject:
`refactor(backoff)!: drop AttemptGuard, ship plain methods` + `BREAKING CHANGE:` footer. Release-plz computes the next publishable version from commit
history since the last published tag and applies Conventional Commits: `!` + `BREAKING CHANGE:` on a pre-1.0 crate bumps minor → so the inaugural
plain-methods release will publish as 0.1.0 (or higher if release-plz finds more breaking commits since 0.0.1). The local revert keeps source-of-truth
consistent with what's actually been published; release-plz rewrites `Cargo.toml` at release time.

### Documentation deliverables

- `crates/shared/backoff/src/lib.rs` — rustdoc rewrite on the four public items. Keep `#![warn(missing_docs)]`.
- `crates/shared/backoff/README.md` — rewrite all three examples. Drop guard pattern AND the `// uptrakit-backoff: allow ? in attempt scope`
  escape-hatch reference. Show: success path (`reset` + break), failure path (`escalate` + sleep), partial-progress path (`reset` +
  `sample_base_jitter` + sleep), bounded retry pattern.
- `docs/development/coding-standards.md` §"Service Reconnect Backoff" — rewrite again (~120 lines). Drop guard pattern + syn-AST lint references + the
  escape-hatch comment table at the bottom of the section. Show plain-method shapes.
- `crates/shared/service-sdk/src/lifecycle.rs` inline comments — update or delete comments that reference "the guard", "spinning a fake attempt()
  cycle", or otherwise narrate the guard pattern. Around the `LoopOutcome::Disconnected` arm at `lifecycle.rs:481` and the enrollment loop at
  `lifecycle.rs:331`.
- `release-plz.toml` — already correctly configured (publishable section + `changelog_include` entry from the prior spec, merged on main). No changes
  required here.
- No new ADR (still a library refactor, not architecture).
- No `CONTEXT.md` update.

## Verification

```bash
cargo fmt --all
cargo test -p uptrakit-backoff
cargo test -p uptrakit-service-sdk
cargo test -p uptrakit-web-api
cargo test -p uptrakit-nats
cargo test -p uptrakit-agent-core
cargo test -p uptrakit-mqtt-runtime
cargo test -p uptrakit-plugin-package-manager-npm
cargo test -p uptrakit-functional-tests release_config

cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
cargo package -p uptrakit-backoff --allow-dirty
markdownlint --config .markdownlint.json '**/*.md'
```

End-to-end smoke (manual):

1. Run a service (`agent`); trigger transient network failure on controller side. Confirm reconnect sleeps escalating delay.
2. Trigger `1008 superseded` (start a second service with same `service_id`). Confirm next reconnect is near 2 s base (the bug fix still works).
3. Confirm no `backoff_guard_no_question_in_attempt_scope` test runs in CI (file deleted).

## Deferred

- `1008 supersede` close-code-aware service exit (wire-layer concern, separate spec).
- Yanking the existing 0.0.1 squat on crates.io for `uptrakit-backoff` (the local 0.1.0 was never released; only 0.0.1 ever hit the registry; squat
  cleanup parallel to `2026-06-09-publishable-crate-squat-chain-break` deferred items).

## Review heuristic going forward

This is the second spec in 48 hours on the same surface, reversing the prior conclusion. The 2026-06-10 spec went through three rounds of standards
review + a contrarian pass + extensive grilling and still shipped a design that the implementer's first hands-on use in code review immediately
recognized as over-engineered. The missing decision rule:

> **Every safety mechanism in a spec must cite either (a) a concrete bug that occurred in this codebase OR (b) a concrete near-miss with evidence. No
> safety mechanism survives review if the only justification is "this would prevent a class of mistakes" without an example of that class.**

The guard pattern was sold as preventing "forgot to resolve" — a class that has zero hits in the 7-site codebase. The `#[must_use]` on `AttemptGuard`
and the syn-AST workspace scanner defended a hypothetical. Applied to this spec: the `#[must_use]` on `Backoff::escalate()` is also defending a
hypothetical (zero contributors have escalated without sleeping), so it stays only as cheap belt-and-suspenders, not as a load-bearing property. Cite
this heuristic in future reviews of safety-mechanism additions.

## Note on prior spec

`2026-06-10-backoff-guard-api-design.md` is superseded by this spec on the library-API shape. The bug-fix split at `lifecycle.rs:354` (the actual
fix), the `service-sdk` dedup, the publish-to-crates.io plumbing, the `coding-standards.md` rewrite — all that work was correct and stays in place.
Only the `AttemptGuard` type, the `attempt()` ceremony at call sites, and the `backoff_guard_no_question_in_attempt_scope` workspace lint go away.
