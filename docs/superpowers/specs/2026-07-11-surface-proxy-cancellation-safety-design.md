# Surface-Proxy In-Flight Cancellation Safety — Design

**Date:** 2026-07-12 **Status:** Draft **Source:** `.superpowers/audit-2026-07-11.md` — HIGH "Surface proxy in-flight
bookkeeping is not cancellation-safe: budgets and idempotency reservations leak permanently when invoke() is dropped".

## Problem

`SurfaceProxy::invoke_inner` (`crates/ui/surface-proxy/src/proxy.rs`) reserves in-flight state up front and releases
it only on explicit code paths _after_ subsequent `.await` points:

- `register_pending` (proxy.rs:522) inserts a `PendingRequest`, increments `in_flight_per_provider` and
  `in_flight_per_tenant`, and reserves an `in_flight_idempotency` entry — all under
  `pending: Arc<parking_lot::Mutex<PendingState>>`.
- Release happens only on the success/error branches: `take_pending`/`remove_pending` (decrement counters + drop the
  idempotency reservation) or `release_idempotency`, each reached **after** an await —
  `service_connections.send().await` (:370), `tokio::time::timeout(rx).await` (:381) on the ProviderProxied path;
  `local_executor.execute().await` (:312) with `release_idempotency` at :315 on the ControllerLocal path.

`invoke()` is awaited directly inside an axum handler (surfaces.rs:310-319). When the HTTP client disconnects
mid-request, hyper **drops the future** at whichever `.await` it is parked on, so the release code never runs:

- **ProviderProxied:** the `PendingRequest`, both budget counters, and the idempotency reservation leak until the
  service disconnects (`fail_in_flight_for_provider`). `MAX_IN_FLIGHT_PER_PROVIDER` (32) leaks permanently rate-limit
  the provider; the per-tenant cap (128) blocks the whole tenant.
- **ControllerLocal:** cancellation during `execute()` leaks the idempotency reservation (`release_idempotency` at
  :315 never runs), and `cleanup_expired()` (:647) sweeps only `idempotency_cache` and `provider_failures` — **never**
  `in_flight_idempotency` or `pending` — so that idempotency key returns `DuplicateRequest` until process restart.

Root cause: in-flight reservations have no drop-time cleanup and no backstop sweep.

## Approach

RAII guard as the primary fix (immediate, drop-safe), plus a deadline backstop sweep for defense in depth. The lock is
`parking_lot::Mutex`, so a `Drop` impl can take it synchronously — no async-in-Drop problem.

### 1. `register_pending` returns an RAII cleanup guard

