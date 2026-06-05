# Frontend Settings ETag Auto-Injection

## Summary

The Dashboard frontend currently fails to save several controller settings panels with the
backend error `if-match header is required`. Two settings helpers know how to round-trip ETags
(`frontend/src/lib/api/oauth.ts`, `frontend/src/lib/api/settings.ts`); every other settings
endpoint that goes through the generic `request()` helper in `frontend/src/lib/api.ts` is broken
because `request()` does not attach `If-Match`.

This design centralizes ETag handling inside `request()` (and `requestVoid()`) using two
module-scoped caches keyed by URL scope prefix (`/global-settings/*` and `/settings/*`), then
removes the explicit `{ data, etag }` envelope plumbing from the two existing helpers so the
codebase has one pattern. The chosen approach mirrors the backend's actual data model (one
ETag per scope, shared by every endpoint in that scope) and requires no component-level ETag
state.

## Background

### Current backend contract

Both `/api/v1/global-settings/*` and `/api/v1/settings/*` are wrapped with
`etag_middleware` at `crates/ui/web-api/src/router.rs:880-902`:

- `PUT`/`PATCH` without `If-Match` → `428 Precondition Required`,
  `{"error":"if-match header is required","code":"if_match.required"}`
  (`crates/ui/web-api/src/middleware/etag.rs:30-37`).
- `PUT`/`PATCH` whose `If-Match` does not equal current scope version →
  `409 Conflict`, `{"code":"if_match.stale"}` (`etag.rs:69-78`).
- All `2xx` responses (GET, PUT, PATCH) get an `ETag` header injected
  (`etag.rs:83-105`). `refresh_etag()` re-reads the version after every mutation.

There are exactly two scopes: `GlobalSettingsVersion` (for `/global-settings/*`) and
`SettingsVersion` (for `/settings/*`). Both are scope-wide; saving any endpoint inside a
scope bumps the version for every other endpoint in that scope. The backend already enforces
this — the frontend just needs to track and forward the latest value per scope.

### Current frontend split

| Path                                | Current caller                                                 | ETag handling          |
| ----------------------------------- | -------------------------------------------------------------- | ---------------------- |
| `/global-settings/oauth`            | `frontend/src/lib/api/oauth.ts` (uses `authenticatedFetch`)    | Explicit `{data,etag}` |
| `/settings/access`                  | `frontend/src/lib/api/settings.ts` (uses `authenticatedFetch`) | Explicit `{data,etag}` |
| `/global-settings/network`          | `frontend/src/lib/api.ts:628,632` (uses `request()`)           | None — broken          |
| `/global-settings/nats`             | `frontend/src/lib/api.ts:705,709`                              | None — broken          |
| `/global-settings/providers/github` | `frontend/src/lib/api.ts:715,721`                              | None — broken          |
| `/global-settings/zeroconf`         | `frontend/src/lib/api.ts:727,731`                              | None — broken          |
| `/settings/agent-certificates`      | `frontend/src/lib/api.ts:587,593`                              | None — broken          |

Five broken save buttons in production. Two working ones with bespoke per-endpoint plumbing
that no other endpoint copies.

## Goals

- Every settings `PUT`/`PATCH` from the Dashboard succeeds when sent against an unmodified
  current backend version.
- Stale-version conflicts (`409 if_match.stale`) continue to surface to the user via the
  existing error toast path; no silent swallowing.
- One implementation pattern for ETag handling across all settings endpoints.
- Future settings endpoints added under `/global-settings/*` or `/settings/*` work
  automatically without per-endpoint plumbing.

## Non-Goals

- Backend changes. The middleware contract is correct and remains untouched.
- Automatic conflict resolution / retry / merge UI for `409 if_match.stale`. Today's
  "show error toast, user reloads" UX is preserved.
- Generalizing ETag handling to non-settings endpoints. Other routes that may eventually
  use `If-Match` (none currently) can opt in explicitly.
- Cross-tab cache synchronization. Two browser tabs editing the same scope simultaneously
  will continue to produce `409` on the loser; that is the desired behavior.

## High-Level Design

### Scope-keyed cache in `lib/api.ts`

Add a module-scoped two-key `Record` (exhaustive over the union) plus a path-prefix
discriminator:

```typescript
type SettingsScope = 'global' | 'tenant';

const settingsEtagCache: Record<SettingsScope, string | null> = {
	global: null,
	tenant: null
};

function settingsScope(path: string): SettingsScope | null {
	if (path.startsWith('/global-settings/')) return 'global';
	if (path.startsWith('/settings/')) return 'tenant';
	return null;
}
```

