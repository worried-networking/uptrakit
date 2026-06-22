# Spec — Migrate `uptrakit_backoff` → `backon`, remove the in-house crate

- **Date:** 2026-06-22
- **Status:** Draft (spec)
- **Author:** Andrey Yantsen (with Claude)
- **Scope:** Replace every `uptrakit_backoff` call site with the `backon` crate (v1.6.0) and delete `crates/shared/backoff/` entirely.

## Problem / Goal

`uptrakit_backoff` (`crates/shared/backoff/`) is a ~180-line in-house stateful backoff struct
(`Backoff::new(base, max)` with `reset()`, `escalate() -> Duration`, `sample_base_jitter()`).
Goal: standardise on the maintained `backon` crate, eliminate the bespoke retry loops (notably two
`last_err.expect("…")` footguns), and **remove `uptrakit_backoff` completely** so a single backoff
mechanism exists in the workspace.

This is the explicit user directive: full removal, all six call sites migrated.

## Decisions (locked during grilling)

1. **Full removal.** All 6 call sites migrate to `backon`; the crate is deleted. The contrarian's
   "split" recommendation (keep the struct for the 3 reconnect loops) was considered and **rejected**
   in favour of a single mechanism. Trade-off accepted: reconnect loops swap named verbs
   (`reset`/`escalate`/`sample_base_jitter`) for backon's iterator-rebuild model.
2. **Accept backon's jitter.** backon's `with_jitter()` adds `cur * rng.f32()` where `rng.f32() ∈ [0.0, 1.0)`
   → jitter `∈ [0, current)`, effective delay band `[current, 2·current)` (zero jitter is possible but
   astronomically rare; the lower bound is closed). This is **4× wider** than the in-house
   `[0, current/4]` and is **not configurable**. The anti-stampede floor (delay ≥ `current`) is
   preserved. Existing jitter assertions in tests are rewritten to the new band (`delay >= base`).
3. **Dependency pin:** `backon = "1.6.0"` (latest stable on crates.io as of 2026-06-22), license
   `Apache-2.0` (already in `deny.toml` allow-list). Declared with
   `default-features = false, features = ["std", "tokio-sleep"]` to drop the default
   `gloo-timers-sleep` (wasm) and `std-blocking-sleep` (blocking thread sleep, unused in async code)
   sleeper backends; `tokio-sleep` is the only sleeper the `.retry()` combinator needs here.
4. **Preserve attempt counts.** Mechanism swap is behaviour-preserving. **Verified against backon 1.6.0
   source:** `(|| …).retry(builder.with_max_times(M))` performs **`M+1` total attempts** — 1 initial
   call plus `M` backoff-driven retries (the iterator yields `M` delays; the combinator adds the
   initial call on top). Therefore, to preserve a loop's existing total of `T` attempts, set
   `with_max_times(T - 1)`. Mapped per existing loop shape: `for _ in 1..=K` (K total) →
   `with_max_times(K - 1)`; `for _ in 0..=K` (K+1 total) → `with_max_times(K)`. Exact values per site
   below; an attempt-count test guards each (the test is the real backstop — do not trust the arithmetic alone).
5. **`#[must_use]` preserved via helper.** The deleted `escalate()` carried `#[must_use]` (binding rule
   `coding-standards.md#service-reconnect-backoff`, enforced by `clippy::let_underscore_must_use`).
   `Iterator::next()` is not `#[must_use]`, so the compile-time guard is lost. Restored by a
   `#[must_use]` helper `next_backoff_delay` (below) used at every reconnect site.

## backon API mapping

| in-house `Backoff` | backon equivalent |
| --- | --- |
| `Backoff::new(base, max)` | `ExponentialBuilder::default().with_min_delay(base).with_max_delay(max).with_jitter()` |
| infinite reconnect | `….without_max_times()` then `builder.build()` |
| bounded retry | `….with_max_times(M)` + `(\|\| async {…}).retry(builder).when(pred).await` |
| `escalate() -> Duration` | `next_backoff_delay(&mut iter)` (`#[must_use]` helper wrapping `iter.next()`) |
| `reset()` | `iter = builder.build()` (rebuild; `ExponentialBuilder` is `Copy`) |
| `sample_base_jitter()` | rebuild then take first: `iter = builder.build(); next_backoff_delay(&mut iter)` — first yield is `min_delay + jitter` ≡ base+jitter, and the rebuild *replaces* the preceding `reset()` rebuild (do not rebuild twice) |

