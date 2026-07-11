# WS Session Cleanup Race + Panic — Design

**Date:** 2026-07-12
**Status:** Draft
**Source:** `.superpowers/audit-2026-07-11.md` — HIGH "WS session cleanup races reconnection — plain unregister()
can evict a live replacement; connected_at().expect() can panic".

## Problem

Two connection-registry lifecycle bugs in the service-WebSocket handlers
(`crates/ui/web-api/src/routes/service_ws/handler/`):

1. **Cleanup evicts a live replacement (race).** Session ownership is tracked by comparing `OffsetDateTime`
   timestamps (`authenticated_session_ownership`, session_authenticated.rs:613-628) rather than the
   `connection_id` UUID the registry already assigns. `cleanup_authenticated_session` performs several awaits
   (`processor_handle.await`, workload-claim DB writes, connectivity notifications) and then calls the
   **unconditional** `state.service_connections.unregister(&service_id)` (session_authenticated.rs:589; same in
   session_enrolled.rs:214, embedded.rs:211). Agents reconnect immediately after a drop, so a replacement can
   `register()` inside that await window — and the old session's plain `unregister()` then removes the **new
   live** registration. Subsequent `registry.send()` returns false, `is_connected()` reports false, and pushed
   updates/dispatches to a genuinely connected agent are silently dropped until the next reconnect. The registry
   already provides the race-safe `unregister_current(&service_id, connection_id)` (service-connections/lib.rs:165
   — removes only if the current registration's `connection_id` matches), but no web-api call site uses it; the
   `ServiceConnectionHandle` (which carries `connection_id`, lib.rs:148-149) is discarded after extracting its
   cancellation token.
2. **`connected_at().expect()` can panic (no-unwrap violation).** `register_connection`
   (session_authenticated.rs:187-191) calls
   `.connected_at(&service_id).await.expect("connected service should have a registered timestamp")` — a
   **second** registry lookup after `register()`. An admin `force_disconnect` (from actions/services.rs and
   routes/services/lifecycle.rs:346 on deactivate/reject/merge) landing between `register()` and `connected_at()`
   removes the entry, so the `.expect()` panics the production WS task. The invariant spans two lock
   acquisitions, so it is genuinely racy — a real panic, not a "can't happen".

## Approach

Both fixes use data the registry **already mints inside `register()`** — no new registry state, no new locks.
Correctness rests on two verified invariants (contrarian-confirmed): (i) `register()`'s remove-old →
cancel-token → insert-new sequence (lib.rs:141-145) runs under **one** `inner.write()` guard with no `.await`,
so a superseded session's cleanup can never observe the transient empty slot; and (ii) the eviction check needs
per-`register()` **uniqueness**, not time-ordering — `connection_id` equality is the property, so the check must
stay an `==` (never "optimize" it to a `>` on the v7 timestamp). Both belong in the doc comments so a later
refactor (e.g. sharding the registry lock, or a monotonic-id "optimization") can't silently break them.

### 1. Race-safe cleanup via `connection_id`

- `register()` returns a `ServiceConnectionHandle` carrying `connection_id` (private field, `.connection_id()`
  accessor — verified). Store **bare `connection_id: Uuid`** (Copy, trivial) in the session state — matching the
  file's existing extract-and-discard pattern (session state already keeps only the derived `cancel_token`,
  discarding the handle), not the whole handle. Every cleanup path then calls
  `unregister_current(&service_id, connection_id)`: a stale cleanup whose id no longer matches the current
  registration is a no-op — the live replacement stays registered. `unregister_current` returns `bool`; log at
  debug on `false` with structured fields `tracing::debug!(%service_id, %connection_id, "cleanup skipped —
  connection already replaced")` (matching the file's `%service_id`-first convention) so the race is observable.

  **The three paths are NOT symmetric (review-corrected — do not treat as a uniform check-swap):**
  - **authenticated (session_authenticated.rs):** already has `AuthenticatedSessionOwnership {Current, Replaced,
    Removed}` + `finalize_authenticated_session`; the `Replaced` arm already skips `unregister`. This is an
    **upgrade** of the existing *timestamp-based* guard to `connection_id`, to close the residual TOCTOU: the
    `Current` arm still reaches `cleanup_authenticated_session`'s unconditional `unregister(&service_id)`
    (:589), and a replacement racing in between the ownership read and that call is still evicted. **Preserve
    the three-way decision (contrarian-critical):** `finalize_authenticated_session` today branches
    `{Current, Replaced, Removed}`, and the `Replaced` arm carries real side effects (`processor_cancel.cancel()`,
    `processor_handle.await`, surface-proxy `fail_in_flight_for_provider`, `SurfacesChanged` broadcast). A single
    `unregister_current` bool collapses `Replaced` and `Removed` into one `false` — a naive `if removed { … }
    else { skip }` would **drop the `Replaced` side effects** (in-flight surface requests never failed, no
    `SurfacesChanged`), a silent regression. The rewrite must keep a three-way outcome: use `connection_id`
    equality for the *eviction* decision (replace the timestamp compare), but still distinguish
    Replaced-vs-Removed for the side-effect arms (e.g. `unregister_current` result + an explicit `is_connected`
    check, or extend the registry to return a three-state outcome). Only after that, delete the now-unused
    `authenticated_session_ownership` timestamp helper (verified single caller).
  - **enrolled (session_enrolled.rs):** has **no ownership check at all** — `cleanup_enrolled_session` guards
    only on `cancel_token.is_cancelled()` (a bool), then unconditionally `unregister`s (:214). This is an
    **add**, not a swap: `EnrolledSessionState` gains a `connection_id` field (today it keeps only the
    `cancel_token` from the handle, discarding the rest), threaded to the cleanup call.
  - **embedded:** the `register()` call is at **embedded.rs:257** (which today does `let _ = …handle`, discarding
    it — correction: not `embedded_support.rs`), and `cleanup_embedded_service_session` is a **freestanding fn
    taking individual params**, no session struct. The fix threads `connection_id` from that `register()`
    down through `run_embedded_message_handler*` → `cleanup_embedded_service_session` as a new parameter (stop
    discarding the handle at :257).

### 2. Eliminate the panic by returning `connected_at` from `register()`

`register()` constructs `connected_at: OffsetDateTime::now_utc()` (lib.rs:136) at the moment it creates the
connection — the authoritative value. Add `connected_at` to the returned `ServiceConnectionHandle` (carries only
`connection_id` + `cancel_token` today) so the caller reads it from the handle instead of the second racy
`.connected_at(&service_id)` lookup. Deleting the second lookup removes the *race*, not just the panic — strictly
better than wrapping the same racy lookup in `.ok_or_else(report!)` (which would also force `register_connection`
to become `Result`-returning, rippling typed-error plumbing for a value already known). Delete the production
`.expect()`. Accessor-deletion ripple (review-found): `ServiceConnectionRegistry::connected_at()` has a **second
caller** in shared test scaffolding — `test_support.rs:401` `register_test_connection`, used by many
`session_authenticated.rs` tests. Deleting the accessor means reworking that helper + its call sites first; the
simpler option is to **leave the accessor** (harmless once production stops calling it) and only delete the
production panic. The required change is removing the production `.expect()`; accessor deletion is optional
cleanup.

## Tests

Unit tests on `ServiceConnectionRegistry` (its test module exists) + handler-level tests where the harness
allows:

1. **Race regression (the HIGH):** register connection A (get handle A), register connection B for the same
   `service_id` (B supersedes A, A's token cancelled), then call `unregister_current(service_id,
   A.connection_id)` — assert it returns `false` and **B is still registered / `is_connected()` true**. Contrast
   with the old `unregister()` which would have removed B.
2. **Normal cleanup:** register A, `unregister_current(service_id, A.connection_id)` with no replacement →
   returns `true`, service gone.
3. **`connected_at` from handle:** assert the handle's `connected_at` equals what the registry stored (no second
   lookup needed); and that a `force_disconnect` between register and use cannot panic — construct the sequence
   that previously hit `.expect()` and assert it returns/handles gracefully (the value now comes from the handle,
   so the removed entry is irrelevant).
4. All three cleanup paths covered — the shared registry-level race test above is the core guard, plus a
   handler test per path. **Embedded is not optional** (embedded.rs:211 is a named race site): if the harness
   can't drive the full embedded handler, at minimum assert `cleanup_embedded_service_session` calls
   `unregister_current` with the threaded `connection_id` and no-ops on mismatch. Do not let the embedded fix be
   silently skipped under an "if the harness supports it" hedge — the plumbing through `embedded_support.rs` is
   required for the fix to exist at all.
5. No time-API tests (`connected_at` is a stored value, not a timer); DB-backed handler tests use no
   `start_paused` (snapshot rule).

## Documentation deliverables

- Doc comments: `ServiceConnectionHandle` (now carries `connected_at`), and the three cleanup sites (why
  `unregister_current` not `unregister` — the reconnect race). The `unregister` vs `unregister_current`
  distinction is already documented on the registry; note that plain `unregister` should not be used from
  disconnect-cleanup paths (add a doc-warning on `unregister` pointing to `unregister_current` for cleanup).
- No API/wire/OpenAPI change (internal connection-registry lifecycle; observable effect is that pushes to a
  reconnected agent stop being silently dropped, and the WS task stops panicking — both fixes).
- No new ADR: concurrency-correctness fix using an existing registry primitive.

## Out of scope / deferred

- The surface-proxy in-flight cancellation-safety HIGH (`proxy.rs:348`) — separate crate, separate mechanism
  (RAII drop-guard), its own spec.
- The shutdown-drain `unregister(&service_id)` at `controller-runtime/tasks.rs:115` is **explicitly not in
  scope** (pre-classified during review): it's an intentional unconditional bulk-unregister during graceful
  shutdown (analogous to `force_disconnect`), not a race-prone per-connection cleanup — leave it. The other
  `unregister()` hits (tasks.rs:602/636/642) are `#[cfg(test)]`. So of ~8 call sites, only the 3 cleanup paths
  are targets; the grep needs to filter test modules and the shutdown site.
- Reworking the whole session-ownership model beyond swapping timestamp-compare for `connection_id` (the
  connection_id is the registry's own identity; using it is the minimal correct change).