A `Record` over the union (rather than `Map`) keeps `Record<SettingsScope, …>` exhaustive
under `strict` and removes the `.get()`/`.set()` noise; the field count is fixed by the
backend's two ETag scopes.

### Augmented `request()` only

For every `request()` call:

1. Compute `scope = settingsScope(path)`.
2. If method is `PUT` or `PATCH` and `scope !== null` and caller has not provided their own
   `if-match` header, inject the cached value when present. Bypass detection uses one line
   that covers all three `HeadersInit` shapes (`Headers`, `[string,string][]`, `Record`):

   ```typescript
   const callerHas = new Headers(options.headers ?? {}).has('if-match');
   ```

   If the cache slot is `null` (no GET has happened yet), send the request without
   `If-Match` and let the backend reject with `428`; the existing error toast surfaces and a
   reload re-populates the cache.

3. After `authenticatedFetch` returns: if `res.ok` and `scope !== null`, read
   `res.headers.get('etag')` and assign it to `settingsEtagCache[scope]`. This covers both
   GET (initial fetch) and PUT/PATCH (post-mutation refresh). **The cache-write is
   unconditional** — it applies whether or not the caller supplied their own `If-Match`.
   The bypass flag from step 2 suppresses _injection_, never _storage_; never add an
   `if (!callerHas)` guard around the write.
4. On non-OK responses, leave the cache untouched.

`requestVoid()` is **not** augmented. Its only `/settings/*` caller today is a DELETE
(`/settings/oidc-providers/{id}`) that is outside `etag_middleware`; DELETE semantics do
not fit optimistic-concurrency writes and no plausible future settings DELETE will. Keeping
the hook to a single function avoids dead behavior and shrinks the test surface.

`authenticatedFetch` is unchanged.

### Cache reset on auth change

Cross-user pollution: a logout/login or 2FA re-auth in the same tab leaves tenant A's ETag
in cache; tenant B's first save then 409s for no user-visible reason. Mitigation must
distinguish a **user change** (login, logout, re-auth as different user) from a **silent
token refresh** (same user, new `access_token` string). Token-string equality is wrong —
`refreshAccessToken()` in `auth.svelte.ts:49` rotates the string for the same user, and
naive "different string ⇒ reset" would wipe the cache on every refresh, making the
already-noted long-idle `409` UX hit happen on every routine token rotation.

**Detect on the JWT `sub` claim**, not the token string. Decoder lives module-local in
`lib/api.ts` (no new dep; tokens are JWTs already produced by this controller):

```typescript
function subClaim(token: string | null): string | null {
	if (!token) return null;
	const parts = token.split('.');
	if (parts.length < 2) return null;
	try {
		let b64 = parts[1].replace(/-/g, '+').replace(/_/g, '/');
		b64 += '='.repeat((4 - (b64.length % 4)) % 4); // base64url → base64 padding
		const payload = JSON.parse(atob(b64));
		return typeof payload.sub === 'string' ? payload.sub : null;
	} catch {
		return null;
	}
}
```

**Wire via a subscriber callback on `token-store`**, not a back-edge import:
`token-store.svelte.ts` already exists explicitly to break the `api.ts` ↔ `auth.svelte.ts`
cycle (see its file header). Adding `import { resetSettingsEtagCache } from './api'` to
`token-store` reintroduces that cycle. Instead, extend `token-store` with a listener
registration:

```typescript
type TokenChangeListener = (prev: string | null, next: string | null) => void;
const tokenChangeListeners: Set<TokenChangeListener> = new Set();

export function onTokenChange(cb: TokenChangeListener): () => void {
	tokenChangeListeners.add(cb);
	return () => tokenChangeListeners.delete(cb);
}

export function setAccessToken(token: string | null): void {
	const prev = accessToken;
	accessToken = token;
	for (const cb of tokenChangeListeners) cb(prev, token);
}
```

A `Set` plus an unsubscribe return value keeps listener registration idempotent under
Vite/SvelteKit HMR — if `api.ts` is re-evaluated mid-session, the same callback identity
is deduplicated and a future cleanup hook can opt-in.

Then in `lib/api.ts`, register at module init:

```typescript
onTokenChange((prev, next) => {
	if (subClaim(prev) !== subClaim(next)) {
		settingsEtagCache.global = null;
		settingsEtagCache.tenant = null;
	}
});
```

This preserves the one-way `api.ts → token-store` dependency arrow, resets only on real
user changes (login, logout, re-auth as different user), and is self-correcting: future
`setAccessToken` callers do not need to remember to pass a "reason" flag.

### Idiomatic Svelte/TS shape

- Module-scoped `const Map` is the idiomatic place to hold cross-call client state in a
  single-page app. No need for stores or `$state` — the cache is not reactive and no
  component re-renders depend on it.
