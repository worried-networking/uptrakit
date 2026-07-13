# Frontend Session & Data-Load Reliability — Design

**Date:** 2026-07-11
**Status:** Draft
**Source:** `.superpowers/audit-2026-07-11.md` — HIGH "SSE streams bypass the 401 token-refresh path entirely"
(found by two auditors) + HIGH "Unguarded surface read-model $effect retries failed reads in an unbounded request
loop" + MEDIUM "401-after-refresh surfaces Error(\"[object Object]\")" + MEDIUM "Detail pages load data only in
onMount — param-only navigation shows stale entity" + MEDIUM "Stale cached settings ETag has no 412 recovery".

## Problem

Five frontend reliability defects, all in the session/data-load layer (`frontend/src/lib` + route pages):

1. **SSE outside the refresh machinery (HIGH).** `sse.ts` does its own raw fetch with `getAccessToken()`,
   fully outside `api/client.ts`'s deduped 401-refresh. There is no proactive refresh timer anywhere — refresh
   happens only on 401 through the API wrappers. Once the in-memory access token expires: the central events
   stream (`maxReconnectAttempts: Infinity`) gets 401, treats it as a generic error, and reconnects every 30s
   forever **with the same stale token** — live dashboard updates silently die while the server is hammered;
   the update output stream (max 5 attempts) gives up after ~31s, killing the output panel of a long-running
   interactive update unless an unrelated API call happens to rotate the token. `sse.ts` also duplicates the
   BASE-url resolution from `api/client.ts`.
2. **Unbounded surface-read retry loop (HIGH).** `hosts/[id]/+page.svelte` calls
   `void loadSurfaceReadModels(...)` in an `$effect` without `untrack` and without the requested/loading
   guard. `loadSurfaceReadModel` reads two reactive `SvelteMap`s inside the tracking context, and on fetch
   **failure deletes both entries** — flipping the tracked deps and re-running the effect immediately: an
   infinite, backoff-free request loop for as long as the read errors. Same unguarded pattern in
   `software/[id]/+page.svelte` (two effects) and `software/+page.svelte`; `surfaces/[id]/+page.svelte`
   already guards correctly — the hazard is known but inconsistently mitigated.
3. **Post-refresh 401 → opaque `Error("[object Object]")` (MEDIUM; the audit's description was RIGHT — an
   earlier draft of this spec "corrected" it to silent-fake-success, which empirical verification during
   review disproved).** Mechanism, traced and run: the response interceptor excludes 401 from `ApiError`
   conversion (`if (!response.ok && response.status !== 401)`), but the **generated** client
   (`client.gen.ts`) still gates on `response.ok` internally — under `throwOnError:false` a still-401 retry
   produces `{ error: <parsed body object>, request, response }` with `error` always set. `unwrap()` throws
   that plain object; `refreshAndRetry`'s catch feeds it to `translateFetchError`, which (not an
   `Error`/`TypeError`/`DOMException`) returns `new Error(String(err))` — literally
   `Error("[object Object]")`. So a retry that still 401s (user deactivated, permissions revoked) throws an
   untyped, message-less error: no `ApiError`, no status, rejected token left in the store, session-expired
   banner cleared in `finally`. Callers cannot distinguish session death from a network blip. The sibling
   first-401 path builds `unauthorizedApiError` correctly; only the post-refresh path is broken. No unit test
   covers the still-401 retry.
4. **onMount-only detail loads (MEDIUM).** `hosts/[id]` derives `id` from `page.params` but loads only in
   `onMount`; SvelteKit reuses the component on param-only navigation, so `/hosts/A → /hosts/B` keeps showing
   A's data under B's URL — while SSE filters now match B and would splice B's events over A's view. Reachable
   via surface-table entity links. `software/[id]` has the identical pattern.
5. **Stale-ETag saves have no recovery (MEDIUM; status codes corrected during review — the audit's "412"
   does not exist in this codebase).** The backend etag middleware returns **409 `if_match.stale`** on ETag
   mismatch and **428 `if_match.required`** when the header is missing; there is no 412 anywhere.
   `applyIfMatch` auto-attaches the cached per-scope settings ETag; `captureEtag` updates only on
   `response.ok`. A concurrent editor bumping the settings version leaves this tab's cached ETag stale →
   every save retry sends the same `If-Match` and hits 409 `if_match.stale` again, until hard reload. The
   existing unit test ("leaves cache untouched when PUT response is not OK") exercises the 428 case and,
   pins the no-recovery behavior.

## Approach

Five contained fixes, each following an already-shipped in-repo pattern; no new libraries, no state-management
rework.

