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

- **ControllerLocal:** cleanup happens _inside_ `invoke_inner` — `release_idempotency` runs right after the
  `execute().await`. Fold that release into `guard.defuse()` (or call defuse right after it): the guard's own `armed`
  bit flips false, Drop becomes a no-op. Explicit, in-function, exactly fits.
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
`in_flight_idempotency[K]` **only if its stored owner matches this request** — `remove_if`-style. This closes the
pre-existing `take_pending` hole and prevents the guard from adding a fourth identity-blind remover. Impl detail: the
shared body reads the owning `request_id` from the removed `PendingRequest` (already in hand after
`pending.remove(request_id)?`) and compares it to the `IdempotencyInFlight[K]` owner before removing — do not
re-derive the owner from `[K]` itself. Counter decrements stay safe because every actor routes through
`take_pending`'s `pending.remove(request_id)?`-then-decrement (verified: `decrement_counter` uses `saturating_sub`);
the guard's Drop **must reuse `take_pending`'s body verbatim**, never an independent decrement.

**This is the codebase's established RAII idiom, not a new pattern** (review-corrected): `PendingSurfaceRequest`
(`crates/shared/service-sdk/src/surface_proxy.rs`) — same crate family, same problem shape (a pending-map entry +
oneshot correlation, `Arc<Mutex<…>>`-protected) — already does exactly this: a `pending_registered: bool` field,
`cleanup_pending()` gated on it (no-op when false), set `false` on both the success path and after Drop cleanup,
invoked from `impl Drop`. Model `PendingGuard` on it directly (down to the field semantics); `DockerSocketProxy::drop`
is a secondary, less-exact reference (unconditional, no success-already-released branch).

Every `invoke_inner` exit — client-disconnect drop, timeout, provider error, success — now releases the reservation
exactly once (by exactly one of the actors above; the presence-check makes the second-comer a no-op). The existing
`take_pending` cleanup body is reused inside the guard's Drop so there is one cleanup implementation, not two.

Implementer note (review-flagged): the crate has **orphan uncompiled files** under `proxy/` (`bookkeeping.rs`,
`idempotency.rs`, `dispatch.rs`, `prepared.rs`, `validation.rs`) — leftovers from an incomplete decomposition, not
declared via `mod`, dead code. The live target is `proxy.rs` (register_pending ~522, take_pending ~548,
cleanup_expired ~647, decrement_counter ~668 — `saturating_sub`, safe against double-decrement); do not edit the
orphan `proxy/bookkeeping.rs` by mistake.

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
out, never in contention with the normal timeout path). The sweep removes via `take_pending`'s body (identity-tagged
idempotency removal included), so counters and the idempotency map stay correct.

Both mechanisms are wanted: the guard makes the common cancellation path leak-free immediately; the sweep bounds any
residual leak to one cleanup interval. (The guard alone would suffice for the reported bug, but a stored reservation
with _no_ time bound is exactly the class the audit flagged — the sweep closes it structurally.)

## Tests

Unit tests on `SurfaceProxy`/`PendingState` (the crate's test module):

1. **Cancellation leak regression (the HIGH):** drive `invoke()` to the point where it is parked on the
   provider-response await (ProviderProxied) / `execute()` await (ControllerLocal), then **drop the future** (e.g.
   `tokio::time::timeout` the invoke, or poll-once-then-drop). Assert `in_flight_per_provider`,
   `in_flight_per_tenant`, and `in_flight_idempotency` all return to their pre-invoke values, and a subsequent
   same-key request does **not** get `DuplicateRequest`.
2. **Success path unchanged / no double-release:** a normal successful invoke releases exactly once (counters correct,
   no underflow); the guard's disarm prevents a second decrement.
3. **Budget non-leak under repeated cancellation:** cancel N invokes to the same provider; assert the provider is not
   permanently rate-limited (counter returns to 0), i.e. the 32-leak lockout can't happen.
4. **Deadline sweep backstop:** insert an in-flight reservation, `tokio::time::advance()` past `deadline + margin`,
   run `cleanup_expired`, assert removed + counters decremented. **And the inverse (contrarian):** a reservation with
   a 300s timeout, advanced only ~35s, is **not** reaped (slow-but-alive must survive) and does not record a provider
   failure. `#[tokio::test(start_paused = true)]` + `tokio::time::advance()` — the crate's existing test pattern
   (paused clock intercepts `std::time::Instant`; no injection shim, no `OffsetDateTime`).
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
