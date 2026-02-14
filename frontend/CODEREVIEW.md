# Frontend Code Review

**Scope:** All 29 source files in `frontend/src/`, plus configuration files.
**Date:** 2026-02-13
**Branch:** `refactor/codereview-frontend`

## Executive Summary

The Uptrakit SvelteKit frontend is a compact admin dashboard (~2,400 LOC across
29 files) built on Svelte 5, SvelteKit 2, Skeleton UI, and Tailwind CSS v4. It
covers authentication (password + OIDC), service/host management, MQTT client
configuration, software tracking, and global settings.

The codebase demonstrates several strong practices: in-memory-only access tokens,
Content-Security-Policy headers, token refresh deduplication, strict logo URL
validation, and proper focus trap/keyboard handling in modals. However, the
review uncovered security vulnerabilities (XSS via raw HTML rendering), backend
resilience gaps (no timeouts, no retries, no offline handling), architectural
anti-patterns, and accessibility shortcomings that should be addressed before
production hardening.

Findings are organized by category below. Each finding includes a severity
level, the affected file(s) with line numbers, and a recommended fix.

---

## 1. Security and Safety

### ~~SEC-01: XSS via `{@html}` in ConfirmDialog (HIGH)~~ **FIXED**

**Status:** Resolved. Replaced `{@html message}` with structured `messagePrefix` + `entityName` props.
User-controlled data is now rendered via Svelte's auto-escaped `{expression}` syntax. All four
callers updated.
Either pass the entity name as a separate prop and use text interpolation with a
`<strong>` element in the template, or sanitize the message before rendering.

### SEC-02: CSP allows `unsafe-inline` for scripts (LOW)

**File:** `src/app.html:9`

The Content-Security-Policy header includes `script-src 'self' 'unsafe-inline'`.
The `unsafe-inline` is needed for the theme initialization script in `app.html`
(lines 12-18), so this is a conscious trade-off to prevent FOUC. However, it
weakens XSS protection since inline scripts injected via other vectors (such as
SEC-01) will execute.

**Recommendation:** After fixing SEC-01, consider moving the theme script to a
separate file and using a nonce or hash-based CSP instead of `unsafe-inline`.

### SEC-03: OIDC authorize URL redirect not validated (LOW)

**File:** `src/lib/auth.ts:63`

`handleOidcLogin` sets `window.location.href = authorize_url` where
`authorize_url` comes from the backend API response. If the backend is
compromised or a man-in-the-middle attack occurs, this could redirect to a
malicious URL.

**Recommendation:** Validate that `authorize_url` is an HTTPS URL before
redirecting. This is a defense-in-depth measure since the API should already be
trusted.

---

## 2. Backend High Availability and Error Handling

### HA-01: No request timeouts (HIGH)

**File:** `src/lib/api.ts:84`, `65`, `185`, `200`

Every `fetch()` call lacks a timeout. If the backend hangs (e.g. long GC pause,
network partition), the UI blocks indefinitely with no user feedback. This
affects both `authenticatedFetch` (line 84) and direct fetch calls in
`refreshAccessToken` (line 65), `oidcCompleteRegistration` (line 185), and
`oidcExchange` (line 200).

**Recommendation:** Wrap fetch calls with `AbortController` and a configurable
timeout (e.g. 30 seconds for normal requests, 10 seconds for refresh). Show a
user-friendly timeout error message.

```typescript
function fetchWithTimeout(url: string, options: RequestInit, timeoutMs = 30000): Promise<Response> {
    const controller = new AbortController();
    const id = setTimeout(() => controller.abort(), timeoutMs);
    return fetch(url, { ...options, signal: controller.signal }).finally(() => clearTimeout(id));
}
```

### HA-02: No retry or backoff for transient failures (MEDIUM)

**File:** `src/lib/api.ts` (entire module)

The API layer performs each request exactly once. Transient failures (502, 503,
network glitches) result in immediate error display. There is no retry with
exponential backoff.

**Recommendation:** Add a retry wrapper for idempotent GET requests with
exponential backoff (e.g. 3 retries with 1s, 2s, 4s delays). Non-idempotent
requests (POST/PUT/DELETE) should not auto-retry.

### HA-03: Network errors surface as unfriendly messages (MEDIUM)

**File:** `src/lib/api.ts:83-129`

When `fetch()` throws (network offline, DNS failure, TLS error), the error
propagates as `TypeError: Failed to fetch`. Page-level catch blocks display this
raw message to the user (e.g. `src/routes/hosts/+page.svelte:33`).

**Recommendation:** Catch `TypeError` in `authenticatedFetch` or `request` and
translate it into a user-friendly message like "Unable to connect to the server.
Check your network connection."