- Path-prefix detection is a small pure function; no class, no plugin abstraction. Mirrors
  the backend's `etag_middleware<S>` generic parameter (one type per scope) at the lowest
  conceptual cost.
- No new dependency. No `fetch` wrapper layering. Existing `request()` signature is unchanged.

### Migration: drop explicit envelopes from `oauth.ts` and `settings.ts`

Both helpers currently bypass `request()` and call `authenticatedFetch` directly only to read
the response `etag` header. Once the cache handles this transparently, the helpers can be
collapsed to:

- `getOAuthSettings()` → `Promise<OAuthSettingsResponse>` (was `OAuthSettingsWithEtag`)
- `updateOAuthSettings(body)` → `Promise<OAuthSettingsResponse>` (drop `etag` param)
- `getAccessSettings()` → `Promise<AccessSettingsData>`
- `updateAccessSettings(body)` → `Promise<AccessSettingsData>` (drop `etag` param)

Implementations switch to `request()`. The `OAuthSettingsWithEtag` type in `oauth.ts` and
`AccessSettingsWithEtag` in `lib/types.ts` are deleted.

### Caller updates

Two Svelte components destructure the envelope and track an `etag` `$state`:

- `frontend/src/routes/settings/AccessSettings.svelte:54, 82`
- `frontend/src/routes/settings/McpAccessTab.svelte:149, 166`

Both lose the destructure and the `etag` local state; they call the helpers and use the
returned data directly.

Two test mock files resolve with envelope shape:

- `frontend/src/routes/settings/AccessSettings.test.ts:5, 9`
- `frontend/src/routes/settings/surface-tabs.test.ts:41`

Mocks switch to resolving plain data.

### Contract / behavior table

| Scenario                                           | Behavior                                                                                                       |
| -------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| First load of a settings page                      | GET populates cache; subsequent saves succeed.                                                                 |
| Save without prior GET (programmatic)              | `428` from backend, error toast, no cache update.                                                              |
| Save → backend mutates → response carries new ETag | Cache updates; next save uses fresh ETag.                                                                      |
| Two tabs, tab A saves, tab B saves stale           | Tab B: `409 if_match.stale`, error toast. Cache stays stale until tab B GETs again.                            |
| External `If-Match` passed by caller               | Honored verbatim; cache **not read** for header injection. Response `ETag` still updates the cache on success. |
| Non-settings path                                  | `request()` behavior unchanged; scope is `null`.                                                               |

## Files Touched

| File                                                  | Change                                                                                                                                                |
| ----------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `frontend/src/lib/api.ts`                             | Add cache + scope helper; augment `request()` only; add `subClaim()` decoder; register `onTokenChange` listener that resets cache when `sub` differs. |
| `frontend/src/lib/api.test.ts`                        | Add cache-behavior tests (new file or extend existing if present).                                                                                    |
| `frontend/src/lib/token-store.svelte.ts`              | Add `onTokenChange(cb)` listener registration; have `setAccessToken()` invoke listeners with `(prev, next)`.                                          |
| `frontend/src/lib/api/oauth.ts`                       | Simplify; drop envelope and `etag` param; route via `request()`.                                                                                      |
| `frontend/src/lib/api/settings.ts`                    | Same simplification.                                                                                                                                  |
| `frontend/src/lib/types.ts`                           | Remove `AccessSettingsWithEtag`.                                                                                                                      |
| `frontend/src/routes/settings/AccessSettings.svelte`  | Drop `etag` state, drop destructure.                                                                                                                  |
| `frontend/src/routes/settings/McpAccessTab.svelte`    | Same.                                                                                                                                                 |
| `frontend/src/routes/settings/AccessSettings.test.ts` | Update mock return shape.                                                                                                                             |
| `frontend/src/routes/settings/surface-tabs.test.ts`   | Update mock return shape.                                                                                                                             |

## Dependencies

No new external dependencies. All work is within the existing TypeScript / Svelte 5 frontend.

## Risks & Mitigations

- **Hidden state**: `request()` gains module-scoped cache state. Mitigation: scope is strictly
  limited to two URL prefixes; cache is read-only from outside; behavior is identical for any
  caller that supplies its own `If-Match`.
- **Stale cache between distinct browser sessions**: Inherent to the optimistic-concurrency
  model. Mitigation: backend already returns `409 if_match.stale` and frontend surfaces it via
  existing error toast.
