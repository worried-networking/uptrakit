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

The review identifies remaining open issues across several categories:

- **Security:** OIDC redirect validation is protocol-only, CSP `img-src` allows any HTTPS image. (Error
  message length is now capped at 500 characters.)
- **Code quality:** Mixed Svelte 4/5 state management patterns, imperative API via `export function` for
  settings components, hardcoded API base path, network/theme event listeners never cleaned up, inconsistent
  MQTT client secret field naming.
- **Backend HA:** Auth redirect on refresh failure loses context, no WebSocket/SSE for real-time updates.
  (Offline form disabling has been added.)
- **Testing:** Route or shared components and integration/E2E tests remain uncovered. (Token refresh/retry
  logic and auth functions — `handleLogin`, `handleLogout`, `handleOidcCallback` — are now tested.)

---

## 1. Architecture

### 1.1 Positive Findings

| Finding                                                                                                      | Location                         |
| ------------------------------------------------------------------------------------------------------------ | -------------------------------- |
| Clean SPA architecture with SvelteKit static adapter; CSR-only is appropriate for an admin panel behind auth | `svelte.config.js`, `+layout.ts` |
| Good separation of concerns: `lib/` for shared code, `routes/` for pages, `components/` for reusable UI      | `src/` tree                      |
| Centralized API client with type-safe functions and consistent request/response patterns                     | `lib/api.ts`                     |
| Centralized auth guard in root layout prevents unauthorized access to protected routes                       | `+layout.svelte:48-57`           |
| Permission-based navigation filtering using derived state                                                    | `+layout.svelte:67-80`           |
| In-memory access token (never persisted to localStorage)                                                     | `lib/auth.ts:9`                  |
| Token refresh deduplication prevents redundant concurrent refresh requests                                   | `lib/api.ts:64,122-128`          |
| Settings components use `Promise.allSettled` for parallel loading with individual error handling             | `settings/+page.svelte:113-151`  |

### 1.2 Issues

#### ~~ARC-01: Mixed Svelte 4 / Svelte 5 state management patterns~~ RESOLVED

**Location:** `lib/auth.svelte.ts`, `lib/theme.svelte.ts`, `lib/stores/network.svelte.ts`

Svelte 4 `writable()` stores in `auth.ts`, `theme.ts`, and `stores/network.ts` have been
migrated to Svelte 5 runes. The old files have been deleted and replaced with
`.svelte.ts` counterparts using module-level `$state()` with exported getter/setter pairs
(e.g., `getUser()` / `setUser()`), matching the existing pattern in
`notifications.svelte.ts`. All 9 consumer files updated. Also resolves CQ-05 and CQ-06:
event listener cleanup documented in each file.

#### ~~ARC-03: Settings components use imperative API via `export function`~~ RESOLVED

**Location:** `settings/+page.svelte`, all six settings component files

State has been lifted to the parent (`settings/+page.svelte`). The parent now holds typed
data variables (e.g., `let registrationSettings: RegistrationSettings | undefined = $state(undefined)`)
and passes them as props to each component. Components accept a typed `settings` prop and
initialize their own local state via `$effect`. All `bind:this` refs, `export function load()` methods,
`$state(undefined!)` assertions, and the `refsReady` coordination flag have been removed.

#### ~~ARC-04: Single API base path with no configuration~~ RESOLVED

`BASE` in `lib/api.ts` now reads from `import.meta.env.VITE_API_BASE` with `/api/v1` as the fallback:

```typescript
const BASE: string = import.meta.env.VITE_API_BASE || '/api/v1';
```

`src/vite-env.d.ts` was created to declare the `ImportMetaEnv` interface with `VITE_API_BASE?: string`.
The variable is documented in `docs/end-user/deployment/reverse-proxy.md`.

---

## 2. Security and Safety

### 2.1 Positive Findings

| Finding                                                                                                                                                          | Location                    |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------- |
| Content Security Policy header in `app.html` restricts default-src, script-src, style-src, img-src, connect-src, font-src, object-src, base-uri, and form-action | `app.html:9`                |
| In-memory access token never touches localStorage or sessionStorage                                                                                              | `lib/auth.ts:9`             |
| Refresh token is HttpOnly + Secure + SameSite=Strict (backend-enforced)                                                                                          | Backend `refresh_cookie.rs` |
| OIDC authorize URL validated as HTTPS before redirect                                                                                                            | `lib/auth.ts:63-65`         |
| Logo URLs validated as HTTPS via `isValidLogoUrl()` using `URL()` constructor (rejects `javascript:`, `data:`, etc.)                                             | `lib/utils.ts:6-14`         |
| Redirect parameter validated: must start with `/` and not `//` (prevents open redirect)                                                                          | `login/+page.svelte:89-95`  |
| `credentials: 'same-origin'` on all fetch calls (refresh token cookie only sent to same origin)                                                                  | `lib/api.ts:85,108`         |
| Device code validated with strict regex before use                                                                                                               | `device/+page.svelte:10`    |
| `referrerpolicy="no-referrer"` on external OIDC provider images                                                                                                  | `login/+page.svelte:257`    |
| `encodeURIComponent` used for all URL parameters                                                                                                                 | Multiple locations          |
| `RefreshError` class distinguishes 4xx (real auth failure) from 5xx (transient) to avoid unnecessary logouts                                                     | `lib/api.ts:71-79`          |