### 1. SSE joins the shared refresh path

In `sse.ts` `connect()`: on an HTTP 401 response, await `dedupedRefresh()` — **already exported** from
`api/client.ts` (verified; `api/raw.ts` imports it for the same purpose, so no export change and no new
circular-dep risk) — then schedule the normal reconnect (which re-reads the fresh token).

**Terminality keys on the refresh, not the reconnect — and only on auth-class refresh failure** (contrarian,
both passes): `dedupedRefresh()` rejects on ANY failure, including transient network errors and 5xx on
`/auth/refresh` — treating every rejection as "session revoked" would let one unlucky shared refresh kill both
streams permanently. Reuse `mapRefreshFailure`'s existing discrimination: a **4xx/auth-class** refresh failure
is the true "session revoked" signal → `onStateChange('error')`, stream stops; a transient failure
(timeout/`TypeError`/5xx) re-enters the backed-off reconnect/refresh cycle. A 401 on the
reconnect *after* a successful refresh is ambiguous (token-propagation lag, multi-tab rotation races — the very
scenario the dedup test models) and must **not** kill the `Infinity`-reconnect events stream permanently; those
just re-enter the normal reconnect/refresh cycle — bounded for the finite-attempt output stream by its existing
`maxReconnectAttempts`, and no longer a stale-token hammer for the events stream because every cycle now awaits
the deduped refresh (fresh token or terminal refresh failure). While in the file, drop the duplicated BASE
resolution by importing it from `./api/client`. No change to reconnect pacing for non-401 failures;
`maxReconnectAttempts` semantics stay. Ordering, load-bearing: the `dedupedRefresh()` await sits **inside the
backed-off reconnect cycle** (after the delay), not immediately on 401 — otherwise the refresh endpoint becomes
the hot path a persistently-401ing valid token would hammer; correctly ordered, that case settles into one
refresh + one connect per backoff period (benign, and correct liveness — access may be restored without
re-auth). Scope note: `dedupedRefresh` dedup is **per-tab** (per JS context) — N tabs waking from expiry issue
N refreshes; correct (refresh-token cookie) and pre-existing, cross-tab coordination out of scope. Known
accepted gap: a dead-but-cycling events stream surfaces no user-facing signal (follow-up material, not this
spec).

### 2. Surface read-model loads: failed-ids set + guarded effects

Two halves, per the audit:

- In `lib/surfaces/registry.svelte.ts` `loadSurfaceReadModel`: on failure, **stop deleting the tracking
  state — record the failure in a separate `SvelteSet<string>` of failed surface ids** (contrarian-simplified
  from an earlier discriminated-union design: the loop-breaker is simply that the load is not re-armed, and a
  failed-ids set achieves that with **zero** change to `getSurfaceReadModel`'s return type — consumed by
  multiple production call sites (grep at implementation time; do not trust a count) and ~10 test mocks; the
  retry affordance only needs *that* a read failed, not the message). Concretely: on error, remove the load
  promise but insert into `failedReads`; the load guard becomes
  `readsBySurface.has(id) || readLoadPromises.has(id) || failedReads.has(id)`. **`readRequestedBySurface`
  stays** — correction from an earlier draft that called it write-only dead state: `getSurfaceReadRequested`
  reads it and is load-bearing in production (the effect guard + loading-skeleton derivation in
  `surfaces/[id]/+page.svelte`, and the `isReadRequested` input to `isSurfaceTabPending()` on the settings,
  software, and software/[id] pages) — deleting it would break those. `failedReads` is added alongside it. A
  **new** exported `refreshSurfaceReadModel(surfaceId)` (none exists today — verified) clears the failed mark
  and re-fetches. **Honesty correction (contrarian): no surface UI calls a read-model retry today** — the only
  existing retry control (`SurfaceReadPanel`'s `hydrationRetryNonce`) sits on the separate
  `invokeSurfaceInteraction` hydration path, and all six `getSurfaceReadModel` consumers render an absent entry
  silently. Without recovery, the failed-ids guard would trade the infinite loop for a **session-permanent blank
  surface** — worse for the user than the loop. Therefore the failure marks get an explicit **eviction policy**
  (this is the load-bearing half of the fix):
  - cleared in `clearSurfaceRegistry` (logout) and `loadSurfaceRegistry` (registry re-fetch) — add to both;
  - cleared **per consuming-page navigation**: the keyed effects from the second half below clear the failed
    marks for their surface ids (via `refreshSurfaceReadModel`) before loading — one implicit retry per
    navigation, bounded (a failure within one page-view stays failed for that view; no loop re-arms because the
    clear happens once per key change, not per failure).
  A visible per-surface retry control wired to `refreshSurfaceReadModel` is **deferred** (named follow-up, not
  claimed here); the export exists for it and for the navigation-clear above.
- In the three unguarded call sites (`hosts/[id]`, `software/[id]` ×2 effects, `software/+page`): key the
  effect on the surface-id list and run loaders untracked — `untrack(() => loadSurfaceReadModels(...))`,
  matching the in-repo precedent at `software/+page.svelte` (`untrack(() => loadAll(pg))`). Precision note:
  `surfaces/[id]/+page.svelte` guards with requested/loading **flags** and no `untrack` — a valid alternative
  shape that is deliberately left as-is (the failed-ids guard lives inside `loadSurfaceReadModel` itself, so
  that page is protected by it too; its per-page flags become harmless redundancy that also drives its
  loading-skeleton UI — converging it is churn without payoff). Two guard idioms will coexist by choice; the
  registry-level failed-ids guard is the invariant, page idioms are presentation. Defense in depth: the
  failed-ids set alone stops the infinite loop; untrack stops redundant re-fires.

### 3. `refreshAndRetry` post-refresh 401 → proper `ApiError`

In `refreshAndRetry`, inspect the retried fields result **before** `unwrap()` runs — and close the **class**,
not just the 401 instance (contrarian): any retry response with `!result.response.ok` gets converted to a typed
`ApiError` from `result.error` (mirroring what the interceptor does for first-call non-401s — a post-refresh
403 from revoked permissions would otherwise hit the same opaque `Error("[object Object]")` path). The 401 case
additionally throws via `unauthorizedApiError(result.response, result.error)` — the same constructor the
first-401 path uses — clears the access token (`setAccessToken(null)`), and leaves the session-expired banner
raised (matching `mapRefreshFailure`'s 4xx policy). Placing the check before `unwrap()` means the opaque throw
path (plain-object → `translateFetchError` → `Error("[object Object]")`) is never reached for any non-OK retry.

### 4. Detail pages reload on param change

Replace the `onMount` data loads in `hosts/[id]` and `software/[id]` with the effect-pattern shape documented
in `docs/development/ui/primitives.md` and shipped in `software/+page.svelte`:
`$effect(() => { void id; untrack(() => { loadData(); … }); })` — the effect keys on the derived `id`, loaders
run untracked (the Svelte 5 rule against loader-triggered loops; `afterNavigate` has zero in-repo usage and is
not the house pattern). `onMount`-only concerns
(one-time subscriptions) stay in `onMount`.

**Stale-response guard, required (contrarian):** keying on `id` fixes *which trigger reloads* but not
*out-of-order resolution* — navigate `/hosts/A → /hosts/B` fast and A's slower response resolving after B's
clobbers B's entity state (the exact bug, reintroduced as a race). Every keyed loader that writes single-entity
page state captures the `id` it loaded for and **discards the response if the current `id` has changed** before
committing (per-load generation check; `loadSurfaceReadModels` is already safe — it writes a per-id map). The
param-navigation test must include an out-of-order-resolution case, not only single id-change.

Pre-condition, made explicit (not "verify later"): per page, list every reactive value each untracked loader
reads and confirm it is either the keyed `id` or genuinely load-once — `untrack` converts "loads once" into
"reloads on `id` and **only** `id`", correct only if `id` is the sole reload trigger. `hosts/[id]` is clean
(loaders close over `id` alone — verified). `software/[id]` has additional reactive state in scope
(`activeTab`, tab-group derivations, allowlist state) — any loader that should re-fire on those must be keyed
on them explicitly (touch before `untrack`) or left out of the untracked block; `loadAllTags` and other
id-independent loaders stay one-time in `onMount`.

### 5. Stale-ETag recovery for settings saves (409 `if_match.stale`)

In the client response path (where `captureEtag` lives): on a **409 whose `error_code` is `if_match.stale`**
for a settings-scope PUT/PATCH, clear `settingsEtagCache[scope]`. No new errorCode is invented — the backend
already sends `if_match.stale` (the codebase's `<scope>.<reason>` convention), and the interceptor already
converts non-401 non-OK responses to `ApiError`, so callers can match on it today; the missing piece is only
the cache-clear. Surfacing follows the codebase's universal mutation-failure pattern — `showError()` toast
("Settings changed since you loaded this page — please reload and try again" — softened wording: a 409
`if_match.stale` can also come from this tab's own missed ETag capture, so don't assert "changed elsewhere"),
raised as the **terminal** action of the save handler so no later error overwrites it (noting
`notifications.svelte.ts` holds a single active error message; acceptable, the save failure is the dominant
error at that moment). No
auto re-GET-and-retry: a silent auto-merge over someone else's concurrent change is the wrong default for
settings. The existing 428 test ("leaves cache untouched when PUT response is not OK") stays as-is — it covers
the missing-header case correctly; a **new** case covers 409 `if_match.stale` → cache cleared.

## Tests

Vitest (existing harness; coverage thresholds apply):

1. `sse.ts`: 401 → one deduped refresh → reconnect with new token; **auth-class** refresh failure (4xx) →
   terminal error state, no further reconnects; **transient** refresh failure (network/5xx) → keeps cycling;
   401-after-successful-refresh → stream keeps cycling (not terminal), settling into backoff-paced cycles
   (assert no tight loop — one refresh per cycle). (Mock fetch; the dedup assertion is that two concurrent
   streams share one refresh call.)
2. `registry.svelte.ts`: failed read marks `failedReads` and is **not** re-requested by a re-run effect;
   `refreshSurfaceReadModel` clears the mark and re-fetches; eviction: `clearSurfaceRegistry` and
   `loadSurfaceRegistry` clear `failedReads`; a keyed-effect navigation clears the page's surface marks (one
   implicit retry per navigation). Loop-freedom must be asserted on the **re-failure** case: navigate → load
   fails → mark set → assert stable fetch count across the reactive settle (the set-then-clear churn must not
   re-fire the effect — a single-navigation single-fetch assertion proves nothing).
3. `client.refresh.test.ts`: new case "retry still 401s" — written against the **corrected** implementation
   (today's behavior throws opaque `Error("[object Object]")`, so the test lands with the fix, asserting
   `ApiError` identity, status 401, preserved `error_code`, token cleared, banner not cleared). SSE/backoff
   tests use Vitest fake timers — no real waits.
4. Param navigation: component test (or route-level test per existing harness) — id change triggers reload;
   same-id re-render does not; **out-of-order resolution**: A's slower response resolving after B's is
   discarded (generation guard), B's entity state wins.
5. `client.etag.test.ts`: existing 428 case stays; new 409 `if_match.stale` case asserts cache cleared +
   typed error surfaced; other non-OK statuses still leave the cache untouched.

## Documentation deliverables

- `docs/development/ui/primitives.md` ("Effect pattern" — today a bolded callout inside the `### createUrlParam`
  subsection, not a standalone section; `frontend/AGENTS.md` does not document it, verified): promote it to its
  own subsection and extend with the failed-ids retain-on-error convention for shared store-backed loaders and
  the param-keyed reload shape from fix 4 (this spec ADDS that shape to the doc; fix 4 follows the shape,
  citing the shipped `software/+page.svelte` precedent).
- Scope note: all five fixes modify existing `.ts`/`.svelte` files only — no new files.
- Circular-import check to run at implementation (name the chain, don't hand-wave): `sse.ts` gains an import of
  `dedupedRefresh`/BASE from `api/client.ts`; today `sse.ts` imports only `getAccessToken` from `auth.svelte`,
  while `client.ts` imports from `token-store.svelte` — verify no path from `client.ts` back into `sse.ts`.
- Doc comments on `loadSurfaceReadModel`/`refreshSurfaceReadModel` (retain-on-error semantics) and the exported
  deduped refresh. `refreshSurfaceReadModel`'s contract, stated in its doc comment: it mutates **only**
  `failedReads` and triggers the load — it must NOT delete `readsBySurface`/`readRequestedBySurface` entries
  (recovery must not flip deps that skeleton/pending derivations watch — same mutates-tracked-state class as the
  original bug).
- Generation-guard granularity (fix 4): the guard wraps only the **single-entity commit** — not the whole
  untracked block (`loadSurfaceReadModels` is per-id-map-safe and must not be discarded by the entity guard).
- No backend/API/wire changes; no OpenAPI regen; no ADR (frontend-internal reliability fixes).

## Out of scope / deferred

- Proactive token-refresh timer (`expires_in`-based) — the on-401 refresh now covers SSE, the last consumer
  outside it; a timer is an optimization with its own failure modes.
- Auto re-GET + retry on 412 (explicit-message UX chosen; revisit only if users report friction).
- The frontend maintainability mediums from the audit (component decomposition etc.) — separate concern.
- e2e coverage for the history page (separate audit finding).
- Generalizing the ETag layer beyond settings scopes (existing deferred item in the tracker).