- **Stale cache after long idle in a single tab**: A user who leaves a settings tab open
  for hours, then clicks Save without refreshing, will hit `409 if_match.stale` instead of
  the cleaner `428 if-match required` they would get today. Recoverable via reload, but a
  worse mental model ("someone else changed it" when nobody did). Accepted UX tradeoff;
  most settings panels already GET on mount via `$effect`, so normal navigation refreshes
  the cache. A future improvement (auto-refetch on 409) is listed under Deferred.
- **Cross-user cache pollution within same tab**: Mitigated by JWT `sub`-claim diffing
  inside an `onTokenChange` listener (see _Cache reset on auth change_ above). The
  listener is the _only_ path that clears the cache — silent token refreshes (same `sub`,
  new token string) do not reset, avoiding spurious `409`s after every refresh.
- **JWT `sub` decode failures**: If a future token format omits or hides `sub`, the
  decoder returns `null` for both sides and the cache is **not** reset; this is fail-safe
  for refresh (no spurious wipe) but means a true user change might also not wipe.
  Detectable via the cache-behavior test suite; revisit if token shape changes.
- **POST endpoints in `/settings/*` (e.g. `/settings/renew-server-certificate`, `/settings/oidc-providers`)**:
  not under `If-Match` middleware; `request()` will not attach `If-Match` because method is
  `POST`. No change to those flows.
- **DELETE on `/settings/oidc-providers/{id}`**: not under `If-Match` middleware today; no
  change.

## Testing Strategy

### Automated

- New cache-behavior tests in `frontend/src/lib/api.test.ts` (or alongside existing api
  tests) — required by AGENTS.md "New logic covered by success+failure path tests":
  1. **Success path**: stub `fetch` to return `ETag: "v1"` on `GET /global-settings/network`,
     then stub a subsequent `PUT /global-settings/network` and assert the outgoing
     `Request.headers` contains `if-match: v1`.
  2. **Bypass path**: caller passes their own `if-match: "custom"` on `PUT`; assert outgoing
     header equals `custom` (cache value not substituted).
  3. **Failure path**: stub `PUT` to return `428`; assert the cache slot retains its prior
     value (no overwrite on non-OK).
- Existing unit tests cover the migrated helpers via mocks; updated mocks return plain data.
- Frontend quality gates remain authoritative:
  `cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build`.
- Backend integration tests `crates/ui/web-api/src/integration_tests/if_match.rs` are
  unaffected (no backend change) but should be re-run as sanity:
  `cargo test -p uptrakit-web-api --features db-sqlite if_match`.

### Manual verification (golden path)

Start dev server. As a user with `ManageGlobalSettings` and tenant management permissions:

1. Open Global Settings tab → Save each panel: Network, NATS (both Save and Clear),
   GitHub Provider, Zeroconf. Each must succeed with the existing success toast.
2. Open General Settings tab → Save Access Settings; save Agent Certificate Settings.
3. Open MCP Access tab → Save OAuth Settings.
4. Two-tab regression: open Network panel in tab A and tab B; save in tab A; save in tab B.
   Tab B must show the `etag mismatch (stale version)` error toast (proves cache update path
   does not silently swallow conflicts).

## Documentation Deliverables

- This spec: `docs/superpowers/specs/2026-06-05-settings-etag-auto-injection-design.md`.
- Implementation plan: `~/.claude/plans/playful-leaping-mango.md` (already drafted).
- **No ADR required**: the change consolidates an existing pattern; it does not introduce a
  new architectural boundary. `docs/adr/0001-web-api-decomposition-strategy.md` and friends
  describe backend architectural decisions; the ETag scoping model itself is already
  established by the existing `etag_middleware<S>` design.
- **No CONTEXT.md / ARCHITECTURE.md / AGENTS.md change**: no new domain term; "settings ETag"
  is already implicit in the backend contract.
- **No README change**: no operator-facing or developer-onboarding contract change.
- **No `docs/development/coding-standards.md` change**: the existing backend section on
  ETag/If-Match continues to describe the canonical contract; the frontend mechanism is an
  implementation detail of `lib/api.ts`.
- **JSDoc on the new exported behavior**: none required because `request()` and `requestVoid()`
  are module-internal (`async function`, not exported). The new `settingsScope` helper is also
  module-internal.

## Deferred / Out of Scope

- Automatic re-fetch on `409 if_match.stale` (would mask the cleaner `428` UX on long-idle
  tabs; defer until concurrent-edit pain is measured).
- Automatic retry of `409 if_match.stale` with re-GET (would require user prompt or merge UI).
- Generalized ETag-aware request layer for non-settings endpoints.
- Reactive store exposing the cached scope versions to UI (e.g. "settings changed elsewhere"
  banner).
- Removing the now-redundant `getCombinedSettings` GET path optimization; the combined
  endpoint stays as-is.