### HA-04: Token refresh treats all failures as auth expiry (MEDIUM)

**File:** `src/lib/api.ts:120-125`

The catch block in `authenticatedFetch` after a failed token refresh clears the
token and redirects to `/login` unconditionally. If the refresh endpoint fails
due to a network error or 500 status, the user is logged out unnecessarily.

**Recommendation:** Distinguish between auth failures (401, 403) and
infrastructure failures (network error, 5xx). Only redirect to login for auth
failures. For infrastructure failures, surface an error message and leave the
session intact.

### HA-05: Inconsistent parallel-request error handling (MEDIUM)

**Files:**
- `src/routes/software/+page.svelte:64` - uses `Promise.all`
- `src/routes/settings/+page.svelte:78` - uses `Promise.allSettled`
- `src/routes/settings/global/+page.svelte:45` - uses `Promise.allSettled`

The software page uses `Promise.all`, so a failure in either
`getSoftwareItems()` or `getProviderConfigs()` causes both to fail. The settings
pages correctly use `Promise.allSettled` for partial failure tolerance.

**Recommendation:** Use `Promise.allSettled` consistently for pages that load
multiple independent resources. Display which sections loaded and which failed.

### HA-06: No offline/online detection (LOW)

**File:** (codebase-wide)

There is no `navigator.onLine` check or `online`/`offline` event listener. When
the user goes offline, requests fail silently with generic error messages and no
recovery mechanism.

**Recommendation:** Add an offline banner using `navigator.onLine` and
`window.addEventListener('online'/'offline')`. Optionally auto-retry failed
requests when connectivity is restored.

### HA-07: No request cancellation on navigation (LOW)

**File:** `src/lib/api.ts` (entire module)

No `AbortController` is used, so in-flight requests are not cancelled when the
user navigates away from a page. This can cause stale responses to update state
after the component is destroyed.

**Recommendation:** Accept an optional `AbortSignal` in API functions. In page
components, create an `AbortController` in `onMount` and abort in the cleanup
function.

### HA-08: MQTT polling has no backoff on repeated failures (LOW)

**File:** `src/routes/settings/+page.svelte:57-66`

The MQTT client status is polled every 10 seconds with `setInterval`. Errors are
silently suppressed (line 63-65). If the backend is down, this generates one
failed request every 10 seconds indefinitely with no backoff.

**Recommendation:** Use exponential backoff on consecutive failures (e.g. 10s,
20s, 40s, up to 5 minutes). Reset to 10s after a successful response.

### HA-09: Partial settings load shows no failure indication (LOW)

**File:** `src/routes/settings/+page.svelte:78-96`

When `loadAllSettings()` uses `Promise.allSettled`, failed sections are silently
skipped. The user sees the loading spinner disappear (line 95) with some
sections showing stale or default data, with no indication of what failed.

**Recommendation:** Track which sections loaded successfully and display inline
error messages for sections that failed, with a retry button.

---

## 3. Architecture

### ARCH-01: Settings components use imperative `bind:this` + `export load()` anti-pattern (MEDIUM)

**File:** `src/routes/settings/+page.svelte:23-28`, `109-114`

All six settings sub-components expose an `export function load()` method called
imperatively via `bind:this` references. This creates fragile coupling: the
parent must wait for refs to be ready (`refsReady` flag + `tick()` on
lines 48-51), and calling methods on `undefined!` references before mount
crashes.

**Affected components:**
- `RegistrationSettings.svelte:17`
- `AuthenticationSettings.svelte:15`
- `MqttClientsSettings.svelte:37`
- `OidcProvidersSettings.svelte:45`
- `AgentCertificateSettings.svelte:16`
- `EnrollmentTokenSettings.svelte:25`, `29`

**Recommendation:** Replace with a declarative props-down pattern. Pass loaded
data as props to each settings component. Each component manages its own
internal state from the props it receives. This eliminates the need for
`bind:this`, the `refsReady` flag, and the `$state(undefined!)` workaround.

### ARCH-02: Missing `email_verified_trusted` toggle in OIDC form (MEDIUM)

**File:** `src/routes/settings/OidcProvidersSettings.svelte:30-41`, `103-154`

The `OidcProviderResponse` type includes `email_verified_trusted` (defined in
`src/lib/types.ts:149`), and both `CreateOidcProviderRequest` (line 166) and
`UpdateOidcProviderRequest` (line 333) accept it. However, the OIDC
create/edit form does not include a checkbox for this field, and
`saveOidcProvider()` never sends it.