Imports: `use backon::{ExponentialBuilder, BackoffBuilder, Retryable};`
(`BackoffBuilder` for `.build()`, `Retryable` for `.retry()`). The iterator returned by `build()`
is `#[doc(hidden)]` (`backon::ExponentialBackoff`) — **never name it**; bind with inference
(`let mut iter = builder.build();`) and pass through the generic `next_backoff_delay` helper.

> **Footgun — `max_times` cliff.** backon's default `max_times = Some(3)`. An infinite reconnect
> loop that forgets `.without_max_times()` silently stops backing off after 3 attempts → a daemon
> that quits reconnecting during a sustained outage, with no compile error and no short-test failure.
> Mitigation below (centralised builder + `nth()` guard test).

## Shared reconnect-builder helper

Two infinite reconnect loops share params (`base = 2s`, `max = 60s`): `service-sdk/lifecycle.rs`
and `mqtt-runtime/mqtt_client.rs` (the latter already depends on service-sdk and currently consumes
its `pub use uptrakit_backoff::Backoff` re-export). To centralise `.without_max_times()` so it can
**never** be forgotten at these sites:

- In `crates/shared/service-sdk/src/lib.rs`: **remove** `pub use uptrakit_backoff::Backoff;` and add

  ```rust
  use std::time::Duration;

  /// Builder for the standard service reconnect/enrollment backoff: 2s base, 60s cap,
  /// jittered, never terminates (infinite reconnect).
  pub fn reconnect_backoff_builder() -> backon::ExponentialBuilder {
      backon::ExponentialBuilder::default()
          .with_min_delay(Duration::from_secs(2))
          .with_max_delay(Duration::from_secs(60))
          .with_jitter()
          .without_max_times()
  }

  /// Pull the next delay from an infinite backoff iterator. `#[must_use]` restores the
  /// compile-time guard the deleted `Backoff::escalate()` provided (a dropped delay is now a
  /// `clippy::let_underscore_must_use` error). `.expect()` makes a forgotten `without_max_times()`
  /// fail loudly in tests instead of silently masking the `max_times` cliff.
  ///
  /// SAFETY: an `ExponentialBuilder` with `without_max_times()` and **no** `with_total_delay()`
  /// never yields `None` (delay saturates at `Duration::MAX`, verified in backon 1.6.0 source).
  /// Do NOT pass a builder configured with `with_total_delay()` — that is the one config that makes
  /// the iterator terminate and would turn this `.expect()` into a live panic.
  #[must_use]
  pub fn next_backoff_delay(iter: &mut impl Iterator<Item = Duration>) -> Duration {
      iter.next()
          .expect("backoff iterator built via reconnect_backoff_builder() never terminates")
  }
  ```

- `mqtt-runtime` gains a direct `backon = { workspace = true }` dependency and calls
  `uptrakit_service_sdk::reconnect_backoff_builder()` + `next_backoff_delay`.

The other infinite loop (`nats_transport.rs`, `1s/30s`, different params) constructs its builder
inline **with `.without_max_times()`**. `web-api` does **not** depend on `uptrakit-service-sdk`
(confirmed: its Cargo.toml has no such entry), so rather than add a whole-crate dependency edge for
one function, **replicate the 2-line `#[must_use]` helper locally** in `nats_transport.rs` (the
`#[must_use]` guarantee is preserved either way; only DRY suffers, and 2 lines is the lazier correct
choice than a new inter-crate edge). It gets its own guard test (below). Bounded sites never touch
`without_max_times` and never call `next_backoff_delay` (they use the `.retry()` combinator, which
handles exhaustion).

## Per-site migration plan

### High value — bounded retry-a-closure (the `.retry()` win)

