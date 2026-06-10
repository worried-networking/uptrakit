# Backoff Guard API — Reset-on-Partial-Success by Construction

**Date:** 2026-06-10 **Status:** Proposed

## Problem

The service-SDK enrollment loop in `crates/shared/service-sdk/src/lifecycle.rs:331` keeps escalating its reconnect backoff to the 60-second cap even
after a genuinely successful connection. Reproducer from production logs:

```text
11:31:33  reconnect in 73.341s
11:32:46  reconnecting with enrollment secret
11:32:46  WebSocket connected
11:32:46  waiting for approval...
11:33:20  connection closed by controller: superseded by new connection (1008)
11:33:20  reconnect in 72.834s   ← still capped
```

The service connected to the controller, completed the TLS + WebSocket upgrade, sat in `waiting for approval...` for 34 s, and was then closed by the
controller. The next reconnect should start fresh (base = 2 s), not inherit the 72-second cap from an earlier failure streak.

Root cause is API, not policy. The current `Backoff::next_delay()` / `Backoff::reset()` pair offers no syntactic forcing function. The enrollment
loop's success path is `break;` on `Ok(())` — `reset()` is never reached because the partial-success failure (received-close after WS upgrade) takes
the same `Err` branch as a pre-upgrade TCP refusal. The decision "was this attempt progress-bearing or not?" exists in the error variants but is
invisible to the backoff.

`uptrakit-backoff` is a 60-line internal crate with 7 call sites across the workspace. It is about to be published to crates.io as part of unblocking
`uptrakit-service-sdk` from carrying an in-tree duplicate (`crates/shared/service-sdk/src/backoff.rs`, identical fork). This spec changes the API
once, before the first published version, so the bug class ("forgot to reset on partial success") becomes a compile error.

## Goals

1. **Fix the user-reported bug** by splitting the collapsed `is_receive_closed_report(&e) || is_transient_network_report(&e)` arm at
   `lifecycle.rs:354` into two arms, one per predicate. This is the actual fix — it lives at the call site, not in the library. The library change
   exists to make the surrounding mistakes harder.
2. **Make the most common forgot-to-resolve mistakes a compile error.** `backoff.attempt();` (unbound expression) and `let _g = backoff.attempt();`
   (underscore bind) become hard build errors via `unused_must_use` + `clippy::let_underscore_must_use = "deny"`. What the guard API does **not**
   prevent at compile time is `let guard = backoff.attempt(); ...?; guard.escalate();` — a `?` between `attempt()` and the resolution verb compiles
   cleanly (the guard is "used") and at runtime produces only a `warn!` log on the unresolved drop (no state mutation). The implementation plan closes
   this gap via a workspace syn-AST test that forbids `?` operators inside the lexical scope of a live `AttemptGuard`.
3. **Misclassification remains a call-site responsibility.** The guard API does not prevent calling the wrong verb for a given error (e.g.
   `escalate()` when `reset()` was correct) — that's the same kind of mistake the current `next_delay`/`reset` API allows. Per-call-site
   classification tests (see §Tests) are the regression net for misclassification.
4. The 7 call sites migrate in a single PR. No deprecation period; pre-1.0 crate (0.0.1 → 0.1.0); zero crates.io users today.
5. `uptrakit-backoff` is published from this change forward; the duplicate copy in service-sdk is deleted.
6. The new API is small enough that future maintainers do not need a tutorial to use it correctly.

## Non-goals (YAGNI)

- **No `cancel()` verb on the guard.** Zero of the 7 call sites need an explicit deliberate-abandonment path today. Where the existing code shape
  could otherwise drop a guard unresolved (signal arms inside `tokio::select!`), the migration recipe is to construct the guard only after deciding to
  use it, so the cancellation arm doesn't see one. Add `cancel()` when the first real caller needs it; addition is non-breaking.
- **No `EnrollmentError::connection_was_established()` classifier method.** The enrollment site already has `is_receive_closed_report` and
  `is_transient_network_report` predicates (`lifecycle.rs:653,664`); they cleanly distinguish the post-upgrade close from a pre-upgrade fast-fail.
  Splitting the existing combined `if … || …` branch into two arms — one per predicate — costs five lines, zero new API surface.
