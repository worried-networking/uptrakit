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
3. **Post-refresh 401 → silent fake success (MEDIUM; mechanism corrected during review — the audit's
   `[object Object]` description was inaccurate, the real behavior is worse).** The response interceptor
   deliberately excludes 401 from `ApiError` conversion (`if (!response.ok && response.status !== 401)`), and
   `unwrap()` throws only when `result.error` is set — a bare-401 retry has `error === undefined`. So in
   `refreshAndRetry`, a retry that still 401s (user deactivated, permissions revoked) is **returned to the
   caller as the raw `{data?, error?, request?, response?}` wrapper masquerading as a successful payload** —
   no throw, no `ApiError`, rejected token left in the store, session-expired banner cleared in `finally`.
   Callers render garbage or mis-detect success. The sibling first-401 path builds `unauthorizedApiError`
   correctly; only the post-refresh path is broken. No unit test covers the still-401 retry.
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
   existing unit test ("leaves cache untouched when PUT response is not OK") exercises the 428 case and
   pins the no-recovery behavior.

## Approach

Five contained fixes, each following an already-shipped in-repo pattern; no new libraries, no state-management
rework.

### 1. SSE joins the shared refresh path

In `sse.ts` `connect()`: on an HTTP 401 response, await `dedupedRefresh()` — **already exported** from
`api/client.ts` (verified; `api/raw.ts` imports it for the same purpose, so no export change and no new
circular-dep risk) — then schedule the normal reconnect (which re-reads the fresh token).

**Terminality keys on the refresh, not the reconnect** (contrarian-driven): `dedupedRefresh()` rejecting or
returning no token is the true "session revoked" signal → `onStateChange('error')`, stream stops. A 401 on the
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
re-auth). Known accepted gap: a dead-but-cycling events stream surfaces no user-facing signal (follow-up
material, not this spec).

### 2. Surface read-model loads: failed-ids set + guarded effects

Two halves, per the audit:

- In `lib/surfaces/registry.svelte.ts` `loadSurfaceReadModel`: on failure, **stop deleting the tracking
  state — record the failure in a separate `SvelteSet<string>` of failed surface ids** (contrarian-simplified
  from an earlier discriminated-union design: the loop-breaker is simply that the load is not re-armed, and a
  failed-ids set achieves that with **zero** change to `getSurfaceReadModel`'s return type — which five
  production call sites and ~10 test mocks consume; the retry affordance only needs *that* a read failed, not
  the message). Concretely: on error, remove the load promise but insert into `failedReads`; the load guard
  becomes `readsBySurface.has(id) || readLoadPromises.has(id) || failedReads.has(id)`. The write-only
  `readRequestedBySurface` flag is deleted (dead state). A **new** exported `refreshSurfaceReadModel(surfaceId)`
  (none exists today — verified) clears the failed mark and re-fetches, for the retry affordance surface UIs
  will call.
- In the three unguarded call sites (`hosts/[id]`, `software/[id]` ×2 effects, `software/+page`): key the
  effect on the surface-id list and run loaders untracked — `untrack(() => loadSurfaceReadModels(...))`,
  matching the in-repo precedent at `software/+page.svelte` (`untrack(() => loadAll(pg))`). Precision note:
  `surfaces/[id]/+page.svelte` guards with requested/loading **flags** and no `untrack` — a valid alternative
  shape; the untrack form is chosen here because the failed-ids guard makes per-page flags redundant.
  Defense in depth: the failed-ids set alone stops the infinite loop; untrack stops redundant re-fires.

### 3. `refreshAndRetry` post-refresh 401 → proper `ApiError`

In `refreshAndRetry`, inspect the retried fields result **before** `unwrap()` returns it as data: if
`result.response?.status === 401`, throw `unauthorizedApiError(result.response, result.error)` — the same
constructor the first-401 path uses — clear the access token (`setAccessToken(null)`), and leave the
session-expired banner raised (matching `mapRefreshFailure`'s 4xx policy). There is no existing throw site for
this case (the interceptor excludes 401 by design for the refresh machinery); this check is the new, single
point where a post-refresh 401 becomes a typed error instead of fake success.

### 4. Detail pages reload on param change

Replace the `onMount` data loads in `hosts/[id]` and `software/[id]` with the effect-pattern shape documented
in `docs/development/ui/primitives.md` and shipped in `software/+page.svelte`:
`$effect(() => { void id; untrack(() => { loadData(); … }); })` — the effect keys on the derived `id`, loaders
run untracked (the Svelte 5 rule against loader-triggered loops; `afterNavigate` has zero in-repo usage and is
not the house pattern). `onMount`-only concerns
(one-time subscriptions) stay in `onMount`.

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
("Settings were changed elsewhere — reload and retry"), not a new Callout (noting `notifications.svelte.ts`
holds a single active error message; acceptable, the save failure is the dominant error at that moment). No
auto re-GET-and-retry: a silent auto-merge over someone else's concurrent change is the wrong default for
settings. The existing 428 test ("leaves cache untouched when PUT response is not OK") stays as-is — it covers
the missing-header case correctly; a **new** case covers 409 `if_match.stale` → cache cleared.

## Tests

Vitest (existing harness; coverage thresholds apply):

1. `sse.ts`: 401 → one deduped refresh → reconnect with new token; refresh **failure** → terminal error state,
   no further reconnects; 401-after-successful-refresh → stream keeps cycling (not terminal), settling into
   backoff-paced cycles (assert no tight loop — one refresh per cycle). (Mock fetch; the dedup assertion is
   that two concurrent streams share one refresh call.)
2. `registry.svelte.ts`: failed read marks `failedReads` and is **not** re-requested by a re-run effect;
   `refreshSurfaceReadModel` clears the mark and re-fetches.
3. `client.refresh.test.ts`: new case "retry still 401s" — written against the **corrected** implementation
   (today's behavior is silent fake success, so the test lands with the fix, asserting `ApiError` identity,
   status 401, preserved `error_code`, token cleared, banner not cleared).
4. Param navigation: component test (or route-level test per existing harness) — id change triggers reload;
   same-id re-render does not.
5. `client.etag.test.ts`: existing 428 case stays; new 409 `if_match.stale` case asserts cache cleared +
   typed error surfaced; other non-OK statuses still leave the cache untouched.

## Documentation deliverables

- `docs/development/ui/primitives.md` ("Effect pattern" note — the actual canonical home of the
  `$effect`/`untrack` data-load convention; `frontend/AGENTS.md` does not document it, verified): extend with
  the failed-ids retain-on-error convention for shared store-backed loaders and the param-keyed reload shape from fix 4.
- Doc comments on `loadSurfaceReadModel`/`refreshSurfaceReadModel` (retain-on-error semantics) and the exported deduped refresh.
- No backend/API/wire changes; no OpenAPI regen; no ADR (frontend-internal reliability fixes).

## Out of scope / deferred

- Proactive token-refresh timer (`expires_in`-based) — the on-401 refresh now covers SSE, the last consumer
  outside it; a timer is an optimization with its own failure modes.
- Auto re-GET + retry on 412 (explicit-message UX chosen; revisit only if users report friction).
- The frontend maintainability mediums from the audit (component decomposition etc.) — separate concern.
- e2e coverage for the history page (separate audit finding).
- Generalizing the ETag layer beyond settings scopes (existing deferred item in the tracker).