This means admins cannot configure whether to trust the provider's email
verification claim, leaving it at the server default.

**Recommendation:** Add an `email_verified_trusted` checkbox to the OIDC form
and include it in both create and update payloads.

### ARCH-03: Software page hardcodes `perPage=500` for provider configs (LOW)

**File:** `src/routes/software/+page.svelte:66`

`getProviderConfigs(1, 500)` fetches up to 500 provider configs in a single
request. This works for small deployments but does not scale. If more than 500
configs exist, some will be missing from the dropdown.

**Recommendation:** Either paginate provider configs with a search/filter
dropdown, or document the hard limit. For most deployments this is acceptable,
but the magic number should be extracted to a named constant.

### ARCH-04: Client-side filtering with server-side pagination (MEDIUM)

**File:** `src/routes/services/+page.svelte:28-29`, `34-44`

The services page applies a `typeFilter` client-side (line 28-29) but loads data
with server-side pagination (line 37). This means filtering by "Agents" or "MQTT
Services" only filters the current page, not all services. A page with 20
services might show 5 agents after filtering, while agents on other pages are
invisible.

**Recommendation:** Pass the `status` or type filter to the `getServices()` API
call so the server paginates filtered results. If the API does not support type
filtering, add it.

### ARCH-05: No route-level error boundaries (LOW)

**File:** (missing `src/routes/+error.svelte`)

There is no `+error.svelte` file anywhere in the routes tree. Unhandled errors
in `load` functions or unexpected runtime errors will render SvelteKit's default
error page, which is unstyled and does not match the app's design.

**Recommendation:** Add a `src/routes/+error.svelte` that renders a styled error
page consistent with the app's theme, including a link to navigate back.

---

## 4. Code Quality

### CQ-01: Minimal test coverage (MEDIUM)

**File:** `src/lib/auth.test.ts`

Only 3 unit tests exist, covering `initialize()` in `auth.ts`. There are no
tests for:
- `api.ts` (error extraction, token refresh deduplication, retry-after-401 flow)
- `utils.ts` (`isValidLogoUrl`, `formatDate`, `copyToClipboard`)
- `notifications.svelte.ts`
- Any Svelte component

**Recommendation:** Prioritize tests for:
1. `api.ts` - especially `authenticatedFetch` 401 retry and
   `extractErrorMessage`
2. `utils.ts` - edge cases for `isValidLogoUrl` (javascript:, data:, http:)
3. Component tests for `ConfirmDialog` and `Pagination` using Svelte testing
   library

### CQ-02: Duplicated context menu and error handling patterns (LOW)

**Files:**
- `src/routes/hosts/+page.svelte:37-49`, `101-105`
- `src/routes/services/+page.svelte:46-58`, `127-131`

Both pages duplicate nearly identical code for:
- `toggleMenu()` / `closeMenu()` - menu open/close with positioning
- `handleWindowClick()` - click-outside detection using `.actions-menu` class
- Error state management (`let error: string | null = $state(null)`)
- Confirm dialog state management

**Recommendation:** Extract a reusable `useContextMenu` utility or a
higher-order component that encapsulates the menu toggle, positioning,
click-outside, and confirm dialog patterns.

### CQ-03: Fragile `$state(undefined!)` initialization (LOW)

**Files:**
- `src/routes/settings/+page.svelte:23-28`
- `src/lib/components/ContextMenu.svelte:15`

Using `$state(undefined!)` with the non-null assertion operator suppresses
TypeScript's strictness. If `bind:this` fails to initialize before access, this
causes a runtime crash with no helpful error message.

**Recommendation:** Use `$state<Type | null>(null)` with null checks, or
restructure to avoid the need for component references (see ARCH-01).

### CQ-04: Unused `getMqttLimit` API import path (LOW)

**File:** `src/lib/api.ts:342-348`

`getMqttLimit()` and `updateMqttLimit()` are exported but never called from any
component or page.

**Recommendation:** Either use these functions or remove them to reduce dead
code. If they are planned for future use, add a comment indicating that.

---

## 5. Coding Standards and Accessibility

### A11Y-01: Action buttons lack `aria-label` (MEDIUM)

**Files:**
- `src/routes/hosts/+page.svelte:156-162`
- `src/routes/services/+page.svelte:221-228`

The ellipsis action buttons (`&#8943;` / `⋯`) have no `aria-label`, so screen
readers announce them as "button" with no context about their purpose.

**Recommendation:** Add `aria-label="Actions for {entity.friendly_name}"` and
`aria-haspopup="menu"` to each ellipsis button.

### A11Y-02: Toast notifications lack ARIA live region (LOW)

