# Settings ETag Auto-Injection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the five broken settings save buttons (Network, NATS, GitHub Provider, Zeroconf, Agent Certificates) by
centralizing `If-Match`/`ETag` handling inside `frontend/src/lib/api.ts::request()`. Migrate the two existing
explicit-etag helpers (`oauth.ts`, `settings.ts`) onto the same auto-cache mechanism so the frontend has one pattern.

**Spec:** [`docs/superpowers/specs/2026-06-05-settings-etag-auto-injection-design.md`](../specs/2026-06-05-settings-etag-auto-injection-design.md)

**Architecture:** Two module-scoped pieces in `lib/api.ts` — a `Record<SettingsScope, string | null>` cache keyed on
two URL prefixes (`/global-settings/` and `/settings/`), plus a `subClaim()` JWT decoder. `request()` auto-attaches
`If-Match` on `PUT`/`PATCH` when scope matches and the caller did not supply their own header; auto-updates the
cache from successful response `ETag`. Cross-user pollution mitigated via a subscriber-callback pattern in
`token-store.svelte.ts` — `onTokenChange(cb)` returns an unsubscribe handle; `api.ts` registers a listener that
diffs JWT `sub` claims (not token strings) so silent refreshes do not wipe the cache. No backend change. No new
dependencies.

**Tech Stack:** TypeScript (strict), Svelte 5 (runes), SvelteKit, Vitest, native `fetch`.

**Snapshot rules in scope:**

- `prettier: useTabs, singleQuote, trailingComma=none, printWidth=120` | `frontend/.prettierrc`
- `eslint: flat config, @typescript-eslint strict, no-unused-vars argsIgnorePattern=^_` | `frontend/eslint.config.js`
- `typescript: strict=true, checkJs=true` | `frontend/tsconfig.json`
- `markdownlint: line_length=150` | `.markdownlint.json`
- `New logic covered by success+failure path tests` | `AGENTS.md`
- `Commit messages: Conventional Commits` | `docs/development/commit-messages.md`

---

## Files

| Action | Path                                                  | What changes                                                                                                                                                                                                                                              |
| ------ | ----------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Modify | `frontend/src/lib/api.ts`                             | Add `settingsEtagCache` Record, `settingsScope()`, `subClaim()`, `withHeader()`; promote `request()` to `export`; augment `request()`; register `onTokenChange` listener; export `_resetSettingsEtagCacheForTests()`                                      |
| Modify | `frontend/src/lib/token-store.svelte.ts`              | Add `onTokenChange(cb)` registration via `Set<TokenChangeListener>`; have `setAccessToken()` invoke listeners                                                                                                                                             |
| Modify | `frontend/src/lib/api/oauth.ts`                       | Drop `OAuthSettingsWithEtag` type, drop `etag` param, route through `request()`                                                                                                                                                                           |
| Modify | `frontend/src/lib/api/settings.ts`                    | Drop `{data, etag}` envelope, drop `etag` param, route through `request()`                                                                                                                                                                                |
| Modify | `frontend/src/lib/types.ts`                           | Delete `AccessSettingsWithEtag` interface                                                                                                                                                                                                                 |
| Modify | `frontend/src/routes/settings/AccessSettings.svelte`  | Drop `etag` `$state`, use data return value directly                                                                                                                                                                                                      |
| Modify | `frontend/src/routes/settings/McpAccessTab.svelte`    | Same                                                                                                                                                                                                                                                      |
| Modify | `frontend/src/routes/settings/AccessSettings.test.ts` | Update mock return shape (plain data, not envelope)                                                                                                                                                                                                       |
| Modify | `frontend/src/routes/settings/surface-tabs.test.ts`   | Same                                                                                                                                                                                                                                                      |
| Add    | `frontend/src/lib/api.etag.test.ts`                   | New file: cache-behavior tests (success / bypass / failure / sub-change-reset / refresh-no-op); separate from the existing `api.test.ts` which top-level-mocks `token-store.svelte` via `vi.mock()`, making the real `onTokenChange` listener unreachable |