- **No configurable threshold, no stability window, no time-elapsed heuristic.** Earlier design rounds explored a time-based auto-reset; it was
  rejected because (a) it requires every consumer to honor an undocumented "caller must sleep the returned delay" contract; (b) it silently mis-fires
  during scheduler delay; (c) it adds knobs the library author has to support forever. The guard model puts the decision at the call site explicitly.
- **No `Outcome` enum on `resolve(outcome)`.** Separate verbs on the guard read better at the call site than `guard.resolve(Outcome::Success)`. User
  preference, confirmed in design loop.
- **No close-code-aware service exit on 1008 supersede.** That's a wire-layer decision (should two services with the same `service_id` fight or should
  one exit?). Out of scope for a backoff API spec.

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

    /// Begin a tracked attempt cycle. The returned guard MUST be resolved
    /// via `reset` or `escalate` before drop. Drop of an unresolved guard
    /// emits a `warn!` log; state is not mutated.
    pub fn attempt(&mut self) -> AttemptGuard<'_>;

    /// Sample a `base + jitter` delay **independent of `current`** —
    /// returns a value in `[base, base + base/4]` regardless of how
    /// escalated the backoff state is. Use when a caller has just resolved
    /// a guard via `reset()` (so `current == base`) and needs the post-reset
    /// delay without spinning a fake `attempt()`. Re-samples jitter per call.
    pub fn sample_base_jitter(&self) -> Duration;
}

#[must_use = "AttemptGuard must be resolved via .reset() or .escalate()"]
pub struct AttemptGuard<'a> {
    backoff: &'a mut Backoff,
    resolved: bool,
}

impl<'a> AttemptGuard<'a> {
    /// Sample a delay for THIS attempt cycle (`current + jitter`).
    /// Does not advance backoff state. Jitter is re-sampled on every call,
    /// so two consecutive calls return different values — the `sample_`
    /// prefix is deliberate. Store the result before resolving the guard:
    /// `let delay = guard.sample_delay(); guard.escalate(); sleep(delay).await;`.
    pub fn sample_delay(&self) -> Duration;

    /// The backoff cycle was healthy: set `current` to `base`. Call when the
    /// work returned Ok OR when the work returned Err but the cycle reached
    /// a meaningful application-level milestone (e.g. WS upgrade completed
    /// before a server-initiated close).
    pub fn reset(self);

    /// The backoff cycle was unhealthy: set `current` to `min(current * 2, max)`.
    /// Call when the attempt failed without reaching a meaningful milestone
    /// (e.g. TCP refused, DNS error, pre-upgrade transient).
    pub fn escalate(self);
}

impl Drop for AttemptGuard<'_> {
    fn drop(&mut self) {
        if !self.resolved && !std::thread::panicking() {
            tracing::warn!("backoff guard dropped unresolved (state unchanged); resolve before any ? or early-return between attempt() and resolution");
        }
    }
}
```

### Two verbs, not three

Initial design had three verbs: a success verb, a partial-success verb (Err but cycle healthy), and a failure verb. The first two had identical state
effect (both set `current` to `base`) — the distinction was call-site documentation only. Dropped to two verbs named neutrally for the state action:
`reset` and `escalate`. The verb names describe what the backoff does, not what the work returned. The caller picks `reset` whenever the cycle was
healthy — Ok-on-success OR Err-after-meaningful-progress — and documents the picking decision with an inline comment.

This naming sidesteps the R3 trap (a single English-loaded verb like `succeeded` redefined to cover both Ok and Err-with-progress cases, which
reviewers misread). A reviewer seeing `guard.reset()` on a `ReceiveClosed` arm does not have to overcome an "I thought this returned Err?" objection.

### Enforcement model

The library leans on two layers — compile-time first, runtime as a safety net. Both load-bearing.

1. **Compile-time** — three workspace lints, all promoted to errors via `Cargo.toml [workspace.lints.rust] warnings = "deny"` and
   `[workspace.lints.clippy] clippy::all = "deny"`:
   - `unused_must_use` fires when the value returned by `Backoff::attempt()` is dropped without being used — covers `backoff.attempt();` (unbound
     expression) because the return type `AttemptGuard` carries `#[must_use]`.
   - `unused_variables` fires on `let g = backoff.attempt();` followed by no read of `g`.
   - `clippy::let_underscore_must_use = "deny"` (in workspace lints) closes the underscore escape: `let _g = backoff.attempt();` is **also a compile
     error**, not a runtime-only fallback.