### 2.2 Issues

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

| Finding                                                                                         | Location                                         |
| ----------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| TypeScript strict mode enabled                                                                  | `tsconfig.json`                                  |
| Consistent error handling pattern across all pages: `try/catch` with `Error` message extraction | All pages                                        |
| Well-implemented abort signal handling with `AbortSignal.timeout()` and `AbortSignal.any()`     | `lib/api.ts:102-105`                             |
| Clean component API with fully typed `$props()`                                                 | All components                                   |
| `ModalBackdrop` has proper focus trapping and focus restoration                                 | `components/ModalBackdrop.svelte`                |
| `ContextMenu` has viewport-aware positioning to prevent overflow                                | `components/ContextMenu.svelte`                  |
| `Pagination` is clean and minimal                                                               | `components/Pagination.svelte`                   |
| Proper use of `Promise.allSettled` for parallel loads with individual error handling            | `settings/+page.svelte`, `software/+page.svelte` |

### 3.2 Issues

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

| Finding                                                                                                | Location                        |
| ------------------------------------------------------------------------------------------------------ | ------------------------------- |
| Token refresh deduplication prevents concurrent refresh storms                                         | `lib/api.ts:122-128`            |
| `RefreshError` class separates 4xx (auth) from 5xx (transient) — avoids forced logout on server errors | `lib/api.ts:71-79,159-166`      |
| MQTT polling uses exponential backoff with jitter (10s base, 5min cap)                                 | `settings/+page.svelte:67-93`   |
| `Promise.allSettled` for settings loading — partial failure doesn't block other sections               | `settings/+page.svelte:113-151` |
| Individual "Retry All" buttons per settings section                                                    | `settings/+page.svelte:166-206` |
| Polling stopped on component destroy via `onDestroy`                                                   | `settings/+page.svelte:63-65`   |

### 4.2 Issues

#### ~~HA-05: Auth redirect on refresh failure loses context~~ RESOLVED

A non-blocking session-expired banner is now shown instead of an immediate hard redirect. On 4xx refresh
failure, `setAccessToken(null)` and `setSessionExpired(true)` are called. A dismissable `role="alert"`
`aria-live="assertive"` banner appears in `+layout.svelte` above page content, allowing the user to copy
unsaved work before following the login link. `window.location.href` is no longer assigned.
`lib/auth.svelte.ts` exposes `getSessionExpired()` / `setSessionExpired()` reactive state; `initialize()`
clears it on successful auth. The `api.test.ts` 4xx test asserts `setSessionExpired(true)` is called and
that `window.location.href` is not assigned.

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

| Metric         | Value                                                          |
| -------------- | -------------------------------------------------------------- |
| Test files     | 3 (`lib/auth.test.ts`, `lib/utils.test.ts`, `lib/api.test.ts`) |
| Test cases     | 46                                                             |
| Test framework | Vitest 4.0.18 + jsdom                                          |
| Code coverage  | Not configured                                                 |

The existing tests cover `initialize()`, `handleLogin`, `handleLogout`, `handleOidcCallback`
(auth, 9 tests), `isValidLogoUrl`, `formatDate`, `safeRedirect`, `copyToClipboard` (utils, 21
tests), `extractErrorMessage`, and `authenticatedFetch` including token refresh/retry logic,
deduplication, 4xx/5xx refresh failure handling, and timeout (api, 16 tests).

### 5.2 Coverage Gaps

| Area                                                                      | Risk   | Priority | Status |
| ------------------------------------------------------------------------- | ------ | -------- | ------ |
| Route components — services, hosts, software, settings                    | Medium | Medium   | Open   |
| Shared components — ModalBackdrop focus trapping, ContextMenu positioning | Low    | Low      | Open   |
| Integration / E2E tests                                                   | High   | Medium   | Open   |

### 5.3 Recommendations

1. Configure code coverage in Vitest and set a minimum threshold.
2. Consider Playwright for E2E tests covering auth flows and settings management.

---

## 6. Accessibility

### 6.1 Positive Findings

| Finding                                                      | Location                                              |
| ------------------------------------------------------------ | ----------------------------------------------------- |
| `role="dialog"` and `aria-modal="true"` on all modal dialogs | `ConfirmDialog.svelte`, `ModalBackdrop.svelte` users  |
| `role="menu"` and `role="menuitem"` on context menus         | `ContextMenu.svelte`, `services/+page.svelte`         |
| `aria-label` on action buttons (e.g., "Actions for {name}")  | `services/+page.svelte:239`, `hosts/+page.svelte:157` |
| `aria-label="Pagination"` on pagination nav                  | `Pagination.svelte:14`                                |
| Focus trapping in `ModalBackdrop` with Tab/Shift+Tab cycling | `ModalBackdrop.svelte:28-49`                          |
| Focus restoration on modal close                             | `ModalBackdrop.svelte:20-25`                          |
| `lang="en"` on html element                                  | `app.html:2`                                          |
| Proper `autocomplete` attributes on form inputs              | `login/+page.svelte`, `register/+page.svelte`         |