**1. `crates/shared/agent-core/src/version_check.rs` — `run_with_retry`**
Reimplement the body of `run_with_retry(name, max_retries, op)` over backon; **keep the wrapper**
(preserves its operation-name logging and the two unchanged call sites `detect_installed` /
`fetch_latest`, both `MAX_RETRIES = 2`). Current loop is `for attempt in 0..=max_retries` =
`max_retries + 1` total attempts = `1 + max_retries` retries-after-first, so backon
`with_max_times(max_retries)` preserves the count exactly. Builder:
`ExponentialBuilder::default().with_min_delay(5s).with_max_delay(20s).with_jitter().with_max_times(max_retries)`.
Use `.retry(builder).when(|e| is_retryable(e)).notify(|e, d| tracing::warn!(…))`.
**Deletes** the manual loop + `last_err.expect("last_err is set")` at `version_check.rs:567`.

**2. `crates/shared/nats/src/connection.rs` — `connect`**
Current loop is `for attempt in 1..=MAX_ATTEMPTS` (`MAX_ATTEMPTS = 10`) = **10 total attempts**. backon
counts retries-after-first, so use `with_max_times(MAX_ATTEMPTS - 1)` = 9 → 10 total (preserve count).
Replace the loop + `last_err.expect(…)` (`connection.rs:89`) with
`(|| async { … }).retry(ExponentialBuilder::default().with_min_delay(1s).with_max_delay(30s).with_jitter().with_max_times(MAX_ATTEMPTS - 1)).await`.
Any connect error is retryable → no `.when` needed (or `.when(|_| true)`).

**3. `crates/plugins/package-managers/npm/src/releases.rs` — `fetch_releases`**
Current loop is `for attempt in 1..=FETCH_MAX_RETRIES` (`FETCH_MAX_RETRIES = 3`) = **3 total attempts**,
so use `with_max_times(FETCH_MAX_RETRIES - 1)` = 2 → 3 total (preserve count). Mixed terminal/retryable
control flow — port carefully. Inside the retried closure:

- transport error / 5xx → `Err(retryable)`
- 404 → `Ok(Vec::new())` (terminal success — not an error)
- other 4xx → `Err(terminal)`

Drive with `.retry(builder).when(|e| e.is_retryable())` so terminal errors short-circuit.
Builder: `with_min_delay(FETCH_BACKOFF_BASE).with_max_delay(FETCH_BACKOFF_MAX).with_jitter().with_max_times(FETCH_MAX_RETRIES - 1)`.

### Reconnect loops — manual iterator (verb → iterator-rebuild)

**4. `crates/ui/web-api/src/nats_transport.rs` — `run_consumer`** (infinite, `1s/30s`)

```rust
let builder = ExponentialBuilder::default()
    .with_min_delay(Duration::from_secs(1)).with_max_delay(Duration::from_secs(30))
    .with_jitter().without_max_times();
let mut iter = builder.build();                       // type inferred; never name ExponentialBackoff
loop {
    match fetch().await {
        Ok(..)  => { iter = builder.build(); /* reset */ /* … */ }
        Err(..) => {
            let delay = next_backoff_delay(&mut iter); // #[must_use] helper; loud on cliff
            tokio::select! { biased; _ = cancel.cancelled() => break, _ = tokio::time::sleep(delay) => {} }
        }
    }
}
```

Preserve the existing `biased` cancellation select.

**5. `crates/core/mqtt-runtime/src/mqtt_client.rs` — `run_event_loop`** (infinite, `2s/60s`)
**Correction:** `reconnect_backoff` is **not** a struct field — it is a local in `start()` (line 311)
passed as a function argument to `run_event_loop()` (`mut reconnect_backoff: Backoff`, line 388). No
struct changes. Replace that parameter/local: thread the `ExponentialBuilder` (it is `Copy`) and bind
the iterator as a stack-local via inference (`let mut iter = builder.build();`). Source the builder from
`reconnect_backoff_builder()`. On `ConnAck` → `iter = builder.build()` (reset); on poll error →
`let delay = next_backoff_delay(&mut iter);`. Keep the `tokio::select!` cancellation. Add `backon` dep
to this crate's `Cargo.toml`.

**6. `crates/shared/service-sdk/src/lifecycle.rs` — enrollment + reconnect loops** (`2s/60s`, 3 verbs)
Highest-care site. Use `reconnect_backoff_builder()` for both loops; bind iterators via inference.