2. **Runtime** — `Drop` triggers when a caller exits a scope without resolving the guard via a path the compile-time lints can't see: `?` propagation
   between `attempt()` and resolution, panic-driven unwind, or any future site where the borrow checker permits a drop the lints didn't catch.
   - **No state mutation** — `current` is unchanged on unresolved drop. Silently escalating on every `?`-driven early-return would recreate the exact
     symptom this spec fixes (inflated delay after a healthy cycle, source unclear to the operator).
   - `tracing::warn!` is the runtime backstop, always emitted (debug and release), so a production incident leaves a breadcrumb in logs.
   - `!std::thread::panicking()` short-circuits during unwind to avoid noisy logging mid-panic.
   - The plan implementing this spec adds a workspace syn-AST test (functional-tests) that statically forbids `?` between `attempt()` and resolution —
     compile-time enforcement of the case the lints miss. No runtime `debug_assert!` is needed because the syn-AST test catches the same pattern
     before the build ships.
   - Test-subscriber note: the unit test that captures the `Drop` `warn!` record must use a non-reentrant `tracing` subscriber (e.g. the
     `tracing-subscriber` `fmt` layer in unbuffered mode, or a channel-based subscriber with `try_send`). A subscriber that holds a `Mutex` across the
     event handler can deadlock if a future code path drops an `AttemptGuard` from inside a `tracing` event.

### Bug-fix at the enrollment site

```rust
// crates/shared/service-sdk/src/lifecycle.rs (sketch)
loop {
    match do_enrollment(...).await {
        Ok(()) => break,
        Err(e) if is_cancelled_report(&e) => return Ok(()),
        Err(e) if is_receive_closed_report(&e) => {
            // Bug fix: post-WS-upgrade close → backoff cycle was healthy.
            let guard = enrollment_backoff.attempt();
            let delay = guard.sample_delay();
            guard.reset();
            tracing::info!(error = %e, "post-upgrade close, reconnecting in {delay:?}");
            tokio::select! {
                () = tokio::time::sleep(delay) => {}
                signal = signals.recv() => return Ok(()),
            }
            identity.load().await?;
            continue;
        }
        Err(e) if is_transient_network_report(&e) => {
            let guard = enrollment_backoff.attempt();
            let delay = guard.sample_delay();
            guard.escalate();
            // sleep / signal arm as above
            continue;
        }
        Err(e) => return Err(e),
    }
}
```

Guard is created only after the loop decides it needs to wait. The cancellation arms (`is_cancelled_report`, the signal arm inside `tokio::select!`)
never see a live guard, so no `Drop` warn fires on clean exit, and no `cancel()` verb is needed.

### Migration of the 7 call sites

Common shape: construct the guard at the point of decision; resolve before sleeping; rely on existing error predicates / outcome match arms to choose
the verb.