`register_pending` **changes signature from `-> ()` to `-> PendingGuard`** and the call site binds the guard to a
local that outlives the current lock block (the guard owns `Arc<Mutex<PendingState>>` + `request_id` +
`idempotency_key`, **not** a `MutexGuard` — so it's held across the awaits without holding the lock). Its `Drop` locks
`pending` and, **if the entry is still present** (presence-check via the map, not a self-owned "was it released" bit),
runs the same cleanup `take_pending` does — remove the `PendingRequest`, decrement both counters, drop the idempotency
reservation. The presence-check is the whole safety mechanism: it makes Drop a natural no-op whenever any other actor
already removed the entry.

**Defuse is per normal `invoke_inner` return, and the mechanism differs by transport (review-corrected — the two
transports release in different task contexts):**

- **ControllerLocal (contrarian-corrected — this path never calls `register_pending`, so the `PendingGuard`
  above does NOT exist here):** ControllerLocal reserves via `reserve_idempotency` (:298) directly — no
  `pending` entry, no counters, no `request_id`-keyed guard — and releases via `release_idempotency` (:315),
  which is **also identity-blind** (:572) and has no drop-time cleanup at all (the Problem section's
  ControllerLocal leak). Fix: a second, idempotency-only RAII guard — `reserve_idempotency` returns
  `IdempotencyGuard { pending: Arc<Mutex<…>>, key, owner }` whose Drop conditionally removes
  `in_flight_idempotency[K]` only when the stored owner matches; `guard.defuse()` replaces the explicit :315
  release on the success path. `release_idempotency` itself gains the same owner-tag conditional remove (it is
  the only mutator on this path — leaving it identity-blind would keep the cross-request eviction open on this
  transport). Reserve-side atomicity (the owner tag's origin): `reserve_idempotency`'s presence-check and
  owner-tagged insert happen under ONE `pending`-lock acquisition, no await between — a read-then-insert lock
  gap would let two same-K reserves both insert (last-owner-wins) and reintroduce the eviction at the source.
- **ProviderProxied:** cleanup happens in a **separate task** — `SurfaceProxy::complete()` (driven by the WS
  message-processor task) calls `take_pending()`, which removes the entry and decrements counters, _before_ the
  `rx.await` in `invoke_inner` can resolve `Ok`. So `invoke_inner` never releases on this path. When it wakes on
  `Ok`/timeout/error, it calls `guard.defuse()` — but the entry is already gone, so defuse-or-not, the guard's Drop
  would no-op anyway (presence-check). Defuse is still called on every normal return for clarity, but **correctness
  rests on the presence-check, not the arm bit** — the guard must not keep independent release-bookkeeping that could
  double-decrement against `take_pending`.

Net: on cancellation-drop the guard's Drop is the _only_ cleanup and it runs; on any normal return the guard is
defused (and for ProviderProxied the real release already happened elsewhere). The presence-checked Drop coexists with
the **four** existing release actors — `complete()` (success, other task),
`timeout_pending_request`/`fail_pending_request` (in `invoke_inner`), and `fail_in_flight_for_provider`
(connection-lifecycle code, yet another task).

**Identity-tag the idempotency reservation (contrarian-critical — a pre-existing latent bug the guard would widen).**
The `pending` map is keyed by `request_id` (unique per invoke), but `in_flight_idempotency` is keyed by
`IdempotencyKey` (**not** unique — sharing it is the point of idempotency), and `take_pending` today removes
`in_flight_idempotency[pending.idempotency_key]` **identity-blind**. So the presence-check on `pending[request_id]`
protects the pending map + counters but **not** the idempotency map: once any actor removes `[K]`, a concurrent retry
B can re-reserve `[K]`; if request A's cleanup then runs, it evicts **B's live reservation** — a third duplicate C is
no longer deduplicated and a mutating provider action can execute twice. `HashMap::remove` returning `Some` tells you
_something_ was removed, not that it was _yours_. Fix, applied to **both** the guard's Drop and `take_pending` (they
share the body): add the owning `request_id` (or a monotonic generation) to `IdempotencyInFlight`, and remove
`in_flight_idempotency[K]` **only if its stored owner matches this request** — hand-rolled
(`if map.get(&k).is_some_and(|v| v.owner == request_id) { map.remove(&k); }` — there is no `remove_if` on
`std::collections::HashMap` and no existing conditional-remove precedent in the codebase to copy). This closes the
pre-existing `take_pending` hole and prevents the guard from adding a fourth identity-blind remover. Impl detail: the
shared body reads the owning `request_id` from the removed `PendingRequest` (already in hand after
`pending.remove(request_id)?`) and compares it to the `IdempotencyInFlight[K]` owner before removing — do not
re-derive the owner from `[K]` itself — and the conditional idempotency remove sits **after** the
`pending.remove(request_id)?` early-return, so a no-op `take_pending` (entry already gone) never touches the
idempotency map at all. Counter decrements stay safe because every actor routes through
`take_pending`'s `pending.remove(request_id)?`-then-decrement (verified: `decrement_counter` uses `saturating_sub`);
the guard's Drop **must reuse `take_pending`'s body verbatim**, never an independent decrement. Scope of the
owner-tag fix: `take_pending` + `PendingGuard::drop` (shared body), `release_idempotency` +
`IdempotencyGuard::drop` (ControllerLocal, above) — all four removers become owner-conditional.

**Precedent, scoped honestly (review-tempered):** `PendingSurfaceRequest`
(`crates/shared/service-sdk/src/surface_proxy.rs`) — same crate family, same problem shape (a pending-map entry +
oneshot correlation, `Arc<Mutex<…>>`-protected) — proves the **struct shape and arm-bit mechanism**: a
`pending_registered: bool` field, `cleanup_pending()` gated on it (no-op when false), set `false` on the success
path and after Drop cleanup, invoked from `impl Drop`; it also proves the guard-owns-`Arc<Mutex<…>>`+ids shape.
What it does NOT prove: the **presence-check-as-primary-safety** design — `PendingSurfaceRequest` is its map's
only remover, so a bit suffices there; `PendingGuard` faces four concurrent release actors, which is exactly why
this spec makes the presence-check (not the bit) the correctness mechanism. That half is novel design work and
gets the extra test scrutiny (tests 1/5). `DockerSocketProxy::drop` is a secondary, less-exact reference
(unconditional, no success-already-released branch).

Every `invoke_inner` exit — client-disconnect drop, timeout, provider error, success — now releases the reservation
exactly once (by exactly one of the actors above; the presence-check makes the second-comer a no-op). The existing
`take_pending` cleanup body is reused inside the guard's Drop so there is one cleanup implementation, not two.

Implementer note (review-flagged): the crate has **orphan uncompiled files** under `proxy/` (`bookkeeping.rs`,
`idempotency.rs`, `dispatch.rs`, `prepared.rs`, `validation.rs`) — leftovers from an incomplete decomposition, not
declared via `mod`, dead code. The live target is `proxy.rs` (register_pending ~522, take_pending ~548,
cleanup_expired ~647, decrement_counter ~668 — `saturating_sub`, safe against double-decrement); do not edit the
orphan `proxy/bookkeeping.rs` by mistake — trap sharpened: the orphan tree contains an identically-named
`IdempotencyInFlight` struct, so a grep hit or IDE jump can land in the dead copy; the live one is in `proxy.rs`
(~:179, `request_fingerprint` only — the owner field this spec adds goes THERE).

**Drop-safety constraints:** `Drop::drop` locks the `parking_lot::Mutex` for a synchronous `remove`/decrement only —
no `.await`, no nested lock, no `unwrap`. It cannot panic: the `HashMap` ops are infallible and a missing entry is a
no-op (matching `decrement_counter`'s existing saturating behavior), so cleanup is idempotent by construction — the
safety comes from the operations themselves, not from the `panic = "abort"` profile.

### 2. Deadline backstop in `cleanup_expired`

Store an **absolute per-request `deadline: std::time::Instant = reserved_at + resolved_timeout`** on `PendingRequest`
(`std::time::Instant` — the type `cleanup_expired` already uses for `idempotency_cache`'s
`stored_at`/`provider_failures`' `blocked_until`, not `OffsetDateTime`). **Not a global threshold**
(contrarian-critical): per-request timeout is `resolve_timeout`-derived up to `MAX_TIMEOUT_SECONDS` (300s), and
`cleanup_expired` has no per-request timeout — a fixed ~30s sweep would reap a slow-but-alive 300s-timeout request,
dropping its oneshot sender → phantom `ServiceDisconnected` + a **spurious provider-failure** that can trip the
circuit breaker on a healthy provider. So the sweep reaps only entries past `deadline + generous margin` (margin ≥ a
few cleanup intervals — this is a last-resort GC for a genuinely orphaned entry a live path would already have timed
out, never in contention with the normal timeout path). The sweep removes via the **bare** `take_pending` body
(identity-tagged idempotency removal included) and **records no provider failure** — load-bearing, stated
explicitly: a mass-reap of ≥5 orphans for one provider routed through any failure-recording path
(`fail_in_flight_for_provider`-style) would trip the circuit breaker (`FAILURE_LIMIT`=5/60s) on an
already-struggling provider; `take_pending` itself records nothing — keep it that way. Counters and the
idempotency map stay correct. Reap-a-live-entry semantics (traced during review, safe): removing the entry
drops its oneshot sender; a still-parked `invoke_inner` wakes into the existing `Ok(Err(RecvError))` arm and
its guard-Drop no-ops on the already-removed entry — no double-clean. The sweep must also reap **orphaned
ControllerLocal idempotency reservations** (entries in `in_flight_idempotency` with no matching `pending` entry,
past their deadline) — the `IdempotencyGuard` covers the drop path, the sweep is its backstop, symmetrical with
the pending sweep; give `IdempotencyInFlight` its own `deadline` for this.

Both mechanisms are wanted: the guard makes the common cancellation path leak-free immediately; the sweep bounds any
residual leak to one cleanup interval. (The guard alone would suffice for the reported bug, but a stored reservation
with _no_ time bound is exactly the class the audit flagged — the sweep closes it structurally.)

## Tests

Unit tests on `SurfaceProxy`/`PendingState` (the crate's test module):

1. **Cancellation leak regression (the HIGH):** drive `invoke()` to the point where it is parked on the
   provider-response await (ProviderProxied) / `execute()` await (ControllerLocal), then **drop the future**
   (outer `tokio::time::timeout` shorter than the invoke's own timeout — poll-once-then-drop is unreliable, there
   are earlier awaits). Assert `in_flight_per_provider`, `in_flight_per_tenant`, and `in_flight_idempotency` all
   return to their pre-invoke values, and a subsequent same-key request does **not** get `DuplicateRequest`.
   Harness prerequisite (named, not assumed): the ProviderProxied half needs a fake `ServiceConnectionRegistry`
   whose `send()` returns `true` without a live socket (plus cooperating `is_connected`/`is_yielded`) — verify
   the crate's test module has one or scope it as a deliverable; without it this test silently collapses to
   ControllerLocal-only and the reported HIGH goes unregressed. The ControllerLocal half parks the executor on a
   never-completing future (`Notify`).
2. **Success path unchanged / no double-release:** a normal successful invoke releases exactly once (counters correct,
   no underflow); the guard's disarm prevents a second decrement.
3. **Budget non-leak under repeated cancellation:** cancel N invokes to the same provider; assert the provider is not
   permanently rate-limited (counter returns to 0), i.e. the 32-leak lockout can't happen.
4. **Deadline sweep backstop (mechanism corrected during review — the draft's `start_paused` +
   `tokio::time::advance()` plan was WRONG: tokio's paused clock virtualizes only `tokio::time::Instant`; it
   never moves `std::time::Instant`, which is what `cleanup_expired` and every timestamp in this file read, and
   no existing test in the crate combines the two — the claimed "existing pattern" did not exist):** keep
   `std::time::Instant` (consistent with the file's other sweep timestamps — do NOT mix a `tokio::time::Instant`
   deadline into a sweep whose `now` is std) and test **synchronously**: construct a reservation whose
   `deadline` is already past relative to real now (`Instant::now().checked_sub(margin + delta).expect(…)` —
   test-only expect), call `cleanup_expired` once, assert removed + counters decremented. **And the inverse
   (contrarian):** a reservation with `deadline = now + 300s` is **not** reaped and records no provider failure.
   No `start_paused`, no tokio-time APIs — plain `#[test]`/direct call (satisfies the snapshot time-test rule by
   not touching tokio time at all; this is the documented pattern for `std::time::Instant`-driven expiry).
5. **Cross-request idempotency (contrarian-critical):** request A reserves idem-key K; K is released and a concurrent
   B re-reserves K while A's cleanup then runs — assert A's cleanup does **not** evict B's live reservation (the
   identity-tag check holds), so a subsequent same-key C is still deduplicated.

## Documentation deliverables

- Doc comment on `PendingGuard` (arm/defuse contract, drop-safety invariants: sync-only lock, no await, cleanup
  idempotent) and on `register_pending` (now returns a guard the caller must hold for the request's lifetime and
  defuse on success).
- Doc note on `cleanup_expired` that it now also reaps expired in-flight reservations (the backstop).
- No API/wire/OpenAPI change (internal proxy bookkeeping; observable effect is that a disconnected client no longer
  leaks provider/tenant budget or a stuck idempotency key). No new ADR.

## Out of scope / deferred

- The WS session-cleanup race HIGH (separate committed spec — different crate/mechanism, though same
  "cancellation-safety in the connection layer" theme).
- Reworking the idempotency-cache / provider-failure sweep beyond adding the in-flight reap (those paths aren't
  faulted).
- Making `invoke` itself cancellation-token-aware instead of relying on future-drop (the RAII guard makes future-drop
  safe, which is the minimal correct fix; a cooperative-cancellation redesign is larger and unneeded).