- `reset()` → `iter = builder.build()`
- `escalate()` → `let delay = next_backoff_delay(&mut iter);`
- `sample_base_jitter()` (the `ReceiveClosed` / `Disconnected` arms, used *after* a reset) →
  `iter = builder.build(); let delay = next_backoff_delay(&mut iter);` — this single rebuild *is* the
  reset (do not also rebuild for the preceding `reset()`); first yield = base+jitter, matching the old
  reset-then-sample.

Keep the `// reset chosen:` / `// escalate chosen:` audit comments — update them to describe the
rebuild/`next_backoff_delay` mechanics while preserving the *why*.

## Tests

- **Delete** the 5 unit tests in `crates/shared/backoff/src/lib.rs` (crate removed).
- **Rewrite** the two existing backoff-verb regression tests in `service-sdk/lifecycle.rs` against
  backon behaviour, preserving intent (these are the only two that exist today — `grep` confirms):
  - `enrollment_receive_closed_maps_to_reset_verb` (the **bug canary**): after several failures
    escalate the delay, then assert a `ReceiveClosed` drops the next delay back into the base band
    (`>= 2s`, `< 4s`) rather than the escalated/capped value. The reset-vs-escalate decision remains the unit under test.
  - `enrollment_transient_network_maps_to_escalate_verb`: assert a transient-network error advances
    (`escalate`), not resets.