**File:** `src/lib/components/ToastNotifications.svelte:11`

The toast container does not use `role="alert"` or `aria-live="polite"`. Screen
readers will not announce success/error toasts when they appear.

**Recommendation:** Add `role="status"` and `aria-live="polite"` to the toast
container `<div>`, or use `role="alert"` for error toasts.

### A11Y-03: Loading states not announced to screen readers (LOW)

**Files:**
- `src/routes/+layout.svelte:84-87`
- `src/routes/settings/+page.svelte:102-106`
- `src/routes/settings/global/+page.svelte:113-116`

Loading indicators are purely visual (`<p>Loading...</p>`). Screen readers are
not informed when content is loading or when it has finished.

**Recommendation:** Add `aria-busy="true"` to the main content area during
loading and use `aria-live="polite"` regions to announce when content is ready.
The settings page partially does this (line 108) but the layout and global
settings pages do not.

---

## 6. Positive Observations

- **In-memory access tokens** (`src/lib/auth.ts:8-9`): The access token is
  never persisted to `localStorage` or `sessionStorage`, reducing XSS impact.

- **Token refresh deduplication** (`src/lib/api.ts:98-103`): Concurrent 401
  responses share a single refresh cycle, preventing token refresh storms.

- **Content-Security-Policy** (`src/app.html:9`): The CSP restricts image
  sources to HTTPS, blocks `object-src`, limits `form-action` and `base-uri` to
  `self`.

- **Logo URL validation** (`src/lib/utils.ts:6-14`): `isValidLogoUrl()` only
  permits `https:` URLs, blocking `javascript:`, `data:`, and `http:` schemes.

- **Open redirect protection** (`src/routes/login/+page.svelte:89-95`):
  `safeRedirect()` validates that the redirect parameter starts with `/` and
  not `//`, preventing open redirect attacks.

- **Device code validation** (`src/routes/device/+page.svelte:10-12`): The
  device code is validated against a strict regex before use, preventing
  injection.

- **Proper focus management** (`src/lib/components/ModalBackdrop.svelte:12-26`):
  Modals trap focus, restore previous focus on close, and handle keyboard
  (Escape and Tab) correctly.

- **FOUC prevention** (`src/app.html:12-18`): An inline script applies the dark
  mode class before the framework renders, preventing a flash of unstyled
  content.

- **Consistent error handling pattern**: Every page uses the same
  `e instanceof Error ? e.message : 'fallback'` pattern for error extraction.

- **Clean type definitions** (`src/lib/types.ts`): All API types are defined in
  a single file with clear interfaces, making the API contract explicit.

---

## 7. Prioritized Action Items

### Priority 1 - Security (fix before production)

| ID | Finding | Effort |
|--------|---------------------------------------------|--------|
| ~~SEC-01~~ | ~~Fix XSS in ConfirmDialog `{@html}`~~ **FIXED** | ~~Small~~ |

### Priority 2 - Backend Resilience (fix for production readiness)

| ID | Finding | Effort |
|--------|---------------------------------------------|---------|
| HA-01 | Add request timeouts via AbortController | Medium |
| HA-03 | Translate network errors to friendly messages| Small |
| HA-04 | Distinguish network vs auth failures on refresh| Medium |
| HA-05 | Use `Promise.allSettled` consistently | Small |
| ARCH-04| Pass type filter to server in services page | Small |

### Priority 3 - Architecture and Quality (improve maintainability)

| ID | Finding | Effort |
|---------|----------------------------------------------|--------|
| ARCH-01 | Replace `bind:this`+`load()` with props-down | Medium |
| ARCH-02 | Add `email_verified_trusted` to OIDC form | Small |
| CQ-01 | Add tests for api.ts and utils.ts | Medium |
| A11Y-01 | Add `aria-label` to action buttons | Small |
| ARCH-05 | Add `+error.svelte` error boundary | Small |

### Priority 4 - Nice to Have (polish)

| ID | Finding | Effort |
|--------|---------------------------------------------|--------|
| HA-02 | Add retry/backoff for GET requests | Medium |
| HA-06 | Add offline/online detection | Small |
| HA-07 | Add AbortController for navigation cleanup | Medium |
| HA-08 | Add backoff to MQTT polling | Small |
| HA-09 | Show per-section failure in settings | Small |
| CQ-02 | Extract shared context menu utilities | Small |
| CQ-03 | Replace `$state(undefined!)` with null | Small |
| A11Y-02| Add ARIA live region to toasts | Small |
| A11Y-03| Announce loading states to screen readers | Small |
| SEC-02 | Replace `unsafe-inline` CSP with nonce/hash | Medium |