---

### Task 1: Add `onTokenChange` subscriber API to `token-store.svelte.ts`

**Files:**

- Modify: `frontend/src/lib/token-store.svelte.ts`

**Snapshot rules:** typescript strict; prettier (tabs, singleQuote, no trailing comma, printWidth=120).

- [ ] **Step 1: Add listener registration and invoke on `setAccessToken`**

  Edit `frontend/src/lib/token-store.svelte.ts`. Keep the existing file header comment (the
  `Dependency graph` arrows). Add after the existing `getAccessToken`/`setAccessToken`:

  ```typescript
  type TokenChangeListener = (prev: string | null, next: string | null) => void;
  const tokenChangeListeners: Set<TokenChangeListener> = new Set();

  /** Register a listener invoked synchronously after every `setAccessToken` call.
   *  Returns an unsubscribe handle. Safe under HMR — duplicate registration of the
   *  same callback identity is deduplicated by the underlying `Set`. */
  export function onTokenChange(cb: TokenChangeListener): () => void {
  	tokenChangeListeners.add(cb);
  	return () => {
  		tokenChangeListeners.delete(cb);
  	};
  }
  ```

  Change `setAccessToken` to capture `prev` and invoke listeners after assignment:

  ```typescript
  export function setAccessToken(token: string | null): void {
  	const prev = accessToken;
  	accessToken = token;
  	for (const cb of tokenChangeListeners) cb(prev, token);
  }
  ```

  Idiomatic patterns: module-scoped `Set` for listener storage (cheap dedup under HMR);
  unsubscribe-by-return-handle is the canonical subscribe shape in JS/TS event APIs. Do not
  introduce a class or RxJS-style observable — overkill for this case.

---

### Task 2: Add ETag cache + scope + JWT decoder to `lib/api.ts`

**Files:**

- Modify: `frontend/src/lib/api.ts`

**Snapshot rules:** typescript strict, prettier (tabs/singleQuote/no trailing comma).