---

## 7. Summary Tables

### Issues by Severity

| Severity | Count | IDs                                                                                 |
| -------- | ----- | ----------------------------------------------------------------------------------- |
| Low      | 4     | SEC-04, SEC-05, CQ-08, HA-06 (ARC-01, ARC-03, ARC-04, CQ-05, CQ-06, HA-05 resolved) |

### Issues by Category

| Category      | Count |
| ------------- | ----- |
| Architecture  | 1     |
| Security      | 2     |
| Code Quality  | 1     |
| Backend HA    | 1     |
| Accessibility | 0     |

### File-Level Summary

| File                                       | Issues            | Notes                                                                                |
| ------------------------------------------ | ----------------- | ------------------------------------------------------------------------------------ |
| `app.html`                                 | SEC-05            | CSP script-src tightened with SHA-256 hash                                           |
| `lib/api.ts`                               | ~~ARC-04, HA-05~~ | BASE reads from `VITE_API_BASE`; hard redirect replaced by `setSessionExpired(true)` |
| `lib/auth.svelte.ts`                       | ~~ARC-01~~        | Migrated to Svelte 5 runes; `sessionExpired` state added                             |
| `lib/vite-env.d.ts`                        | —                 | New: `VITE_API_BASE` env declaration                                                 |
| `lib/theme.svelte.ts`                      | ~~ARC-01, CQ-06~~ | Migrated to runes; cleanup documented (renamed from `theme.ts`)                      |
| `lib/stores/network.svelte.ts`             | ~~ARC-01, CQ-05~~ | Migrated to runes; cleanup documented (renamed from `network.ts`)                    |
| `lib/notifications.svelte.ts`              | —                 | Clean implementation                                                                 |
| `lib/utils.ts`                             | —                 | Clean implementation                                                                 |
| `lib/types.ts`                             | —                 | Comprehensive type coverage                                                          |
| `lib/auth.test.ts`                         | —                 | 9 tests covering initialize, handleLogin, handleLogout, handleOidcCallback           |
| `+layout.svelte`                           | ~~HA-05~~         | Session-expired banner added (dismissable, non-blocking)                             |
| `+layout.ts`                               | —                 | Clean                                                                                |
| `login/+page.svelte`                       | SEC-04            | Offline disabling added; hasRedirected guard added                                   |
| `register/+page.svelte`                    | —                 | Offline disabling and hasRedirected guard added                                      |
| `device/+page.svelte`                      | —                 | Good device code validation                                                          |
| `services/+page.svelte`                    | —                 | Double load + fragile effect fixed                                                   |
| `hosts/+page.svelte`                       | —                 | Retry button added                                                                   |
| `software/+page.svelte`                    | —                 | Clean implementation                                                                 |
| `settings/+page.svelte`                    | —                 | Component refs now properly typed; jitter added to backoff                           |
| `settings/EnrollmentTokenSettings.svelte`  | —                 | Refactored to data-driven approach                                                   |
| `settings/RegistrationSettings.svelte`     | —                 | Clean                                                                                |
| `settings/AuthenticationSettings.svelte`   | —                 | Clean                                                                                |
| `settings/AgentCertificateSettings.svelte` | —                 | Clean                                                                                |
| `settings/MqttClientsSettings.svelte`      | CQ-08             | Minor naming inconsistency                                                           |
| `settings/OidcProvidersSettings.svelte`    | —                 | Clean                                                                                |
| `settings/global/+page.svelte`             | —                 | Clean                                                                                |
| `components/ToastNotifications.svelte`     | —                 | ARIA live regions added                                                              |
| `components/ContextMenu.svelte`            | —                 | Keyboard navigation added                                                            |
| `components/ConfirmDialog.svelte`          | —                 | Clean                                                                                |
| `components/ModalBackdrop.svelte`          | —                 | Excellent focus management                                                           |
| `components/Pagination.svelte`             | —                 | Clean and minimal                                                                    |

---

## 8. Recommendations

### Resolved in this PR

- ~~**Refactor settings to use props instead of imperative refs** (ARC-03)~~ — Done.
- ~~**Standardize state management** (ARC-01)~~ — Done. CQ-05/CQ-06 also resolved.

### Resolved in this PR (continued)

- ~~**Show session-expired banner instead of hard redirect** (HA-05)~~ — Done.
- ~~**Configurable API base path via `VITE_API_BASE`** (ARC-04)~~ — Done.

### Priority 1 (Should consider)

1. **Configure Vitest code coverage** and set a minimum threshold.

### Priority 2 (Nice to have)

2. **Add Playwright E2E tests** covering auth flows and settings management.