| Site                                             | Verb mapping                                                                                                                                                                                      |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `lifecycle.rs:331` enrollment                    | `reset` on `Ok(())`; `reset` on `is_receive_closed_report`; `escalate` on `is_transient_network_report`; uncategorized Err returns without taking the backoff path                                |
| `lifecycle.rs:481` reconnect (transient Err arm) | `reset` on `LoopError::ReceiveClosed`; `escalate` on `LoopError::TransientNetwork`                                                                                                                |
| `lifecycle.rs:481` reconnect (`Ok(outcome)` arm) | `backoff.attempt().reset()` one-liner replaces today's `reset()`                                                                                                                                  |
| `lifecycle.rs:481` `LoopOutcome::Disconnected`   | After the upstream `Ok(outcome)` arm has resolved its guard via `reset()`, sleep `backoff.sample_base_jitter()` to preserve today's `base + jitter` timing without spinning up a fake `attempt()` |
| `mqtt_client.rs:439` ConnAck                     | `backoff.attempt().reset()` one-liner replaces today's `reset()`                                                                                                                                  |
| `mqtt_client.rs:478` poll Err                    | `escalate()` (poll Err is always fast-fail in the rumqttc model)                                                                                                                                  |
| `nats_transport.rs:201` Ok fetch                 | `backoff.attempt().reset()` one-liner                                                                                                                                                             |
| `nats_transport.rs:205` Err fetch                | `escalate()`                                                                                                                                                                                      |
| `nats/connection.rs:54` startup connect          | `reset` on connect Ok (returns from fn); `escalate` per attempt                                                                                                                                   |
| `npm/releases.rs:18` fetch retry                 | Success / 404 paths `return` from inside the loop with no guard (fresh `Backoff` per call); `escalate` per retryable Err branch                                                                   |
| `version_check.rs:535` retry helper              | Success path `return Ok(v)` with no guard (fresh `Backoff` per call); `escalate` per retryable Err                                                                                                |

Notes on table entries that have surprises in the existing code:

- `lifecycle.rs:481` reconnect loop wraps its `match` over `run_event_loop` result with `reconnect_backoff.reset()` on the `Ok(outcome)` arm _before_
  matching on `outcome`. With the guard pattern, replace the upstream reset with `backoff.attempt().reset()` immediately on entering the `Ok(outcome)`
  arm; the inner `Disconnected` arm then sleeps `backoff.sample_base_jitter()` (which samples `base + jitter` without changing state) to preserve
  today's `next_delay()`-after-reset timing without spinning a fake `attempt()` cycle.
- `nats/connection.rs:54` uses a labeled `break 'connect c` on success from inside a `for attempt in 1..=MAX_ATTEMPTS` loop. The guard for the
  successful attempt must be resolved via `reset()` **before** the labeled break, otherwise the guard's `Drop` fires en route to the break and
  escalates a successful connect.
- `npm/releases.rs:18` has two distinct `next_delay()` call sites in the same loop iteration (request-error branch at line 34, 5xx branch at line 59).
  Construct one guard per retry-eligible branch — do not hoist a single guard above the branches, because the success path (`return` inside the loop)
  must not pay the cost of holding a guard.

No call site needs `reset()` outside the enrollment+reconnect loops in service-SDK. That's where the bug lives; that's where the verb earns its keep.

### Audit of the other 6 backoff consumers

Before this spec lands, audit each non-enrollment consumer for the same class of bug: failure paths that classify as "transient" without
distinguishing pre-vs-post-connection establishment, causing escalation to persist after a healthy cycle.

Read each site with the question: _if the work returned `Err` after running for a meaningful duration, does the current code escalate the backoff?_
Concrete findings to look for:

- **`lifecycle.rs:481` reconnect transient-Err arm**: Today escalates on any `LoopError::TransientNetwork` or `LoopError::ReceiveClosed`.
  `ReceiveClosed` after a long-running event loop is structurally identical to the enrollment bug — fix in the same PR using `reset()` for
  `ReceiveClosed`, `escalate()` for `TransientNetwork`.
- **`mqtt_client.rs:478` poll-Err arm**: After ConnAck the event loop ran successfully; a subsequent poll error (broker disconnected) escalates today.
  Investigate whether the rumqttc event loop exposes a "ConnAck-this-session" signal; if so the poll-Err arm can call `reset()`. Pick the verb in
  implementation; document the decision and rationale inline.
- **`nats_transport.rs:205` Err fetch arm**: Same shape. If a fetch returned bytes earlier in the same consumer session, a later fetch failure has the
  same partial-progress flavor. Investigate; pick verb; document.
