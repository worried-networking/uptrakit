# Frontend Code Review

**Date:** 2026-02-17
**Scope:** Complete frontend codebase (`frontend/`)
**Branch:** `refactor/codereview-frontend`
**Reviewer:** AI (Claude Opus 4.6)

## Table of Contents

- [Executive Summary](#executive-summary)
- [1. Architecture](#1-architecture)
- [2. Security and Safety](#2-security-and-safety)
- [3. Code Quality and Coding Standards](#3-code-quality-and-coding-standards)
- [4. Backend High Availability Considerations](#4-backend-high-availability-considerations)
- [5. Testing](#5-testing)
- [6. Accessibility](#6-accessibility)
- [7. Summary Tables](#7-summary-tables)
- [8. Recommendations](#8-recommendations)

---

## Executive Summary

The frontend is a SvelteKit SPA (client-side only, static adapter) serving as the admin UI for the Uptrakit
controller. The codebase is compact (~4,000 lines across ~24 source files), well-organized, and demonstrates
good security awareness. Key strengths include in-memory token storage, Content Security Policy, safe redirect
validation, and a well-structured API client with token refresh deduplication.

The review identifies issues across six categories. The most significant findings are:

- **Security:** CSP uses `'unsafe-inline'` for scripts, no URL path parameter validation, OIDC redirect
  validation is protocol-only.
- **Code quality:** Mixed Svelte 4/5 state management patterns, non-null assertions for component refs, a
  double-load bug on the services page, significant code duplication in enrollment token settings, and no
  linter configuration.
- **Backend HA:** No jitter in MQTT polling backoff (thundering herd risk), no stale data detection, no
  offline action prevention, and no retry mechanism on non-settings pages.
- **Testing:** Only 3 unit tests in a single file. No tests for the API client, route components, settings
  components, or utility functions. No integration or E2E tests.

---

## 1. Architecture

### 1.1 Positive Findings

| Finding | Location |
|---------|----------|
| Clean SPA architecture with SvelteKit static adapter; CSR-only is appropriate for an admin panel behind auth | `svelte.config.js`, `+layout.ts` |
| Good separation of concerns: `lib/` for shared code, `routes/` for pages, `components/` for reusable UI | `src/` tree |
| Centralized API client with type-safe functions and consistent request/response patterns | `lib/api.ts` |
| Centralized auth guard in root layout prevents unauthorized access to protected routes | `+layout.svelte:48-57` |
| Permission-based navigation filtering using derived state | `+layout.svelte:67-80` |
| In-memory access token (never persisted to localStorage) | `lib/auth.ts:9` |
| Token refresh deduplication prevents redundant concurrent refresh requests | `lib/api.ts:64,122-128` |
| Settings components use `Promise.allSettled` for parallel loading with individual error handling | `settings/+page.svelte:113-151` |

### 1.2 Issues

#### ARC-01: Mixed Svelte 4 / Svelte 5 state management patterns

**Severity:** Low
**Location:** `lib/auth.ts`, `lib/theme.ts`, `lib/stores/network.ts` (Svelte 4 `writable()`) vs
`lib/notifications.svelte.ts` and all page components (Svelte 5 `$state()`)

The codebase uses Svelte 4 stores (`writable()`) for auth, theme, and network status, while using Svelte 5
runes (`$state()`, `$derived()`, `$effect()`) everywhere else. This inconsistency adds cognitive load.

**Recommendation:** Migrate `auth.ts`, `theme.ts`, and `stores/network.ts` to Svelte 5 runes for
consistency. Alternatively, document the intentional choice (e.g., stores for cross-component reactivity vs
runes for component-local state).

#### ARC-02: No linter or formatter configuration

**Severity:** Medium
**Location:** Project root (`frontend/`)

No ESLint, Prettier, or equivalent configuration exists. The CI runs `svelte-check` and `vite build` but has
no linting step. This means style inconsistencies, unused imports, and potential bugs go undetected.

**Recommendation:** Add ESLint with `eslint-plugin-svelte` and Prettier. Add a CI lint step.

#### ARC-03: Settings components use imperative API via `export function`

**Severity:** Low
**Location:** `settings/RegistrationSettings.svelte`, `AuthenticationSettings.svelte`,
`AgentCertificateSettings.svelte`, `EnrollmentTokenSettings.svelte`, `MqttClientsSettings.svelte`,
`OidcProvidersSettings.svelte`

Settings components expose `load()` methods called imperatively from the parent via `bind:this`. The parent
stores component refs with `$state(undefined!)`, which is a TypeScript escape hatch. This pattern is unusual
for Svelte and makes the data flow harder to trace.

**Recommendation:** Consider passing loaded data as props instead, with the parent holding the loaded state
and passing it down. This eliminates the need for `bind:this`, the `undefined!` assertions, and the `refsReady`
coordination logic.

#### ARC-04: Single API base path with no configuration

**Severity:** Low
**Location:** `lib/api.ts:41`

```typescript
const BASE = '/api/v1';
```

The API base path is hardcoded. In deployments where the frontend is served from a different origin or sub-path
than the API (e.g., split deployment with a CDN), this would require a code change.

**Recommendation:** Read from an environment variable or a runtime configuration endpoint.

---

## 2. Security and Safety

### 2.1 Positive Findings

| Finding | Location |
|---------|----------|
| Content Security Policy header in `app.html` restricts default-src, script-src, style-src, img-src, connect-src, font-src, object-src, base-uri, and form-action | `app.html:9` |
| In-memory access token never touches localStorage or sessionStorage | `lib/auth.ts:9` |
| Refresh token is HttpOnly + Secure + SameSite=Strict (backend-enforced) | Backend `refresh_cookie.rs` |
| OIDC authorize URL validated as HTTPS before redirect | `lib/auth.ts:63-65` |
| Logo URLs validated as HTTPS via `isValidLogoUrl()` using `URL()` constructor (rejects `javascript:`, `data:`, etc.) | `lib/utils.ts:6-14` |
| Redirect parameter validated: must start with `/` and not `//` (prevents open redirect) | `login/+page.svelte:89-95` |
| `credentials: 'same-origin'` on all fetch calls (refresh token cookie only sent to same origin) | `lib/api.ts:85,108` |
| Device code validated with strict regex before use | `device/+page.svelte:10` |
| `referrerpolicy="no-referrer"` on external OIDC provider images | `login/+page.svelte:257` |
| `encodeURIComponent` used for all URL parameters | Multiple locations |
| `RefreshError` class distinguishes 4xx (real auth failure) from 5xx (transient) to avoid unnecessary logouts | `lib/api.ts:71-79` |

### 2.2 Issues

#### SEC-01: CSP allows `'unsafe-inline'` for scripts

**Severity:** Medium
**Location:** `app.html:9`

```html
<meta http-equiv="Content-Security-Policy"
  content="... script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; ..." />
```

The `'unsafe-inline'` for `script-src` is needed for the theme FOUC-prevention inline script (`app.html:12-18`),
but it significantly weakens XSS protection. Any XSS vulnerability can execute arbitrary inline scripts.

The `'unsafe-inline'` for `style-src` is likely needed by Skeleton UI / Tailwind but should be verified.

**Recommendation:** Replace the inline theme script with a CSP nonce (SvelteKit supports `%sveltekit.nonce%`)
or use a `'sha256-...'` hash for the specific inline script. Investigate whether `style-src` can drop
`'unsafe-inline'`.

#### SEC-02: No URL path parameter validation

**Severity:** Low
**Location:** `lib/api.ts` (multiple functions)

API functions directly interpolate string parameters into URL paths:

```typescript
export function approveService(id: string) { return request(`/services/${id}/approve`, ...); }
export function getOidcAuthorizeUrl(providerId: string) { return request(`/auth/oidc/${providerId}/authorize`); }
```

All `id` values originate from server responses (UUIDs), so exploitation requires a compromised server response.
However, there is no frontend validation that these are valid UUIDs.

**Recommendation:** Add a UUID format check before interpolating IDs into URLs, or use
`encodeURIComponent()` on all path parameters as defense-in-depth.

#### SEC-03: Error messages may expose backend internals

**Severity:** Low
**Location:** All pages (consistent pattern)

```typescript
error = e instanceof Error ? e.message : 'Failed to register software item';
```

Error messages from the backend are displayed directly to users. The backend's `extractErrorMessage()`
(`api.ts:45-57`) parses the error response body and returns it verbatim. If the backend ever returns
detailed error messages (database names, file paths, stack traces), they would be displayed.

The backend appears to sanitize error messages, but the frontend provides no second layer of protection.

**Recommendation:** Consider a maximum length for displayed error messages and/or a sanitization pass.

#### SEC-04: OIDC link token readable from URL parameters

**Severity:** Low
**Location:** `login/+page.svelte:69-74`

When OIDC account linking is required, the backend redirects to
`/login?link_required=true&link_token=...&email=...`. The `link_token` is visible in the browser address
bar and history. This is the intended OAuth flow (the backend generates these URLs), but the token in the
URL could be captured in:

- Browser history
- Referrer headers (mitigated by `referrerpolicy`)
- Server access logs

**Recommendation:** Document this as an accepted risk, or consider using a session-based approach where the
link token is stored server-side and referenced by an opaque session ID.

#### SEC-05: CSP `img-src 'self' https:` allows any HTTPS image

**Severity:** Low
**Location:** `app.html:9`

Combined with OIDC provider logo URLs (admin-configured), this allows loading images from any HTTPS domain.
While `referrerpolicy="no-referrer"` prevents URL leakage via Referer, the image request itself is made,
which could be used for tracking.

**Recommendation:** Accept as necessary for OIDC provider logos. The admin who configures the logo URL is
trusted.

---

## 3. Code Quality and Coding Standards

### 3.1 Positive Findings

| Finding | Location |
|---------|----------|
| TypeScript strict mode enabled | `tsconfig.json` |
| Consistent error handling pattern across all pages: `try/catch` with `Error` message extraction | All pages |
| Well-implemented abort signal handling with `AbortSignal.timeout()` and `AbortSignal.any()` | `lib/api.ts:102-105` |
| Clean component API with fully typed `$props()` | All components |
| `ModalBackdrop` has proper focus trapping and focus restoration | `components/ModalBackdrop.svelte` |
| `ContextMenu` has viewport-aware positioning to prevent overflow | `components/ContextMenu.svelte` |
| `Pagination` is clean and minimal | `components/Pagination.svelte` |
| Proper use of `Promise.allSettled` for parallel loads with individual error handling | `settings/+page.svelte`, `software/+page.svelte` |

### 3.2 Issues

#### CQ-01: Double data load on services page

**Severity:** Medium (bug)
**Location:** `services/+page.svelte:28-32`

```typescript
onMount(() => loadServices(1));

$effect(() => {
    loadServices(1); // Reload services when typeFilter changes
});
```

Both `onMount` and `$effect` call `loadServices(1)` on initial render. The `$effect` runs on every render
cycle when its tracked dependencies change (it tracks `typeFilter` through the closure in `loadServices`).
On mount, this results in two API calls for the same data.

**Recommendation:** Remove the `onMount` call. The `$effect` handles initial load because it runs when
`typeFilter` is first read. If you need explicit initial-only loading, use the `$effect` with explicit
dependency tracking.

#### CQ-02: Non-null assertion `$state(undefined!)` for component refs

**Severity:** Medium
**Location:** `settings/+page.svelte:23-28`

```typescript
let registrationRef: RegistrationSettings = $state(undefined!);
let authenticationRef: AuthenticationSettings = $state(undefined!);
// ... 4 more
```

Using `undefined!` bypasses TypeScript's null safety. These refs are `undefined` until the component mounts,
but the code calls methods on them (e.g., `registrationRef.load(...)`) assuming they're initialized.

The `refsReady` flag (`settings/+page.svelte:21,58-61`) mitigates this by delaying `loadAllSettings()` until
after `tick()`, but this is a fragile coordination pattern.

**Recommendation:** See ARC-03. Passing loaded data as props eliminates this issue entirely. If the current
pattern is kept, add a null check before calling load methods.

#### CQ-03: Significant code duplication in EnrollmentTokenSettings

**Severity:** Low
**Location:** `settings/EnrollmentTokenSettings.svelte`

The component has three nearly identical sets of:
- State variables: `enrollmentConfigured`/`generatedToken`, `mqttEnrollmentConfigured`/`mqttGeneratedToken`,
  `sshAgentEnrollmentConfigured`/`sshAgentGeneratedToken`
- Functions: `handleGenerateToken`/`handleRevokeToken` (repeated 3 times with only the `type` parameter
  differing)
- Template blocks (repeated 3 times, lines 107-245)

**Recommendation:** Extract a reusable `TokenSection` component or use a data-driven approach with an array
of `{ type, label, description }` objects.

#### CQ-04: Fragile `$effect` dependency tracking in services page

**Severity:** Low
**Location:** `services/+page.svelte:30-32`

```typescript
$effect(() => {
    loadServices(1); // Reload services when typeFilter changes
});
```

The comment says "when typeFilter changes" but `typeFilter` is accessed inside `loadServices()` (line 39),
not directly in the `$effect` body. Svelte 5's fine-grained reactivity tracks `typeFilter` through the
closure, so this works. However, if `loadServices` is refactored to accept the filter as a parameter, the
`$effect` would stop re-running on filter changes.

**Recommendation:** Access `typeFilter` directly in the `$effect` body to make the dependency explicit:

```typescript
$effect(() => {
    const _filter = typeFilter; // explicit dependency
    loadServices(1);
});
```

#### CQ-05: Network store event listeners never cleaned up

**Severity:** Low
**Location:** `lib/stores/network.ts:12-13`

```typescript
window.addEventListener('online', updateOnlineStatus);
window.addEventListener('offline', updateOnlineStatus);
```

Event listeners are added at module evaluation time but never removed. As a module singleton, this doesn't
cause a memory leak, but it's not proper resource management.

**Recommendation:** Since this is a module-level singleton that lives for the app lifetime, document this
as intentional. Alternatively, provide a cleanup function for testing.

#### CQ-06: Theme media query listener never cleaned up

**Severity:** Low
**Location:** `lib/theme.ts:32-36`

```typescript
const mq = window.matchMedia('(prefers-color-scheme: dark)');
mq.addEventListener('change', () => {
    const current = getStored();
    if (current === 'system') applyTheme('system');
});
```

Similar to CQ-05 — the listener lives for the app lifetime, which is appropriate, but undocumented.

#### CQ-07: Svelte `$effect` for auth redirect on login/register pages

**Severity:** Low
**Location:** `login/+page.svelte:24-28`, `register/+page.svelte:13-17`

```typescript
$effect(() => {
    if ($user) {
        goto('/');
    }
});
```

This `$effect` runs on every reactive change to `$user`. If the user somehow flickers (e.g., during
refresh token rotation), it could cause an unintended redirect. In practice this is unlikely, but the
pattern is fragile.

**Recommendation:** Use a one-shot redirect check rather than a reactive effect, or add a flag to prevent
re-triggering.

#### CQ-08: Inconsistent `has_password` indicator for MQTT client secret handling

**Severity:** Low (cosmetic)
**Location:** `settings/MqttClientsSettings.svelte:279-281`

The "Password set" badge for MQTT client editing uses `editingMqttClient?.has_password`, which mirrors the
pattern used for OIDC client secret (`editingProvider`). This is consistent, but the OIDC provider form uses
`has_client_secret` from the response type, while the MQTT form checks `has_password`. Both are correct but
use different naming conventions due to the underlying backend types.

---

## 4. Backend High Availability Considerations

### 4.1 Positive Findings

| Finding | Location |
|---------|----------|
| Token refresh deduplication prevents concurrent refresh storms | `lib/api.ts:122-128` |
| `RefreshError` class separates 4xx (auth) from 5xx (transient) — avoids forced logout on server errors | `lib/api.ts:71-79,159-166` |
| MQTT polling uses exponential backoff (10s base, 5min cap) | `settings/+page.svelte:67-93` |
| `Promise.allSettled` for settings loading — partial failure doesn't block other sections | `settings/+page.svelte:113-151` |
| Individual "Retry All" buttons per settings section | `settings/+page.svelte:166-206` |
| Polling stopped on component destroy via `onDestroy` | `settings/+page.svelte:63-65` |

### 4.2 Issues

#### HA-01: No jitter in MQTT polling backoff

**Severity:** Medium
**Location:** `settings/+page.svelte:79-83`

```typescript
const delay = Math.min(
    initialMqttPollDelay * Math.pow(2, mqttPollAttempt - 1),
    maxMqttPollDelay
);
```

All clients use deterministic exponential backoff. If multiple browser tabs or users are open, they will
all retry at the same time after the same backoff period, creating a thundering herd on the backend.

**Recommendation:** Add random jitter:

```typescript
const baseDelay = Math.min(
    initialMqttPollDelay * Math.pow(2, mqttPollAttempt - 1),
    maxMqttPollDelay
);
const delay = baseDelay * (0.5 + Math.random() * 0.5); // 50-100% of base delay
```

#### HA-02: No stale data detection

**Severity:** Medium
**Location:** All data-loading pages

Pages load data once on mount and only refresh on explicit user action (save, page navigation). If another
admin changes settings, or if the backend state changes (e.g., a service becomes approved), the current
client won't know until it navigates away and back.

**Recommendation:** For settings, add a periodic refresh or use `ETag`/`If-Modified-Since` headers.
For entity lists (services, hosts, software), consider a `Last-Modified` header on GET responses and
periodic re-fetching with conflict detection.

#### HA-03: Offline state doesn't prevent form submissions

**Severity:** Low
**Location:** `+layout.svelte:128-132`, all form pages

The `isOnline` store shows a banner when offline, but forms remain fully interactive. Submitting while
offline results in a network error after the fetch timeout.

**Recommendation:** Disable submit buttons or show an inline warning when `$isOnline` is false.

#### HA-04: No retry mechanism on non-settings pages

**Severity:** Low
**Location:** `services/+page.svelte`, `hosts/+page.svelte`

If the initial data load fails (e.g., transient 503 during controller restart), an error message is shown
but there's no retry button. The user must reload the page.

The settings page (`settings/+page.svelte`) has "Retry All" buttons, but services and hosts pages do not.

**Recommendation:** Add a retry button to the error states on all data pages.

#### HA-05: Auth redirect on refresh failure loses context

**Severity:** Low
**Location:** `lib/api.ts:162-165`

```typescript
setAccessToken(null);
window.location.href = '/login?redirect=' + encodeURIComponent(window.location.pathname + window.location.search);
```

When the refresh token is rejected (4xx), the user is immediately redirected to login. This loses any
unsaved form state. In an HA scenario with multiple controllers, a refresh token might be temporarily
invalid (e.g., during a rolling restart where the token was issued by a different instance).

The `redirect` parameter preserves the URL but not the form state.

**Recommendation:** Consider showing a modal ("Your session has expired. Please log in again.") instead
of an immediate redirect, giving the user a chance to copy their unsaved work.

#### HA-06: No WebSocket or SSE for real-time updates

**Severity:** Low (architectural observation)
**Location:** N/A

All data is fetch-on-demand or polling-based. The backend has a WebSocket endpoint
(`/api/v1/ws/service`) for agent-to-controller communication, but no equivalent for the frontend.

For the current scope (admin panel), polling is acceptable. Real-time updates would improve UX for
service status changes and MQTT connection status.

**Recommendation:** Document this as a future enhancement. Not required for current scope.

---

## 5. Testing

### 5.1 Current State

| Metric | Value |
|--------|-------|
| Test files | 1 (`lib/auth.test.ts`) |
| Test cases | 3 |
| Test framework | Vitest 4.0.18 + jsdom |
| Code coverage | Not configured |

The existing tests cover `initialize()`: success path, refresh failure, and existing token reuse. They
use proper mocking of the `api` module.

### 5.2 Coverage Gaps

| Area | Risk | Priority |
|------|------|----------|
| API client (`lib/api.ts`) — token refresh, error extraction, retry logic | High | High |
| Auth functions — `handleLogin`, `handleLogout`, `handleOidcLogin`, `handleOidcCallback` | High | High |
| Utility functions — `isValidLogoUrl`, `formatDate`, `copyToClipboard` | Medium | Medium |
| Route components — services, hosts, software, settings | Medium | Medium |
| Shared components — ModalBackdrop focus trapping, ContextMenu positioning | Low | Low |
| `safeRedirect()` — open redirect prevention | High | High |
| Integration / E2E tests | High | Medium |

### 5.3 Recommendations

1. Add tests for `safeRedirect()` — this is a security-critical function.
2. Add tests for `isValidLogoUrl()` — validates against XSS vectors.
3. Add tests for `authenticatedFetch` — the token refresh retry logic is complex.
4. Add tests for `extractErrorMessage` — the error parsing logic.
5. Configure code coverage in Vitest and set a minimum threshold.
6. Consider Playwright for E2E tests covering auth flows and settings management.

---

## 6. Accessibility

### 6.1 Positive Findings

| Finding | Location |
|---------|----------|
| `role="dialog"` and `aria-modal="true"` on all modal dialogs | `ConfirmDialog.svelte`, `ModalBackdrop.svelte` users |
| `role="menu"` and `role="menuitem"` on context menus | `ContextMenu.svelte`, `services/+page.svelte` |
| `aria-label` on action buttons (e.g., "Actions for {name}") | `services/+page.svelte:239`, `hosts/+page.svelte:157` |
| `aria-label="Pagination"` on pagination nav | `Pagination.svelte:14` |
| Focus trapping in `ModalBackdrop` with Tab/Shift+Tab cycling | `ModalBackdrop.svelte:28-49` |
| Focus restoration on modal close | `ModalBackdrop.svelte:20-25` |
| `lang="en"` on html element | `app.html:2` |
| Proper `autocomplete` attributes on form inputs | `login/+page.svelte`, `register/+page.svelte` |

### 6.2 Issues

#### A11Y-01: Toast notifications lack `aria-live` region

**Severity:** Medium
**Location:** `components/ToastNotifications.svelte`

Screen readers will not announce toast notifications because the container is not marked as a live region.

**Recommendation:** Add `role="status"` or `aria-live="polite"` to the toast container for success messages,
and `role="alert"` or `aria-live="assertive"` for error messages.

#### A11Y-02: No keyboard navigation in context menus

**Severity:** Low
**Location:** `components/ContextMenu.svelte`

Context menus support mouse clicks but not arrow key navigation. The WAI-ARIA menu pattern requires
`ArrowUp`/`ArrowDown` for item focus movement and `Enter`/`Space` for activation.

#### A11Y-03: No skip-to-content link

**Severity:** Low
**Location:** `+layout.svelte`

There is no skip link for keyboard users to bypass the header and sidebar navigation.

---

## 7. Summary Tables

### Issues by Severity

| Severity | Count | IDs |
|----------|-------|-----|
| Medium | 6 | SEC-01, ARC-02, CQ-01, CQ-02, HA-01, HA-02 |
| Low | 14 | ARC-01, ARC-03, ARC-04, SEC-02, SEC-03, SEC-04, SEC-05, CQ-03, CQ-04, CQ-05, CQ-06, CQ-07, HA-03, HA-04, HA-05, HA-06, A11Y-01, A11Y-02, A11Y-03 |

### Issues by Category

| Category | Count |
|----------|-------|
| Architecture | 4 |
| Security | 5 |
| Code Quality | 8 |
| Backend HA | 6 |
| Accessibility | 3 |

### File-Level Summary

| File | Issues | Notes |
|------|--------|-------|
| `app.html` | SEC-01, SEC-05 | CSP could be tightened |
| `lib/api.ts` | SEC-02, SEC-03 | Core API client is well-structured |
| `lib/auth.ts` | ARC-01 | Mixed store pattern |
| `lib/theme.ts` | ARC-01, CQ-06 | Listener cleanup |
| `lib/stores/network.ts` | ARC-01, CQ-05 | Listener cleanup |
| `lib/notifications.svelte.ts` | — | Clean implementation |
| `lib/utils.ts` | — | Clean implementation |
| `lib/types.ts` | — | Comprehensive type coverage |
| `lib/auth.test.ts` | — | Good tests, need more coverage |
| `+layout.svelte` | A11Y-03, HA-03 | Good auth guard pattern |
| `+layout.ts` | — | Clean |
| `login/+page.svelte` | SEC-04, CQ-07 | Complex but well-handled |
| `register/+page.svelte` | CQ-07 | Minor |
| `device/+page.svelte` | — | Good device code validation |
| `services/+page.svelte` | CQ-01, CQ-04, HA-04 | Double load bug |
| `hosts/+page.svelte` | HA-04 | No retry button |
| `software/+page.svelte` | — | Clean implementation |
| `settings/+page.svelte` | CQ-02, HA-01, HA-02 | Component ref pattern |
| `settings/EnrollmentTokenSettings.svelte` | CQ-03 | Code duplication |
| `settings/RegistrationSettings.svelte` | — | Clean |
| `settings/AuthenticationSettings.svelte` | — | Clean |
| `settings/AgentCertificateSettings.svelte` | — | Clean |
| `settings/MqttClientsSettings.svelte` | CQ-08 | Minor naming inconsistency |
| `settings/OidcProvidersSettings.svelte` | — | Clean |
| `settings/global/+page.svelte` | — | Clean |
| `components/ToastNotifications.svelte` | A11Y-01 | Missing aria-live |
| `components/ContextMenu.svelte` | A11Y-02 | No keyboard nav |
| `components/ConfirmDialog.svelte` | — | Clean |
| `components/ModalBackdrop.svelte` | — | Excellent focus management |
| `components/Pagination.svelte` | — | Clean and minimal |

---

## 8. Recommendations

### Priority 1 (Should fix)

1. **Fix double load bug on services page** (CQ-01): Remove the `onMount` call since `$effect` handles
   initial load.
2. **Add jitter to MQTT polling backoff** (HA-01): Prevents thundering herd in multi-client scenarios.
3. **Add linter configuration** (ARC-02): ESLint + Prettier + CI step.
4. **Add tests for security-critical functions** (Testing): `safeRedirect()`, `isValidLogoUrl()`,
   `authenticatedFetch` retry logic.
5. **Add `aria-live` to toast notifications** (A11Y-01): Required for screen reader users.

### Priority 2 (Should consider)

6. **Tighten CSP** (SEC-01): Replace `'unsafe-inline'` with nonce or hash for the theme script.
7. **Refactor settings to use props instead of imperative refs** (ARC-03, CQ-02): Eliminates
   `$state(undefined!)` and simplifies data flow.
8. **Extract reusable enrollment token component** (CQ-03): Reduces duplication from ~140 lines to ~50.
9. **Add retry buttons to services and hosts pages** (HA-04).
10. **Add `encodeURIComponent` to API path parameters** (SEC-02): Defense-in-depth.

### Priority 3 (Nice to have)

11. **Standardize state management** (ARC-01): Migrate remaining Svelte 4 stores to runes.
12. **Add stale data detection** (HA-02): ETag-based or periodic refresh.
13. **Add skip-to-content link** (A11Y-03).
14. **Add keyboard navigation to context menus** (A11Y-02).
15. **Disable submit buttons when offline** (HA-03).
16. **Show session-expired modal instead of hard redirect** (HA-05).