- **Add new tests** (do not assume they exist — they don't yet):
  - sample-base-jitter behaviour: after `reset()`-then-`sample_base_jitter()` (rebuild + first yield),
    the delay is in the base band and a subsequent failure escalates from base, not from a consumed cursor.
  - `PartialProgress` resets: assert it rebuilds (delay back to base band).
  - Jitter bound assertions use the new band: `delay >= base` and `delay < 2·base` (closed lower bound —
    zero jitter is possible, so `>=` not `>`).
- **Add cliff-guard tests** (the single biggest migration risk): for every infinite loop's builder,
  assert the iterator does not terminate, e.g. `reconnect_backoff_builder().build().nth(50).is_some()`
  and the same for the inline `nats_transport` builder.
- **Add** attempt-count tests for the bounded sites (guards the `with_max_times` off-by-one): npm =
  3 total attempts, nats `connect` = 10, `run_with_retry` = `max_retries + 1`.
- **Add** npm branching tests: 404 → empty `Ok`, other 4xx → terminal error (no retry), 5xx/transport → retried.
- **`start_paused`:** tests that only inspect the `Duration` values returned by `next_backoff_delay`
  (no `tokio::time` call) need **no** `start_paused`. Tests that drive a loop through a real
  `tokio::time::sleep` use `#[tokio::test(start_paused = true)]`. The `nth(50)` cliff-guard test calls
  no tokio time API → **must not** set `start_paused`.

## Cargo / release wiring

- Root `Cargo.toml` `[workspace.dependencies]`: **remove** `uptrakit-backoff = { path = … }`; **add**
  `backon = { version = "1.6.0", default-features = false, features = ["std", "tokio-sleep"] }`.
- Swap `uptrakit-backoff = { workspace = true }` → `backon = { workspace = true }` in:
  `web-api`, `nats`, `service-sdk`, `agent-core`, `package-managers/npm`. **Add** `backon` to
  `mqtt-runtime` (new direct dep; was indirect via service-sdk re-export).
- **Delete** `crates/shared/backoff/` (directory, `Cargo.toml`, `src/lib.rs`).
- `release-plz.toml`: remove the `[[package]] name = "uptrakit-backoff"` entry (lines ~192–194) and
  the `"uptrakit-backoff",` publish-group line (~668). backon is external → no release-plz entry.
  Note: `uptrakit-backoff` is currently `publish = true` (a `0.0.1` squat exists on crates.io per the
  `2026-06-10`/`2026-06-11` backoff specs). Deleting the crate stops future publishes; **yanking the
  existing `0.0.1`** is optional cleanup — **deferred** (out of scope here).
- `deny.toml`: no change expected — `Apache-2.0` already allowed; `cargo deny check` must pass for
  backon + its transitive `fastrand` (`Apache-2.0 OR MIT`, both allowed). Verify during impl.

## Documentation deliverables

- `docs/development/coding-standards.md` — **rewrite the `service-reconnect-backoff` section**: it
  currently *mandates* `uptrakit_backoff::Backoff` with reset/escalate verb comments and the
  `escalate()` `#[must_use]` rule (both are Binding Rules in the snapshot). Replace with backon usage:
  the `.retry()` idiom for bounded retries; `reconnect_backoff_builder()` + `next_backoff_delay`
  (`#[must_use]` preserved) for reconnect loops; the `without_max_times` requirement for infinite loops.
- `.superpowers/standards-snapshot.md` — **no manual edit**: it is a generated artifact derived from
  `coding-standards.md`; it regenerates on next snapshot build once that doc is updated.
- `docs/development/service-lifecycle.md` — **update** if it documents the reconnect verb behaviour as
  current state; describe the backon iterator-rebuild model instead.
- `CONTEXT.md` — **conditional update** if it enumerates `crates/shared/backoff` in the crate map.
- **No new ADR.** This is a dependency swap, not an architectural-boundary decision; documented here
  and in coding-standards is sufficient. (Justification: no module boundary, wire format, or trust
  surface changes.)
- **Do NOT rewrite** historical artifacts under `docs/superpowers/plans/` and `…/specs/`
  (`2026-06-10-backoff-guard-api`, `2026-06-11-backoff-plain-methods`, publishing specs) — they record
  past state.
- **User memory** (`MEMORY.md` "Backoff" glossary entry / crate list) — update out-of-band; not a repo deliverable.

## Quality gates

Standard workspace set, plus dependency check (new external crate) and the doctest + semantic-boundary
gates the pre-push hook enforces:

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo test --no-default-features --features db-sqlite
cargo test --workspace --all-features --doc --exclude uptrakit-mqtt-runtime  # doctest gate (pre-push)
cargo deny check                       # validates backon + fastrand license/advisories
python3 ci/check_plugin_semantic_boundary.py   # mandatory: import graph changes
markdownlint --config .markdownlint.json '**/*.md'
```

Verify no lingering references (exclude historical docs and sibling worktrees, which carry stale copies):

```bash
grep -rn "uptrakit_backoff\|uptrakit-backoff" . \
  --exclude-dir=.git --exclude-dir=target --exclude-dir=.claude \
  --exclude-dir='docs/superpowers' | grep -v '^Binary'
```

Expected result: empty (the only remaining hits should be the historical `docs/superpowers/{plans,specs}`
artifacts, which are intentionally left as-is).

## Risks

1. **`max_times` silent cliff** (highest). Mitigated: centralised `reconnect_backoff_builder()` owns
   `without_max_times()`; `next_backoff_delay` uses `.expect()` (loud, not silent `unwrap_or`); inline
   `nats_transport` builder reviewed; `nth(50).is_some()` guard tests on every infinite loop.
2. **Attempt-count off-by-one** — `.retry(builder.with_max_times(M))` = `M+1` total attempts (one
   initial call plus M retries). Mitigated by the per-site mapping (Decision 4, `with_max_times(T-1)`
   for T total) and explicit attempt-count tests as the backstop.
3. **`#[must_use]` regression** — `Iterator::next()` is not `#[must_use]`, losing the compile-time
   guard the deleted `escalate()` had. Mitigated by the `#[must_use] next_backoff_delay` helper at
   every reconnect site.
4. **npm terminal/retryable branching** — easy to mis-port (e.g. retrying a 404). Mitigated by the
   three explicit branch tests.
5. **mqtt reconnect rebuild** — on every successful `ConnAck` the iterator must be *rebuilt*
   (`iter = builder.build()`), not merely advanced. (`reconnect_backoff` is a function local, not a
   struct field — see site 5.)
6. **Jitter band widening** (accepted) — `[current, 2·current)`; floor preserved, spread 4× wider.

## Out of scope / deferred

- No change to retry **policy** (attempt counts, base/max delays) beyond what each site already uses —
  this is a mechanism swap, behaviour-preserving except jitter band.
- No new durable-task / scheduler retry work (separate effort discussed previously).
- No `failsafe`/circuit-breaker addition.

## Open questions

None blocking. Minor decision already taken: keep `run_with_retry` in `version_check.rs` as a thin
wrapper over backon (vs inlining at both call sites) to preserve operation-name logging.