- [ ] **Step 1: Add types, cache, and helpers near top of file (after the `BASE`/`DEFAULT_TIMEOUT_MS` constants)**

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

  function subClaim(token: string | null): string | null {
  	if (!token) return null;
  	const parts = token.split('.');
  	if (parts.length < 2) return null;
  	try {
  		let b64 = parts[1].replace(/-/g, '+').replace(/_/g, '/');
  		b64 += '='.repeat((4 - (b64.length % 4)) % 4);
  		const payload = JSON.parse(atob(b64));
  		return typeof payload.sub === 'string' ? payload.sub : null;
  	} catch {
  		return null;
  	}
  }
  ```

  Idiomatic pattern: `Record<Union, T>` over a small finite-union key is more idiomatic than
  `Map` — exhaustive under `strict`, no `.get()`/`.set()` noise. The base64url-padding
  restore (`(4 - len % 4) % 4`) is the standard JWT-decoder idiom; do NOT add a `jose` or
  `jwt-decode` dependency — the decoder is ~10 lines and the project does not need
  signature verification client-side.

- [ ] **Step 2: Export `request<T>()` and add a header-merge helper**

  Two prerequisite edits before the augmentation in step 3:
  1. **Promote `request` to an export** (currently `async function request<T>` at
     ~line 323): change to `export async function request<T>`. This is required by
     Tasks 4 and 5, which import `request` from `$lib/api`. Do this in the same
     keystroke as step 3 to avoid leaving the file in a half-migrated state.
  2. **Add a `Headers`-aware merge helper** at module scope, near the other private
     helpers. The naïve object-spread used by `authenticatedFetch` today silently drops
     entries when `options.headers` is a `Headers` instance — do not propagate that bug
     into the new code path:

  ```typescript
  function withHeader(init: HeadersInit | undefined, name: string, value: string): Headers {
  	const h = new Headers(init);
  	h.set(name, value);
  	return h;
  }
  ```

  Also add the test-only reset hook (leading underscore is a conventional signal that
  this export is not part of the public API; it will not trigger `no-unused-vars` because
  it is exported and therefore referenced):

  ```typescript
  /** Test-only: clears the scope ETag cache. Do not call from production code. */
  export function _resetSettingsEtagCacheForTests(): void {
  	settingsEtagCache.global = null;
  	settingsEtagCache.tenant = null;
  }
  ```

- [ ] **Step 3: Augment `request<T>()` to attach `If-Match` and capture `ETag`**

  Modify the existing `request<T>()` body (around line 323). Before `authenticatedFetch`,
  detect scope and inject `If-Match` via the `withHeader` helper:

  ```typescript
  export async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
  	const scope = settingsScope(path);
  	const method = (options.method ?? 'GET').toUpperCase();

  	if (scope !== null && (method === 'PUT' || method === 'PATCH')) {
  		const callerHas = new Headers(options.headers ?? {}).has('if-match');
  		const cached = settingsEtagCache[scope];
  		if (!callerHas && cached !== null) {
  			options = {
  				...options,
  				headers: withHeader(options.headers, 'if-match', cached)
  			};
  		}
  	}

  	let res: Response;
  	try {
  		res = await authenticatedFetch(`${BASE}${path}`, options);
  	} catch (err) {
  		// ...existing error mapping unchanged...
  	}

  	if (scope !== null && res.ok) {
  		const etag = res.headers.get('etag');
  		if (etag !== null) settingsEtagCache[scope] = etag;
  	}

  	if (!res.ok) {
  		throw await extractApiError(res);
  	}
  	return res.json();
  }
  ```

  **Critical:** the cache-write after `res.ok` is unconditional — apply it whether or not
  the caller supplied their own `If-Match`. Do NOT add an `if (!callerHas)` guard around
  the write; the bypass flag only suppresses _injection_, never _storage_.

  `requestVoid()` is **not** modified. Its only `/settings/*` caller is a DELETE outside
  `etag_middleware` and DELETE semantics do not fit If-Match.

  Idiomatic note: `new Headers(init)` accepts all three `HeadersInit` shapes (`Headers`,
  `[string,string][]`, `Record<string,string>`) and returns a `Headers` instance — never
  cast `options.headers` to `Record<string, string>`; that drops entries when the caller
  passed `Headers`.

- [ ] **Step 4: Register `onTokenChange` listener at module init**

  Import `onTokenChange` from `./token-store.svelte` (one-way arrow preserved — `api.ts`
  already imports from `token-store`). Register near the bottom of the existing imports/
  initialisation block, before any exports that depend on the cache:

  ```typescript
  import { getAccessToken, onTokenChange, setAccessToken, setSessionExpired } from './token-store.svelte';

  onTokenChange((prev, next) => {
  	if (subClaim(prev) !== subClaim(next)) {
  		settingsEtagCache.global = null;
  		settingsEtagCache.tenant = null;
  	}
  });
  ```

  Idiomatic pattern: subscriber callback preserves the existing one-way dependency arrow
  (`api.ts → token-store`) that the file header on `token-store.svelte.ts` explicitly
  documents. Do NOT have `token-store` import from `api.ts` — that re-creates the cycle the
  file was extracted to break.

---

### Task 3: Cache-behavior tests in `frontend/src/lib/api.etag.test.ts`

**Files:**

- Add: `frontend/src/lib/api.etag.test.ts`

**Snapshot rules:** AGENTS.md "New logic covered by success+failure path tests"; typescript strict; prettier.

**Why a separate file (not `api.test.ts`):** The existing `frontend/src/lib/api.test.ts`
contains a top-level `vi.mock('./token-store.svelte', () => ({ setAccessToken: vi.fn(), ... }))`
that Vitest hoists to module scope. This replaces the real `setAccessToken` with a no-op
`vi.fn()`, so the `onTokenChange` listener registered at `api.ts` module-init never fires
when tests call `setAccessToken(...)`. Cases 6–9 (sub-change-reset, silent-refresh,
malformed JWT, base64url) would pass vacuously without this isolation. Using a separate
file avoids restructuring the existing test suite and keeps the mock scope contained.

- [ ] **Step 1: Add the file with Vitest setup**

  Use `vi.stubGlobal('fetch', ...)` per Vitest idiom. The test file **must import at
  least one symbol from `$lib/api`** so the `onTokenChange` listener registered at
  `api.ts` module-init runs before any test executes (otherwise the sub-change-reset
  cases pass vacuously). Use the exported `_resetSettingsEtagCacheForTests` for
  isolation in `beforeEach` — this is more robust than driving the cache through the
  production listener path, and the dedicated sub-change-reset case below still proves
  the listener is wired.

  Required cases (each is one `it()` block):
  1. **Success path — auto-inject `If-Match` on PUT after GET**: stub `fetch` to return
     `200 OK` with `ETag: "v1"` on `GET /api/v1/global-settings/network`; call
     `getNetworkSettings()`. Then stub a subsequent `PUT /api/v1/global-settings/network`
     and call `updateNetworkSettings({...})`. Assert the outgoing `Request.headers` (or
     captured second `fetch` call's `init.headers`) contains `if-match: v1`.
  2. **Bypass path — caller-supplied `If-Match` honored verbatim**: call `request()`
     directly with `headers: { 'if-match': 'custom' }`. Assert outgoing header equals
     `custom`, not the cached value.
  3. **Bypass path — caller passed `Headers` instance (not plain object)**: same as
     case 2 but with `headers: new Headers({ 'if-match': 'custom' })`. Asserts the
     `withHeader` helper preserves caller entries from all three `HeadersInit` shapes.
  4. **Failure path — non-OK response leaves cache untouched**: prime cache with `"v1"`
     via a successful GET; stub PUT to return `428`. Assert cache slot still equals `"v1"`
     after the failure.
  5. **Cross-scope isolation**: GET `/api/v1/settings/access` returns `ETag: "t1"`; GET
     `/api/v1/global-settings/network` returns `ETag: "g1"`. Assert subsequent PUT to
     `/global-settings/network` carries `g1` (not `t1`).
  6. **Sub-change reset (proves listener wiring)**: prime both cache slots via GETs. Set
     access token to a JWT with `sub: "user-a"`. Set access token to a JWT with
     `sub: "user-b"`. Assert both cache slots are now `null`.
  7. **Silent refresh no-op**: prime cache. Set access token to a JWT with
     `sub: "user-a"`. Set access token to a _different_ JWT string but with
     `sub: "user-a"` (simulating refresh). Assert cache slots are unchanged.
  8. **Malformed JWT**: set access token to `"not.a.jwt"`. Assert no exception thrown;
     cache unchanged (both sides decode to `null`, equal, no reset).
  9. **base64url decode path — non-vacuous**: hand-craft TWO base64url JWTs whose
     middle segments contain `-`/`_` characters (use `makeJwtBase64Url(sub, pad)` with a
     padding string that forces `+`/`/` in the raw base64, then map to `-`/`_`). First
     token has `sub: "user-a"`, second has `sub: "user-b"`. Prime the cache, then set
     access token to JWT-A, then to JWT-B. **Assert both cache slots are `null`** (i.e.
     the listener fired and reset the cache).

     A symmetric "same `sub`, cache unchanged" assertion is rejected: if the decoder
     regression skipped the char-swap, both JWTs would decode to `null === null`, no
     reset, "cache unchanged" — passing vacuously. The "different `sub`" assertion fails
     loudly when the swap breaks (broken decoder → both decode to `null` → no reset →
     cache NOT null → assertion fails). Cases 6/7 use `btoa` output which never contains
     `-`/`_`, so this case is the only one covering the swap.

  Test file shape (skeleton):

  ```typescript
  import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
  import { _resetSettingsEtagCacheForTests, request } from '$lib/api';
  // Import the REAL setAccessToken — do NOT vi.mock('./token-store.svelte') in this file.
  // The existing api.test.ts already uses a top-level vi.mock there; adding the same mock
  // here would prevent the onTokenChange listener (registered in api.ts at module init)
  // from ever firing, making cases 6–9 vacuous.
  import { setAccessToken } from '$lib/token-store.svelte';

  function makeJwt(sub: string, salt = ''): string {
  	const header = btoa(JSON.stringify({ alg: 'none' })).replace(/=+$/, '');
  	const payload = btoa(JSON.stringify({ sub, salt })).replace(/=+$/, '');
  	return `${header}.${payload}.sig`;
  }

  function makeJwtBase64Url(sub: string): string {
  	const header = btoa(JSON.stringify({ alg: 'none' })).replace(/=+$/, '');
  	// Force at least one '+' or '/' in the base64 output, then map to base64url.
  	const payload = btoa(JSON.stringify({ sub, pad: 'ÿÿ' }))
  		.replace(/=+$/, '')
  		.replace(/\+/g, '-')
  		.replace(/\//g, '_');
  	return `${header}.${payload}.sig`;
  }

  describe('settings ETag auto-injection', () => {
  	beforeEach(() => {
  		_resetSettingsEtagCacheForTests();
  		setAccessToken(null);
  		vi.restoreAllMocks();
  	});
  	afterEach(() => {
  		setAccessToken(null);
  	});

  	// ...nine cases above...
  });
  ```

  Idiomatic patterns: Vitest `vi.stubGlobal` for `fetch`; `vi.fn()` returning a `Response`
  built with the standard `new Response(JSON.stringify(...), { headers: { etag: 'v1' } })`
  constructor; no custom mock harness.

---

### Task 4: Migrate `oauth.ts` off explicit envelope

**Files:**

- Modify: `frontend/src/lib/api/oauth.ts`

- [ ] **Step 1: Drop `OAuthSettingsWithEtag` and the `etag` param**
  - Delete the `OAuthSettingsWithEtag` interface (around line 156).
  - Change `getOAuthSettings` return type to `Promise<OAuthSettingsResponse>`.
  - Change `updateOAuthSettings(body, etag)` signature to `updateOAuthSettings(body)`,
    return `Promise<OAuthSettingsResponse>`.
  - Replace the `authenticatedFetch` direct calls (lines 161–207) with calls to the
    generic `request()` helper from `../api`. The auto-cache handles `If-Match` and `ETag`
    transparently:

  ```typescript
  import { request } from '$lib/api';

  export function getOAuthSettings(): Promise<OAuthSettingsResponse> {
  	return request('/global-settings/oauth');
  }

  export function updateOAuthSettings(body: UpdateOAuthSettingsRequest): Promise<OAuthSettingsResponse> {
  	return request('/global-settings/oauth', {
  		method: 'PUT',
  		body: JSON.stringify(body)
  	});
  }
  ```

  Note: `request` is currently `async function` (not exported) — promote it to
  `export async function` in `lib/api.ts` as part of Task 2 step 2 if not already exported.

---

### Task 5: Migrate `settings.ts` off explicit envelope; delete `AccessSettingsWithEtag`

**Files:**

- Modify: `frontend/src/lib/api/settings.ts`
- Modify: `frontend/src/lib/types.ts`

- [ ] **Step 1: Rewrite `getAccessSettings` and `updateAccessSettings` to use `request()`**

  ```typescript
  import { request } from '$lib/api';
  import type { AccessSettingsData, UpdateAccessSettingsRequest } from '$lib/types';

  export function getAccessSettings(): Promise<AccessSettingsData> {
  	return request('/settings/access');
  }

  export function updateAccessSettings(body: UpdateAccessSettingsRequest): Promise<AccessSettingsData> {
  	return request('/settings/access', {
  		method: 'PUT',
  		body: JSON.stringify(body)
  	});
  }
  ```

- [ ] **Step 2: Remove `AccessSettingsWithEtag` from `lib/types.ts` (line 166)**

  Delete the interface definition. If no other type re-exports it, no other file needs to
  change.

---

### Task 6: Update callers — `AccessSettings.svelte`, `McpAccessTab.svelte`

**Files:**

- Modify: `frontend/src/routes/settings/AccessSettings.svelte`
- Modify: `frontend/src/routes/settings/McpAccessTab.svelte`

- [ ] **Step 1: `AccessSettings.svelte` — drop the destructure and `etag` state**

  Locate the `let etag = $state<string | null>(null)` declaration and the two destructure
  sites (load on line ~54, save on line ~82). Remove the `etag` state entirely. Change:

  ```typescript
  // before
  const { data, etag: e } = await getAccessSettings();
  etag = e;
  // after
  const data = await getAccessSettings();
  ```

  And:

  ```typescript
  // before
  const { data, etag: newEtag } = await updateAccessSettings(body, etag);
  etag = newEtag;
  // after
  const data = await updateAccessSettings(body);
  ```

- [ ] **Step 2: `McpAccessTab.svelte` — same simplification**

  Locate `oauthSettingsEtag` `$state` (and its assignments around lines 149 and 166).
  Remove the state and drop the second argument from `updateOAuthSettings(...)`.

---

### Task 7: Update test mocks to return plain data

**Files:**

- Modify: `frontend/src/routes/settings/AccessSettings.test.ts`
- Modify: `frontend/src/routes/settings/surface-tabs.test.ts`

- [ ] **Step 1: `AccessSettings.test.ts` — flatten mock returns**

  Change the `vi.fn(async () => ({ data: {...}, etag: '...' }))` style to
  `vi.fn(async () => ({...}))` returning the bare `AccessSettingsData` shape.

- [ ] **Step 2: `surface-tabs.test.ts` — same change for the single mock at line 41**

  Mirror the shape used in `AccessSettings.test.ts`.

---

### Task 8: Frontend quality gates

**Files:**

- (no edits — verification only)

**Snapshot rules:** prettier, eslint, typescript strict, AGENTS.md "Document all code changes".

- [ ] **Step 1: Run the full frontend gate chain**

  ```bash
  cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
  ```

  All steps must pass. If `format:check` fails, run `npm run format` (defined in
  `frontend/package.json` as `prettier --write .` — uses the pinned local binary and
  picks up `frontend/.prettierrc`) and re-run. Do NOT add `// prettier-ignore`,
  `// eslint-disable`, or `// @ts-ignore` directives to silence the gate.

- [ ] **Step 2: Backend sanity (no backend change, but run as guard)**

  ```bash
  cargo test -p uptrakit-web-api --features db-sqlite if_match
  ```

  Confirms the backend `etag_middleware` contract this plan depends on remains intact.

---

### Task 9: Manual verification in dev server

**Files:**

- (no edits — verification only)

- [ ] **Step 1: Golden-path save on every previously-broken panel**

  Start dev server (`cd frontend && npm run dev` or via the project's launch script). Log
  in as a user with `ManageGlobalSettings` and tenant-management permissions. Open the
  Settings page and exercise each save button:
  - Global Settings tab → Network → change a Trusted Proxy entry → Save → success toast.
  - Global Settings tab → NATS → enter URL → Save; then Clear → success toast on both.
  - Global Settings tab → GitHub Provider → change API base URL → Save → success toast.
  - Global Settings tab → Zeroconf → toggle Enabled → Save → success toast.
  - General Settings tab → Agent Certificates → change lifetime → Save → success toast.

  Each must succeed with no `428` or `409` toast.

- [ ] **Step 2: Regression check on already-working panels**
  - General Settings tab → Access Settings → Save → success toast.
  - MCP Access tab → OAuth Settings → Save → success toast.

- [ ] **Step 3: Two-tab conflict regression**

  Open the Network panel in two browser tabs (A and B). Save in tab A (success). Without
  reloading, save the same panel in tab B with a different value. Tab B must show the
  existing `etag mismatch (stale version)` error toast (the `409 if_match.stale` code path).
  Confirms the cache-update path does not silently swallow conflicts.

- [ ] **Step 4: Cross-user regression**

  Log out, log back in as a different tenant's user, navigate to settings, save a panel
  immediately. Must succeed (no `409`) — proves `onTokenChange` listener cleared the cache.

---

## Documentation Deliverables

Spec explicitly declared **no project-doc impact** beyond the spec and this plan:

- No `README.md` update — no operator-facing or onboarding contract change.
- No `CONTEXT.md` / `ARCHITECTURE.md` / `AGENTS.md` change — no new domain term, no new
  architectural boundary (mechanism mirrors the existing backend `etag_middleware<S>`
  contract).
- No new ADR — change consolidates an existing pattern. `docs/superpowers/specs/2026-05-24-etag-middleware-design.md`
  remains the authoritative ETag design doc; this plan implements its frontend complement.
- No `docs/api/wire-protocol.md` or `asyncapi.yaml` change — no wire change.
- No JSDoc on `request()`/`requestVoid()` — both remain module-internal; the public-facing
  helpers (`getOAuthSettings`, `getNetworkSettings`, etc.) keep their existing names and
  signatures (just stripped of the `etag` parameter).

If implementation reveals any externally-observable contract change (e.g. a previously
documented `etag` parameter on a public API), surface it during the verify pass and add a
follow-up doc task.

---

## Commit Strategy

Conventional Commits per `docs/development/commit-messages.md`.

**Atomic landing — Tasks 2, 4, 5 are one logical unit.** Task 2 exports `request` from
`lib/api.ts`; Tasks 4 and 5 import it. Splitting them across separate commits leaves an
intermediate commit that fails `npm run check` (TS2305: no exported member 'request').
Land them together in a single commit (or as a stacked series rebased into one) — do
not push the partial state.

Tasks 1 (token-store API) and 3 (cache-behavior tests in the new `api.etag.test.ts`)
have no external import dependency on Task 2's `export` change beyond
`_resetSettingsEtagCacheForTests`, so they could land in their own commits. Bundling all
of Tasks 1–7 into a single focused commit is simpler and matches the spec's "one
mechanism" framing.

Suggested commit message:

```text
fix(frontend): auto-attach If-Match on settings updates

Centralizes ETag handling in lib/api.ts request() via a scope-keyed
record and a token-store onTokenChange listener that diffs JWT sub
claims. Restores Network, NATS, GitHub Provider, Zeroconf, and Agent
Certificates save buttons.
```

**Idempotency note for Tasks 4–5:** the type deletions (`OAuthSettingsWithEtag`,
`AccessSettingsWithEtag`) and helper rewrites are one-shot — re-running them after a
successful first execution fails when the interfaces are already absent. If executing
via an automated agent that may retry, guard each step with `grep -q '<symbol>' <file>`
or mark "skip if absent". For human execution, simply do not re-run.

Do **not** use `--no-verify` or bypass the markdownlint/prettier pre-commit hooks. If
either fails, fix the root cause (re-run `npm run format`, fix the offending markdown),
do not suppress.

---

## Out of Scope (carry-forward from spec)

- Automatic re-fetch on `409 if_match.stale`.
- Generalized ETag layer for non-settings endpoints.
- Reactive store exposing scope versions to UI.
- Removing the redundant `getCombinedSettings` GET path optimization.
- Dedicated Playwright spec `frontend/tests/e2e/settings-etag.spec.ts` covering
  Task 9's golden-path saves. The vitest unit suite already covers the cache mechanism;
  an e2e spec would be net-positive but is out of scope for this fix — track as a
  follow-up if manual verification becomes burdensome.