- **`nats/connection.rs:54` startup connect**: Bounded one-shot. Audit whether `async_nats::connect` can fail post-TLS-handshake; if yes the same
  partial-progress question applies. Otherwise `escalate()` per attempt.
- **`npm/releases.rs:18` fetch retry**: HTTP request retry. Audit whether a 5xx after TCP+TLS+HTTP-header-exchange should reset (the server responded;
  the cycle was healthy) or escalate (the server's misbehaving; back off). Pick the verb based on observed registry behavior; document.
- **`version_check.rs:535` retry helper**: Generic retry helper over a plugin op. Audit whether the plugin trait surface exposes a "made progress"
  signal; if not, `escalate()` per attempt and document the gap.

Document the audit decisions inline in each call site as a one-line comment when migrating: `// reset unavailable here: <reason>`. The plan that
implements this spec must include the audit findings as a named subtask, not a deferred follow-up.

### Publish + dedup

Two changes, both required (one alone is insufficient — workspace `publish = ["uptrakit-private"]` blocks crates.io by default, and `release-plz.toml`
separately opts the crate out of release-plz analysis):

1. `crates/shared/backoff/Cargo.toml` — add `publish = true` and bump `version = "0.0.1"` → `version = "0.1.0"` (marks the API break). Other metadata
   already inherited from `[workspace.package]`. The version bump is a **manual edit** committed alongside the API change; release-plz's
   Conventional-Commit-driven auto-bump would otherwise pick `0.0.1 → 0.0.2` for the first `fix:` commit. Include `BREAKING CHANGE:` in the
   commit-message footer so release-plz records it as a minor bump in subsequent automated releases. (`cargo` accepts the literal `publish = true`;
   confirmed by `crates/shared/build-info/Cargo.toml:9`, `crates/shared/wire/Cargo.toml:9`, `crates/shared/service-sdk/Cargo.toml:9`.)
2. `release-plz.toml` — move the `uptrakit-backoff` `[[package]]` entry from the `release = false` block (lines 76–78) to the "Public-API library
   crates" section with `git_release_enable = false`, `publish = true`. Add `"uptrakit-backoff"` to `uptrakit-service-sdk`'s `changelog_include` array
   (lines 663–667) so backoff changes surface in the SDK changelog.

Drop the duplicate:

- Delete `crates/shared/service-sdk/src/backoff.rs`.
- `crates/shared/service-sdk/src/lib.rs`: remove `pub mod backoff;`, replace `pub use backoff::Backoff;` with
  `pub use uptrakit_backoff::{Backoff, AttemptGuard};`
- `crates/shared/service-sdk/Cargo.toml`:
  - Add `uptrakit-backoff = { workspace = true }` to `[dependencies]`.
  - Remove `rand = { workspace = true }` — `backoff.rs` is the only `rand` consumer in service-sdk (verified by grep).

### Tests

In `crates/shared/backoff/src/lib.rs`:

- `attempt_reset_sets_current_to_base`
- `attempt_escalate_doubles_with_cap`
- `dropping_unresolved_guard_warns_and_does_not_mutate_state` — capture `tracing` via a non-reentrant test subscriber; verify warn record AND
  `current` unchanged.
- `dropping_unresolved_guard_during_panic_does_not_warn` — wrap in `std::panic::catch_unwind`; verify the `!std::thread::panicking()` guard suppresses
  the warn during unwind.
- `sample_delay_does_not_advance_state` — multiple peeks return values in the expected range; state unchanged.
- `sample_base_jitter_samples_base_plus_jitter_without_state_change` — confirms `Backoff::sample_base_jitter()` returns a value in
  `[base, base + base/4]`, state unchanged after, consecutive calls return different values (jitter re-sampled).
- `bug_regression_reset_at_cap_returns_base` — escalate via repeated `escalate()` until cap, then `reset()`, then `sample_delay()` returns base. Locks
  in the user's reported scenario.

Existing tests (`doubling_behaviour`, `max_cap`, `reset_returns_to_base`, `zero_base_does_not_panic`) are deleted — they exercise the removed
`next_delay`/`reset` API. The behaviors they cover are preserved by the new tests above.

**Per-call-site classification tests (mandatory)**: every call site that migrates to the guard pattern must add at least one unit test per
error-to-verb mapping it performs, asserting that the classification results in the expected backoff state. The library-level tests above only prove
the verbs work; they do not prove the _call site picks the right verb for the right error_. The classification lives at the caller, so the regression
test must live with the caller. Concrete required tests:

- `service-sdk lifecycle.rs:331` — assert `is_receive_closed_report` → `reset`; `is_transient_network_report` → `escalate`. Mock the `do_enrollment`
  outcome variants and observe `backoff.attempt().sample_delay()` before and after.
- `service-sdk lifecycle.rs:481` — same shape for `LoopError::ReceiveClosed` vs `LoopError::TransientNetwork`.
- Each of the 6 audited sites adds the classification test its audit decision warrants (or notes "no partial-progress signal available; only
  `escalate()` is reachable from this site").

## Documentation deliverables

- `crates/shared/backoff/src/lib.rs` — rustdoc on `Backoff`, `AttemptGuard`, every public method. Mandatory because the crate is now published. Add
  `#![warn(missing_docs)]` at the crate root so the workspace `warnings = "deny"` umbrella promotes any future undocumented public item to a hard
  error.
- `crates/shared/backoff/README.md` — **new**. Required by crates.io convention for a published crate. Short: what it is, three example snippets
  (success-on-Ok pattern, partial-progress pattern, bounded retry pattern), link to docs.rs for full API.
- `docs/development/coding-standards.md` §"Service Reconnect Backoff" (lines 959–989) — rewrite. Existing example uses `next_delay()` / `reset()`;
  replace with the guard pattern.
- Per-crate `CHANGELOG.md` entries via release-plz automation; no manual changes needed.

No ADR: this is a library refactor inside an internal crate, not an architectural decision. The decision to publish the crate is an extension of the
in-flight publishable-crate-squat-chain spec (`2026-06-09`); no new architectural ground.

No `CONTEXT.md` update: terminology unchanged.

## Verification

Quality gates (from standards snapshot):

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
cargo package -p uptrakit-service-sdk --allow-dirty
markdownlint --config .markdownlint.json '**/*.md'
```

End-to-end smoke (manual):

1. Start controller + a service (e.g. `agent`).
2. Wait for `WebSocket connected` and `waiting for approval...`.
3. Force "superseded by new connection" (start a second instance with the same `service_id`, or close the WS server-side).
4. Confirm next reconnect delay is near 2 s (base), not ~60 s. No `backoff guard dropped unresolved` warn should appear.
5. Regression: kill the controller entirely → repeated connection-refused failures → delay still doubles up to 60 s (`escalate()` branch).
6. Mixed: step 3 (supersede → guard.reset()) then step 5 (controller down → failed escalates from base). Verify both branches behave correctly with no
   spurious warns.

## Deferred

- Close-code-aware service exit on 1008 supersede (wire-layer concern).
- `AttemptGuard::cancel()` verb (no current site needs it; addition is non-breaking).
- `EnrollmentError::connection_was_established()` classifier method (existing predicates suffice; addition is non-breaking).
- Yanking the existing 0.0.1 squat of `uptrakit-backoff` on crates.io (tracked under `2026-06-09-publishable-crate-squat-chain-break` deferred items).
- **Typed error-classifier API** (`Backoff::on_error(&dyn Classifier)` or similar where the library owns the error→verb mapping). This is the shape
  that would structurally prevent the misclassification bug, because the call site no longer chooses the verb — it implements a trait that the type
  system can verify exhaustively. Rejected for this spec because it couples `uptrakit-backoff` to caller error types (each consumer would need a
  Classifier impl), conflicting with the "thin sync std-only crate" charter. Worth re-evaluating if misclassification bugs recur after this spec ships
  and the audit-time documentation drifts.

## Open questions

None for the user. All API-shape questions resolved during the GAN-style design loop (separate-verbs over enum, hard break over deprecation,
pessimistic Drop over no-op, no `cancel()` per YAGNI).
