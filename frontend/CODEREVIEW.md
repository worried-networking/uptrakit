# Frontend Code Review

**Scope**: `frontend/` — SvelteKit SPA (Skeleton UI v4 + Tailwind CSS v4)
**Reviewer**: Claude Opus 4.6
**Date**: 2026-02-08
**Files reviewed**: All 14 source files (4 lib modules, 8 route components, 2 config/style files)

---

## Executive Summary

The frontend is a compact, well-structured SvelteKit SPA for the Uptrakit controller dashboard. It handles authentication (password + OIDC), device authorization, agent/host management, and system settings. The codebase uses modern Svelte 5 runes and Skeleton UI v4 consistently.

However, the review identifies **28 findings** across 5 categories, including **critical API path mismatches** that would prevent the frontend from functioning against the current backend, **security concerns** around token storage and XSS vectors, and significant **architectural debt** from duplicated patterns across pages.

> **Implementation status**: Fix plans **FP-1 through FP-5** (plus FP-19 and FP-20) have been implemented in commit `f947964`. See individual findings below for details.

### Severity Distribution

| Severity | Count |
|----------|-------|
| Critical | 4 |
| High | 8 |
| Medium | 10 |
| Low | 6 |

---

## Category 1: Critical — API Contract Mismatches

### F-1: Frontend uses `/agents` endpoints but backend serves `/services` (Critical) — FIXED

**Files**: `src/lib/api.ts:194-216`, `src/lib/api.ts:272-282`

The frontend API client calls:
- `GET /api/v1/agents`
- `POST /api/v1/agents/{id}/approve`
- `POST /api/v1/agents/{id}/reject`
- `DELETE /api/v1/agents/{id}`
- `POST /api/v1/agents/{targetId}/merge`
- `GET /api/v1/agents/enrollment-token/status`
- `POST /api/v1/agents/enrollment-token`
- `DELETE /api/v1/agents/enrollment-token`

But the backend routes (confirmed in `crates/ui/web-api/src/routes/services.rs`) are all under `/api/v1/services`:
- `GET /api/v1/services`
- `POST /api/v1/services/{id}/approve`
- `POST /api/v1/services/{id}/reject`
- `DELETE /api/v1/services/{id}`
- `POST /api/v1/services/{target_id}/merge`
- `GET /api/v1/services/enrollment-token/status`
- `POST /api/v1/services/enrollment-token`
- `DELETE /api/v1/services/enrollment-token`

Every agent-related API call will 404 against the actual backend. The entire Agents page, enrollment token management, and service approval flows are broken.

**Fix plan (FP-1)**: Rename all `/agents` API paths to `/services` in `api.ts`. Update `getAgents()` to pass `?type=agent` filter. The backend `ListServicesQuery` supports `type` and `status` query params. **IMPLEMENTED** in `api.ts`.

### F-2: `AgentResponse` type does not match `ServiceResponse` from backend (Critical) — FIXED

**File**: `src/lib/types.ts:49-58`

The frontend defines:
```typescript
interface AgentResponse {
  id: string;
  hostname: string;
  friendly_name: string;
  ip_address: string | null;
  status: 'pending' | 'approved' | 'rejected';
  last_seen_at: string | null;
  created_at: string;
  updated_at: string;
}
```

But the backend `ServiceResponse` (per AGENTS.md) includes additional fields:
- `service_type` (Agent/Mqtt)
- `client_version`
- `tenant_id`

And the status field is actually `'pending' | 'approved' | 'rejected' | 'deactivated'` (the `deactivated` status is not modeled).

**Fix plan (FP-2)**: Rename to `ServiceResponse`, add missing fields, and update all consuming components. Consider keeping a `type AgentResponse = ServiceResponse` alias during migration. **IMPLEMENTED** in `types.ts`, `api.ts`, `agents/+page.svelte`.

### F-3: Agents page does not filter by service type (Critical) — FIXED

**File**: `src/routes/agents/+page.svelte:31`, `src/lib/api.ts:194-197`

`getAgents()` calls the list endpoint without a `type=agent` filter. Once the path is fixed to `/services`, this will return both agents AND MQTT services mixed together, which is incorrect for the "Agents" page.

**Fix plan (FP-3)**: Pass `?type=agent` as a default filter in `getAgents()`. Consider adding an MQTT Services page as well. **IMPLEMENTED** — folded into FP-1; `getAgents()` always passes `type=agent`.

### F-4: Missing MQTT enrollment token endpoints (Critical) — FIXED

**File**: `src/lib/api.ts`

The settings page manages agent enrollment tokens but has no support for MQTT enrollment tokens. Per AGENTS.md, MQTT enrollment tokens use `?type=mqtt` on the same `/services/enrollment-token` endpoints. The settings page should distinguish between agent and MQTT enrollment tokens.

**Fix plan (FP-4)**: Add `type` parameter to enrollment token API functions. Add MQTT enrollment token section to the settings page. **IMPLEMENTED** in `api.ts` and `settings/+page.svelte`.

---

## Category 2: Security & Safety

### F-5: JWT tokens stored in `localStorage` — vulnerable to XSS (High) — FIXED

**Files**: `src/lib/auth.ts:27-28`, `src/lib/api.ts:53`

Both `access_token` and `refresh_token` are stored in `localStorage`. Any XSS vulnerability (including from third-party dependencies or OIDC provider `logo_url` injection) gives an attacker full access to steal both tokens.

`localStorage` is synchronous, accessible from any script in the same origin, and persists indefinitely. The `refresh_token` is particularly dangerous because it can mint new access tokens.

**Fix plan (FP-5)**: Move `refresh_token` to an `HttpOnly`, `Secure`, `SameSite=Strict` cookie managed by the backend. The `access_token` can remain in memory (not `localStorage`) — store it in a module-level variable that is lost on page refresh. On refresh, the `HttpOnly` cookie silently provides a new access token via the refresh endpoint. **IMPLEMENTED** — backend (`refresh_cookie.rs`, `auth.rs`, `oidc_auth.rs`) sets/reads/clears HttpOnly cookies; frontend (`auth.ts`, `api.ts`) uses in-memory token with `credentials: 'same-origin'`.

### F-6: OIDC `logo_url` rendered as `<img src>` without sanitization (High)

**File**: `src/routes/login/+page.svelte:247`

```svelte
<img src={provider.logo_url} alt="" class="h-5 w-5" />
```

The `logo_url` comes from the database (admin-configured), but if an attacker compromises an admin account or the database, they could set `logo_url` to a `javascript:` URI or a tracking pixel. While modern browsers block `javascript:` in `img.src`, the URL is rendered without any validation.

More importantly, this is an SSRF vector — a malicious `logo_url` pointing to an internal network address would cause every user's browser to make a request to that address on page load.

**Fix plan (FP-6)**: Validate `logo_url` client-side: only allow `https://` URLs. Consider proxying logos through the backend or using a Content Security Policy `img-src` directive.

### F-7: Device authorization code displayed without sanitization (Medium)

**File**: `src/routes/device/+page.svelte:63`

```svelte
<span class="font-mono text-3xl font-bold tracking-widest">{code}</span>
```

The `code` comes directly from URL search params (`$page.url.searchParams.get('code')`). While Svelte auto-escapes text content (so this is safe against HTML injection), the code is also passed to the API:

```svelte
href="/login?redirect=/device?code={encodeURIComponent(code)}"
```

This constructs a URL with user-controlled content that gets placed in a query parameter. The `encodeURIComponent` call is correct, but the `redirect` parameter itself is not validated by the login page — it could be used for open redirect attacks.

**Fix plan (FP-7)**: Validate the `redirect` parameter on the login page to only allow same-origin paths (must start with `/` and not `//`). The device code format (8 uppercase consonants) should also be validated client-side before display.

### F-8: No Content Security Policy (CSP) headers (Medium)

**File**: `src/app.html`

The application has no CSP meta tag or header configuration. A CSP would significantly reduce the impact of any XSS vulnerability by restricting script sources, style sources, and connection targets.

**Fix plan (FP-8)**: Add a strict CSP via meta tag in `app.html` or configure it via the backend's response headers. At minimum: `default-src 'self'; img-src 'self' https:; style-src 'self' 'unsafe-inline'; script-src 'self'`.

### F-9: Registration token displayed in plaintext, no copy-to-clipboard (Medium)

**File**: `src/routes/settings/+page.svelte:762-763`

```svelte
<code class="break-all">{generatedToken}</code>
```

The enrollment token is displayed as plaintext with only a text warning to copy it. No copy-to-clipboard button is provided, increasing the risk the user won't copy it properly. The token also remains visible in the DOM until the user navigates away.

**Fix plan (FP-9)**: Add a copy-to-clipboard button. Consider masking the token after copy with a "show again" toggle.

### F-10: `window.location.href` redirect in token refresh uses hardcoded path (Low)

**File**: `src/lib/api.ts:117`

```typescript
window.location.href = '/login';
```

On refresh failure, this does a hard navigation that loses the current page context. The user cannot return to where they were after re-authentication.

**Fix plan (FP-10)**: Include the current path as a `redirect` query parameter: `window.location.href = '/login?redirect=' + encodeURIComponent(window.location.pathname + window.location.search)`. Then handle the redirect parameter on the login page after successful auth.

---

## Category 3: Architecture & Code Quality

### F-11: Settings page is 952 lines — violates single responsibility (High)

**File**: `src/routes/settings/+page.svelte` — 952 lines

This single component manages 6 distinct settings sections (Registration, Authentication, MQTT, Agent Certificates, Enrollment Token, OIDC Providers), each with its own state, modals, forms, and CRUD operations. This makes it:
- Hard to review for correctness (easy to miss bugs in 952 lines)
- Hard to test or modify one section without risk to others
- Hard for multiple developers to work on simultaneously

**Fix plan (FP-11)**: Extract each section into its own component: `RegistrationSettings.svelte`, `AuthenticationSettings.svelte`, `MqttClientsSettings.svelte`, `AgentCertificateSettings.svelte`, `EnrollmentTokenSettings.svelte`, `OidcProvidersSettings.svelte`. The parent settings page becomes a thin shell that renders these components.

### F-12: Duplicated auth guard pattern across every page (High)

**Files**: `+page.svelte:5-9`, `agents/+page.svelte:16-20`, `hosts/+page.svelte:15-19`, `settings/+page.svelte:110-116`, `settings/global/+page.svelte:34-39`

Every page independently implements auth checking:
```svelte
$effect(() => {
  if (!$user) goto('/login');
});
```

And some also check permissions:
```svelte
$effect(() => {
  if (!$user) goto('/login');
  else if (!canManageSettings) goto('/');
});
```

This is error-prone — a new page could forget the guard entirely. It also creates a flash of content before the redirect fires.

**Fix plan (FP-12)**: Implement auth guards in `+layout.ts` or a layout `load` function. Create a shared `requireAuth(permissions?: Permission[])` utility that can be called in layout/page `load` functions to redirect before the component renders.

### F-13: Duplicated context menu / modal / confirmation patterns (High)

**Files**: `agents/+page.svelte`, `hosts/+page.svelte`

Both pages duplicate:
- Context menu positioning logic (`toggleMenu`, `closeMenu`, `menuPos`, `openMenuId`)
- Confirmation dialog state management (`confirmAction`, `cancelConfirm`, `executeConfirmed`)
- Modal backdrop click/escape handlers
- Window click handler for closing menus
- `formatDate()` function (identical in both)
- `handleWindowClick()` function (identical in both)
- `<!-- svelte-ignore a11y_no_static_element_interactions -->` suppressions

**Fix plan (FP-13)**: Extract into shared components:
- `ContextMenu.svelte` — positioned dropdown menu
- `ConfirmDialog.svelte` — generic confirmation modal
- `ModalBackdrop.svelte` — backdrop with click-outside-to-close
- `lib/utils.ts` — shared `formatDate()` and other utilities

### F-14: Duplicated success/error notification pattern (Medium)

**Files**: `settings/+page.svelte:124-137`, `settings/global/+page.svelte:48-61`

Both settings pages duplicate `showSuccess()`, `showError()`, `clearError()` with identical `setTimeout` logic.

**Fix plan (FP-14)**: Create a `notifications.ts` store or a `Notifications.svelte` component that manages toast-style notifications globally.

### F-15: `$effect()` for data loading runs on every reactive dependency change (Medium)

**Files**: `src/routes/login/+page.svelte:76-85`, `src/routes/agents/+page.svelte:22-26`

```svelte
$effect(() => {
  getAuthMethods().then((methods) => { authMethods = methods; });
});
```

This `$effect` has no explicit dependencies, so it runs on mount — but in Svelte 5, `$effect` re-runs when any reactive value it reads changes. Since this reads nothing reactive, it effectively runs once. However, the pattern is fragile — if someone adds a reactive read inside the callback, it would re-fire the API call unexpectedly.

The agents page has a clearer issue:
```svelte
$effect(() => {
  if ($user) loadAgents();
});
```

This re-runs every time `$user` changes (e.g., if the user object is replaced after a token refresh), causing unnecessary API calls.

**Fix plan (FP-15)**: Use `onMount()` for one-time data fetching. Use `$effect` only for genuinely reactive logic.

### F-16: No loading states on action buttons (Medium)

**Files**: `agents/+page.svelte:258`, `hosts/+page.svelte:222`, `settings/+page.svelte:539-551`

Destructive actions (approve, reject, delete, deactivate) have no loading/disabled state while the API call is in flight. Users can double-click and trigger duplicate requests.

The device authorization page correctly implements this pattern (`approving` state), but other pages do not.

**Fix plan (FP-16)**: Add `submitting` state to action handlers and disable buttons during API calls.

### F-17: Hosts page does not support pagination controls (Medium)

**File**: `src/routes/hosts/+page.svelte:30`

```typescript
const result = await getHosts();
hosts = result.items;
```

The API returns a `PaginatedResponse` with `total`, `page`, `per_page`, `total_pages`, but the hosts page only extracts `items` and never renders pagination controls. Users with many hosts cannot navigate beyond the first page (20 items by default).

**Fix plan (FP-17)**: Add page navigation controls. Track `currentPage` and `totalPages` state. Call `getHosts(page, perPage)` when page changes.

### F-18: Agents page uses flat list without pagination (Medium)

**File**: `src/lib/api.ts:194-197`

`getAgents()` returns `Promise<AgentResponse[]>` — it assumes the backend returns a flat array. But per the AGENTS.md `ListServicesQuery`, the backend uses `PaginatedResponse`. The frontend type should be `PaginatedResponse<AgentResponse>`.

**Fix plan (FP-18)**: Update `getAgents()` to return `PaginatedResponse<AgentResponse>` and add pagination controls to the agents page.

---

## Category 4: Type Safety & Correctness

### F-19: `User.permissions` typed as `string[]` instead of `Permission[]` (High) — FIXED

**File**: `src/lib/types.ts:19`

```typescript
export interface User {
  permissions: string[];
}
```

The `Permission` enum exists but `User.permissions` is typed as `string[]`. This means permission checks use string comparison without type safety:

```typescript
$user?.permissions.includes(Permission.ManageSettings)
```

This works at runtime because `Permission.ManageSettings === 'manage_settings'`, but offers no protection against typos in other parts of the code.

**Fix plan (FP-19)**: Change `permissions: string[]` to `permissions: Permission[]`. **IMPLEMENTED** in `types.ts` (as part of FP-2).

### F-20: MQTT form uses `Record<string, unknown>` instead of typed interface (High) — FIXED

**File**: `src/routes/settings/+page.svelte:210-219`, `228-235`

```typescript
const data: Record<string, unknown> = {
  url: mqttForm.url || undefined,
  enabled: mqttForm.enabled,
  ...
};
const res = await updateMqttClient(editingMqttClient.id, data);
```

The form builds an untyped `Record<string, unknown>` that is passed to `updateMqttClient()` which expects `UpdateMqttClient`. This bypasses TypeScript's type checking entirely.

**Fix plan (FP-20)**: Build a properly typed `UpdateMqttClient` object directly. Use conditional spreading for optional fields. **IMPLEMENTED** in `settings/+page.svelte` (as part of FP-4).

### F-21: `refreshPromise` race condition between clear and reuse (Medium)

**File**: `src/lib/api.ts:57-121`

```typescript
let refreshPromise: Promise<RefreshResponse> | null = null;

async function request<T>(...) {
  if (res.status === 401) {
    try {
      if (!refreshPromise) {
        refreshPromise = refreshAccessToken();
      }
      const refreshed = await refreshPromise;
      ...
    } catch {
      ...
    } finally {
      refreshPromise = null;  // <-- cleared in finally
    }
  }
}
```

If request A triggers refresh and request B arrives while A is awaiting the retry, B will find `refreshPromise` already set and await it. But A's `finally` block clears `refreshPromise = null` after the first awaiter resolves. If request C arrives between A's `finally` and B's resolution, C will start a **new** refresh. This is a minor race condition that could cause an unnecessary second refresh call.

**Fix plan (FP-21)**: Only clear `refreshPromise` when the last awaiter has consumed it, or use a more robust mutual-exclusion pattern (e.g., a single-flight wrapper).

### F-22: `204` response cast to generic type `T` (Low)

**File**: `src/lib/api.ts:111, 128`

```typescript
if (res.status === 204) return undefined as T;
```

This is technically a lie — the function promises to return `T` but returns `undefined`. Callers that expect a body from a 204 endpoint would get `undefined` at runtime with no type error.

**Fix plan (FP-22)**: Return type should be `Promise<T | void>` for endpoints that may return 204. Alternatively, have delete functions explicitly return `Promise<void>`.

### F-23: `OidcCompleteRegistrationRequest` interface defined but never used (Low)

**File**: `src/lib/types.ts:94-97`

```typescript
export interface OidcCompleteRegistrationRequest {
  registration_code: string;
  registration_token: string;
}
```

This interface is defined but `oidcCompleteRegistration()` in `api.ts` manually builds its body with `JSON.stringify({ registration_code, registration_token })` instead of using the type.

**Fix plan (FP-23)**: Either use the interface in the API function signature or remove it.

---

## Category 5: UX & Accessibility

### F-24: Multiple `a11y_no_static_element_interactions` suppressions (High)

**Files**: `agents/+page.svelte:244,267`, `hosts/+page.svelte:208,231`, `settings/+page.svelte:622,782,865,928`

Eight instances of `<!-- svelte-ignore a11y_no_static_element_interactions -->` suppress accessibility warnings on modal backdrop `<div>` elements. These elements have `onclick` and `onkeydown` handlers but no `role` attribute, making them invisible to screen readers.

**Fix plan (FP-24)**: Add `role="dialog"` and `aria-modal="true"` to modal containers. Add `role="presentation"` to backdrop overlays. Use proper focus trapping inside modals.

### F-25: No focus management in modals (Medium)

**Files**: All modal implementations across agents, hosts, and settings pages.

When a modal opens, focus is not moved to the modal. Keyboard users can tab behind the modal into invisible content. When a modal closes, focus is not returned to the trigger button.

**Fix plan (FP-25)**: Implement focus trapping: on open, move focus to the first focusable element in the modal. On close, restore focus to the trigger. Consider using a library like `focus-trap` or implementing a `use:focusTrap` Svelte action.

### F-26: Context menus are positioned with fixed pixel coordinates (Medium)

**Files**: `agents/+page.svelte:42-43`, `hosts/+page.svelte:42-43`

```typescript
const rect = button.getBoundingClientRect();
menuPos = { top: rect.bottom + 4, left: rect.right - 160 };
```

The menu is positioned using `fixed` positioning based on `getBoundingClientRect()`. This breaks when:
- The page is scrolled after the menu is positioned
- The menu would overflow the viewport at the bottom or right edge
- The window is resized while the menu is open

**Fix plan (FP-26)**: Use the Popover API or a library like Floating UI for robust positioning. At minimum, add viewport boundary checks.

### F-27: No empty-state guidance for new users (Low)

**Files**: `agents/+page.svelte:184-186`, `hosts/+page.svelte:167-169`

Empty states show minimal text:
```svelte
<td colspan="6" class="text-center">No agents found.</td>
```

New users get no guidance on how to add agents or hosts.

**Fix plan (FP-27)**: Add helpful empty states: "No agents found. To add an agent, install the uptrakit-agent binary on a host and point it to this controller." Include a link to docs.

### F-28: Theme flash prevention is duplicated (Low)

**Files**: `src/app.html:11-17`, `src/lib/theme.ts:28-37`

The inline script in `app.html` reads `localStorage` and sets the `dark` class to prevent FOUC. Then `initTheme()` in `theme.ts` does the same thing on mount. The logic is duplicated and could diverge.

**Fix plan (FP-28)**: Keep the inline script for FOUC prevention (it must run before paint). In `initTheme()`, only set up the media query listener — skip the initial class toggle since it's already done.

---

## Fix Plan Summary

| ID | Severity | Effort | Description | Status |
|----|----------|--------|-------------|--------|
| FP-1 | Critical | Small | Fix API paths: `/agents` → `/services` | **Done** |
| FP-2 | Critical | Small | Align `AgentResponse` type with backend `ServiceResponse` | **Done** |
| FP-3 | Critical | Small | Add `?type=agent` filter to service list calls | **Done** |
| FP-4 | Critical | Medium | Add MQTT enrollment token support | **Done** |
| FP-5 | High | Large | Move refresh token to HttpOnly cookie | **Done** |
| FP-6 | High | Small | Validate OIDC logo URLs (https-only) | Open |
| FP-7 | Medium | Small | Validate redirect parameter, validate device code format | Open |
| FP-8 | Medium | Small | Add Content Security Policy | Open |
| FP-9 | Medium | Small | Add copy-to-clipboard for enrollment token | Open |
| FP-10 | Low | Small | Preserve redirect path on session expiry | Open |
| FP-11 | High | Medium | Extract settings page into sub-components | Open |
| FP-12 | High | Medium | Centralize auth guards in layout | Open |
| FP-13 | High | Medium | Extract shared menu/modal/confirm components | Open |
| FP-14 | Medium | Small | Create shared notification system | Open |
| FP-15 | Medium | Small | Use `onMount` for data fetching, not `$effect` | Open |
| FP-16 | Medium | Small | Add loading states to action buttons | Open |
| FP-17 | Medium | Medium | Add pagination controls to hosts page | Open |
| FP-18 | Medium | Medium | Fix agents pagination support | Open |
| FP-19 | High | Small | Type `User.permissions` as `Permission[]` | **Done** |
| FP-20 | High | Small | Use typed interfaces for MQTT form data | **Done** |
| FP-21 | Medium | Small | Fix refresh token race condition | Open |
| FP-22 | Low | Small | Fix 204 response type handling | Open |
| FP-23 | Low | Small | Remove unused `OidcCompleteRegistrationRequest` or use it | Open |
| FP-24 | High | Medium | Add ARIA roles to modals | Open |
| FP-25 | Medium | Medium | Implement focus trapping in modals | Open |
| FP-26 | Medium | Medium | Use robust menu positioning | Open |
| FP-27 | Low | Small | Add helpful empty states | Open |
| FP-28 | Low | Small | Deduplicate theme initialization logic | Open |

### Recommended Priority Order

1. **FP-1, FP-2, FP-3** — Critical: Fix API paths (the app is non-functional without this)
2. **FP-19, FP-20** — High: Type safety fixes (prevents runtime bugs)
3. **FP-5, FP-6, FP-8** — High: Security hardening
4. **FP-11, FP-12, FP-13** — High: Architecture refactoring (reduces future bug surface)
5. **FP-24, FP-25** — High/Medium: Accessibility compliance
6. Everything else by severity

---

## Positive Observations

1. **Consistent Svelte 5 usage**: Proper use of `$state`, `$derived`, `$effect`, and `$props` runes throughout.
2. **Good error handling pattern**: `extractErrorMessage()` gracefully handles JSON and text error responses.
3. **Token refresh deduplication**: The `refreshPromise` pattern correctly prevents concurrent refresh calls (minor race aside).
4. **Permission-based UI**: Navigation and action buttons correctly check user permissions.
5. **Dark mode support**: Clean theme cycling with FOUC prevention.
6. **OIDC flow completeness**: Account linking, registration with token, device authorization — all covered.
7. **Proper autocomplete hints**: Login/register forms use correct `autocomplete` attributes for password managers.
8. **Static adapter**: Correct choice for embedding in the controller binary.

---

## Detailed Fixing Plans (Top 5)

The following are implementation-ready plans for the 5 worst findings. Each includes exact files, before/after code, step-by-step instructions, dependencies between plans, and verification steps.

---

### DFP-1: Fix API paths — `/agents` → `/services` (F-1, Critical) — IMPLEMENTED

**Goal**: Every frontend API call that currently hits `/api/v1/agents/…` must be rewritten to hit `/api/v1/services/…`, matching the backend routes defined in `crates/ui/web-api/src/routes/services.rs`.

#### Files to modify

| File | Lines | Change |
|------|-------|--------|
| `src/lib/api.ts` | 194–216 | Rename all `/agents/` paths to `/services/` |
| `src/lib/api.ts` | 272–282 | Rename enrollment-token paths from `/agents/enrollment-token` to `/services/enrollment-token` |

#### Step-by-step

**Step 1 — Rename CRUD functions' paths** (`src/lib/api.ts:194-216`)

Before:
```typescript
export function getAgents(status?: string): Promise<AgentResponse[]> {
	const query = status ? `?status=${status}` : '';
	return request(`/agents${query}`);
}

export function approveAgent(id: string): Promise<AgentResponse> {
	return request(`/agents/${id}/approve`, { method: 'POST' });
}

export function rejectAgent(id: string): Promise<AgentResponse> {
	return request(`/agents/${id}/reject`, { method: 'POST' });
}

export function deleteAgent(id: string): Promise<MessageResponse> {
	return request(`/agents/${id}`, { method: 'DELETE' });
}

export function mergeAgent(targetId: string, sourceId: string): Promise<AgentResponse> {
	return request(`/agents/${targetId}/merge`, {
		method: 'POST',
		body: JSON.stringify({ source_id: sourceId })
	});
}
```

After:
```typescript
export function getAgents(status?: string): Promise<PaginatedResponse<ServiceResponse>> {
	const params = new URLSearchParams();
	params.set('type', 'agent');
	if (status) params.set('status', status);
	return request(`/services?${params.toString()}`);
}

export function approveAgent(id: string): Promise<ServiceResponse> {
	return request(`/services/${id}/approve`, { method: 'POST' });
}

export function rejectAgent(id: string): Promise<ServiceResponse> {
	return request(`/services/${id}/reject`, { method: 'POST' });
}

export function deleteAgent(id: string): Promise<MessageResponse> {
	return request(`/services/${id}`, { method: 'DELETE' });
}

export function mergeAgent(targetId: string, sourceId: string): Promise<ServiceResponse> {
	return request(`/services/${targetId}/merge`, {
		method: 'POST',
		body: JSON.stringify({ source_id: sourceId })
	});
}
```

Key changes:
- All `/agents/…` paths become `/services/…`.
- `getAgents()` always passes `type=agent` (see DFP-3).
- Return types switch from `AgentResponse` to `ServiceResponse` (see DFP-2).
- `getAgents()` now returns `PaginatedResponse<ServiceResponse>` to match the backend's paginated list endpoint.

**Step 2 — Rename enrollment-token paths** (`src/lib/api.ts:272-282`)

Before:
```typescript
export function getEnrollmentTokenStatus(): Promise<EnrollmentTokenStatus> {
	return request('/agents/enrollment-token/status');
}

export function createEnrollmentToken(): Promise<EnrollmentTokenResponse> {
	return request('/agents/enrollment-token', { method: 'POST' });
}

export function revokeEnrollmentToken(): Promise<MessageResponse> {
	return request('/agents/enrollment-token', { method: 'DELETE' });
}
```

After:
```typescript
export function getEnrollmentTokenStatus(type: 'agent' | 'mqtt' = 'agent'): Promise<EnrollmentTokenStatus> {
	return request(`/services/enrollment-token/status?type=${type}`);
}

export function createEnrollmentToken(type: 'agent' | 'mqtt' = 'agent'): Promise<EnrollmentTokenResponse> {
	return request(`/services/enrollment-token?type=${type}`, { method: 'POST' });
}

export function revokeEnrollmentToken(type: 'agent' | 'mqtt' = 'agent'): Promise<MessageResponse> {
	return request(`/services/enrollment-token?type=${type}`, { method: 'DELETE' });
}
```

Key changes:
- Paths change from `/agents/enrollment-token` to `/services/enrollment-token`.
- A `type` parameter is added (defaults to `'agent'`), preparing for DFP-4.

**Step 3 — Update imports in `api.ts`** (`src/lib/api.ts:1-34`)

Remove `AgentResponse` from the import block and add `ServiceResponse` (once DFP-2 creates it).

#### Dependencies

- **DFP-2** must be completed first (or simultaneously) — `ServiceResponse` must exist in `types.ts` before `api.ts` can reference it.
- **DFP-3** is folded into Step 1 above (the `type=agent` filter).

#### Verification

1. `npm run check` — no TypeScript errors.
2. Open the Agents page in the browser with DevTools Network tab: requests should go to `/api/v1/services?type=agent` (not `/api/v1/agents`).
3. Approve/reject/delete/merge actions should call `/api/v1/services/{id}/…`.
4. Settings → Enrollment Token should call `/api/v1/services/enrollment-token/…`.
5. All requests should return 200/201/204 (not 404).

---

### DFP-2: Align `AgentResponse` type with backend `ServiceResponse` (F-2, Critical) — IMPLEMENTED

**Goal**: Replace the frontend's `AgentResponse` interface with a `ServiceResponse` interface that matches the backend struct defined in `crates/shared/web-api-types/src/services.rs:54-67`.

#### Files to modify

| File | Lines | Change |
|------|-------|--------|
| `src/lib/types.ts` | 49–58 | Replace `AgentResponse` with `ServiceResponse` |
| `src/lib/types.ts` | (new) | Add `ServiceType` and `ServiceStatus` types |
| `src/lib/api.ts` | 1–5 | Update import to use `ServiceResponse` |
| `src/routes/agents/+page.svelte` | 5, 7, 60, 62, 93 | Update type references |

#### Step-by-step

**Step 1 — Add `ServiceType` and `ServiceStatus`, replace `AgentResponse`** (`src/lib/types.ts`)

Before (`types.ts:49-58`):
```typescript
export interface AgentResponse {
	id: string;
	hostname: string;
	friendly_name: string;
	ip_address: string | null;
	status: 'pending' | 'approved' | 'rejected';
	last_seen_at: string | null;
	created_at: string;
	updated_at: string;
}
```

After:
```typescript
export type ServiceType = 'agent' | 'mqtt';

export type ServiceStatus = 'pending' | 'approved' | 'rejected' | 'deactivated';

export interface ServiceResponse {
	id: string;
	service_type: ServiceType;
	hostname: string;
	friendly_name: string;
	ip_address: string | null;
	status: ServiceStatus;
	client_version: string | null;
	last_seen_at: string | null;
	created_at: string;
	updated_at: string;
}
```

Key changes vs. the old `AgentResponse`:
- **Added `service_type`** — `'agent'` or `'mqtt'` (backend: `ServiceType` enum, serde `rename_all = "snake_case"`).
- **Added `'deactivated'` to `status`** — the backend's `ServiceStatus` has four variants, not three.
- **Added `client_version`** — `string | null`, reported by the agent/mqtt binary on connection.
- **Renamed** from `AgentResponse` to `ServiceResponse` to match the backend.

**Step 2 — Update `HostAgentSummary` status type** (`src/lib/types.ts:235`)

Before:
```typescript
export interface HostAgentSummary {
	id: string;
	friendly_name: string;
	status: 'pending' | 'approved' | 'rejected';
}
```

After:
```typescript
export interface HostAgentSummary {
	id: string;
	friendly_name: string;
	status: ServiceStatus;
}
```

This reuses the `ServiceStatus` type instead of duplicating the union.

**Step 3 — Update imports in `api.ts`** (`src/lib/api.ts:1-34`)

Replace `AgentResponse` with `ServiceResponse` in the import block:

Before:
```typescript
import type {
	AgentCertificateSettings,
	AgentResponse,
	// ...
```

After:
```typescript
import type {
	AgentCertificateSettings,
	ServiceResponse,
	// ...
```

**Step 4 — Update Agents page** (`src/routes/agents/+page.svelte`)

Replace every occurrence of `AgentResponse` with `ServiceResponse`:

| Line | Before | After |
|------|--------|-------|
| 5 | `import type { AgentResponse } from '$lib/types';` | `import type { ServiceResponse } from '$lib/types';` |
| 7 | `let agents: AgentResponse[] = $state([]);` | `let agents: ServiceResponse[] = $state([]);` |
| 60 | `function openMergeDialog(agent: AgentResponse) {` | `function openMergeDialog(agent: ServiceResponse) {` |

Update `loadAgents()` to unpack the paginated response (line 31):

Before:
```typescript
agents = await getAgents();
```

After:
```typescript
const result = await getAgents();
agents = result.items;
```

Add handling for the `'deactivated'` status in the status badge template (line 160-166):

Before:
```svelte
{#if agent.status === 'pending'}
    <span class="badge preset-filled-warning-500">Pending</span>
{:else if agent.status === 'approved'}
    <span class="badge preset-filled-success-500">Approved</span>
{:else}
    <span class="badge preset-filled-error-500">Rejected</span>
{/if}
```

After:
```svelte
{#if agent.status === 'pending'}
    <span class="badge preset-filled-warning-500">Pending</span>
{:else if agent.status === 'approved'}
    <span class="badge preset-filled-success-500">Approved</span>
{:else if agent.status === 'deactivated'}
    <span class="badge preset-tonal">Deactivated</span>
{:else}
    <span class="badge preset-filled-error-500">Rejected</span>
{/if}
```

#### Dependencies

- None — this plan can be done first, and DFP-1 depends on it.

#### Verification

1. `npm run check` — no TypeScript errors (all references to the old `AgentResponse` are gone).
2. `grep -r "AgentResponse" src/` returns zero matches.
3. The agents table correctly renders the `deactivated` status badge.
4. The `client_version` field is available for future use (no UI change required yet).

---

### DFP-3: Add `?type=agent` filter to service list calls (F-3, Critical) — IMPLEMENTED

**Goal**: Ensure the Agents page only displays agents (not MQTT services) by always sending `?type=agent` when calling the list endpoint.

> **Note**: This plan is folded into DFP-1 Step 1. It is documented separately for traceability.

#### Files to modify

| File | Lines | Change |
|------|-------|--------|
| `src/lib/api.ts` | 194–197 | Add `type=agent` to `getAgents()` query params |

#### Step-by-step

**Step 1 — Always include `type=agent`**

This is already covered in DFP-1 Step 1. The new `getAgents()` uses `URLSearchParams` and always sets `type=agent`:

```typescript
export function getAgents(status?: string): Promise<PaginatedResponse<ServiceResponse>> {
	const params = new URLSearchParams();
	params.set('type', 'agent');
	if (status) params.set('status', status);
	return request(`/services?${params.toString()}`);
}
```

The backend's `ListServicesQuery` (in `crates/shared/web-api-types/src/services.rs:69-81`) accepts:
- `type` — `"agent"` or `"mqtt"` (optional, returns all types if omitted)
- `status` — `"pending"`, `"approved"`, `"rejected"`, `"deactivated"` (optional)
- `page` — page number, 1-indexed (optional, default 1)
- `per_page` — items per page (optional, default 20, max 1000)

Without `type=agent`, the endpoint returns both agents and MQTT services interleaved — making the Agents page show MQTT entries that don't belong there.

#### Dependencies

- Part of DFP-1. No additional dependencies.

#### Verification

1. With both agent and MQTT services in the database, navigate to the Agents page.
2. Verify the Network tab shows `GET /api/v1/services?type=agent`.
3. Verify no MQTT services appear in the table.

---

### DFP-4: Add MQTT enrollment token support (F-4, Critical) — IMPLEMENTED

**Goal**: The settings page currently manages a single "Enrollment Token" (implicitly for agents). The backend supports separate enrollment tokens for agents and MQTT services via the `?type=agent|mqtt` query parameter on the same `/services/enrollment-token` endpoints. The settings page must support both.

#### Backend context

The backend handler `enrollment_setting_key()` (`crates/ui/web-api/src/routes/services.rs:98-108`) routes the `type` parameter:
- `type=agent` (or omitted) → `SettingKey::EnrollmentTokenHash` (DB key: `agent_enrollment.token_hash`)
- `type=mqtt` → `SettingKey::MqttEnrollmentTokenHash` (DB key: `mqtt_enrollment.token_hash`)

All three endpoints (`GET …/status`, `POST`, `DELETE`) accept this parameter.

#### Files to modify

| File | Lines | Change |
|------|-------|--------|
| `src/lib/api.ts` | 272–282 | Add `type` parameter (already done in DFP-1 Step 2) |
| `src/routes/settings/+page.svelte` | ~130–140 (state), ~318–340 (handlers), ~748–777 (template) | Duplicate enrollment token state and UI for MQTT |

#### Step-by-step

**Step 1 — API functions already accept `type`**

Done in DFP-1 Step 2. The three enrollment-token functions now accept `type: 'agent' | 'mqtt' = 'agent'`.

**Step 2 — Add MQTT enrollment token state** (`src/routes/settings/+page.svelte`, state declarations ~line 130)

Add new state variables alongside the existing agent ones:

```typescript
// Existing agent enrollment state
let enrollmentConfigured: boolean = $state(false);
let generatedToken: string | null = $state(null);

// New MQTT enrollment state
let mqttEnrollmentConfigured: boolean = $state(false);
let mqttGeneratedToken: string | null = $state(null);
```

**Step 3 — Load MQTT enrollment status on mount** (inside the existing `$effect` or `onMount` that loads settings)

Add after the existing `getEnrollmentTokenStatus()` call:

```typescript
// Existing:
const enrollStatus = await getEnrollmentTokenStatus('agent');
enrollmentConfigured = enrollStatus.configured;

// Add:
const mqttEnrollStatus = await getEnrollmentTokenStatus('mqtt');
mqttEnrollmentConfigured = mqttEnrollStatus.configured;
```

**Step 4 — Add MQTT enrollment token handlers** (after existing `handleGenerateToken` / `handleRevokeToken`)

```typescript
async function handleGenerateMqttToken() {
    clearError();
    try {
        const res = await createEnrollmentToken('mqtt');
        mqttGeneratedToken = res.token;
        mqttEnrollmentConfigured = true;
        showSuccess('MQTT enrollment token generated.');
    } catch (e) {
        showError(e instanceof Error ? e.message : 'Failed to generate MQTT enrollment token');
    }
}

async function handleRevokeMqttToken() {
    clearError();
    try {
        await revokeEnrollmentToken('mqtt');
        mqttEnrollmentConfigured = false;
        mqttGeneratedToken = null;
        showSuccess('MQTT enrollment token revoked.');
    } catch (e) {
        showError(e instanceof Error ? e.message : 'Failed to revoke MQTT enrollment token');
    }
}
```

**Step 5 — Add MQTT enrollment token UI section** (after the existing "Enrollment Token" card, ~line 777)

Insert a new card that mirrors the existing enrollment token section:

```svelte
<!-- Section 5b: MQTT Enrollment Token -->
<div class="card mb-6 p-6">
    <h2 class="h3 mb-4">MQTT Enrollment Token</h2>
    <p class="mb-4 text-sm opacity-70">
        This token is used by MQTT services to register with the controller.
        It is separate from the agent enrollment token.
    </p>
    <div class="mb-4 flex items-center gap-3">
        <span>Status:</span>
        {#if mqttEnrollmentConfigured}
            <span class="badge preset-filled-success-500">Configured</span>
        {:else}
            <span class="badge preset-tonal">Not configured</span>
        {/if}
    </div>

    {#if mqttGeneratedToken}
        <aside class="mb-4 rounded-lg p-4 preset-filled-success-500">
            <p class="font-bold">Copy it now — it will not be shown again</p>
            <code class="break-all">{mqttGeneratedToken}</code>
        </aside>
    {/if}

    <div class="flex gap-2">
        <button class="btn preset-filled-primary-500" onclick={handleGenerateMqttToken}>
            {mqttEnrollmentConfigured ? 'Regenerate' : 'Generate'}
        </button>
        {#if mqttEnrollmentConfigured}
            <button class="btn preset-filled-error-500" onclick={handleRevokeMqttToken}>
                Revoke
            </button>
        {/if}
    </div>
</div>
```

**Step 6 — Update the existing enrollment token section title** (~line 750)

Change the heading from "Enrollment Token" to "Agent Enrollment Token" for clarity:

Before:
```svelte
<h2 class="h3 mb-4">Enrollment Token</h2>
```

After:
```svelte
<h2 class="h3 mb-4">Agent Enrollment Token</h2>
```

#### Dependencies

- **DFP-1** must be done first (enrollment-token API paths and `type` parameter).

#### Verification

1. `npm run check` — no TypeScript errors.
2. Settings page shows two separate enrollment token sections: "Agent Enrollment Token" and "MQTT Enrollment Token".
3. Generating an agent token sends `POST /api/v1/services/enrollment-token?type=agent`.
4. Generating an MQTT token sends `POST /api/v1/services/enrollment-token?type=mqtt`.
5. Each token's status, generate, and revoke work independently.
6. Backend database has separate setting keys: `agent_enrollment.token_hash` and `mqtt_enrollment.token_hash`.

---

### DFP-5: Move JWT tokens out of `localStorage` (F-5, High) — IMPLEMENTED

**Goal**: Eliminate XSS-based token theft by moving the `refresh_token` to an `HttpOnly` cookie (managed by the backend) and keeping the `access_token` only in memory (a module-level variable, not `localStorage`).

This is the most complex fix because it requires **coordinated frontend + backend changes**.

#### Current state (vulnerable)

Both tokens are stored in `localStorage`:

- `src/lib/auth.ts:27-28` — `localStorage.setItem('access_token', …)` and `localStorage.setItem('refresh_token', …)` in `handleLogin()`, `handleRegister()`, and all OIDC handlers.
- `src/lib/api.ts:53` — `authHeaders()` reads `localStorage.getItem('access_token')`.
- `src/lib/api.ts:60` — `refreshAccessToken()` reads `localStorage.getItem('refresh_token')`.
- `src/lib/api.ts:88-121` — refresh logic reads/writes both tokens in `localStorage`.

Any XSS vulnerability (e.g., a compromised dependency, the OIDC `logo_url` injection from F-6, or a future CSP gap) can steal both tokens via `localStorage.getItem()`.

#### Target architecture

```
┌──────────────────┐     ┌──────────────────────────────┐
│   Browser (SPA)  │     │       Backend (Rust)          │
│                  │     │                                │
│  access_token:   │     │  POST /auth/login → response: │
│   module-level   │◄────│    body: { access_token, … }  │
│   variable       │     │    Set-Cookie: refresh_token=… │
│                  │     │      HttpOnly; Secure;         │
│  refresh_token:  │     │      SameSite=Strict; Path=/   │
│   NOT stored     │────►│                                │
│   in JS at all   │     │  POST /auth/refresh →          │
│                  │     │    reads cookie automatically   │
│                  │     │    body: { access_token, … }   │
└──────────────────┘     └──────────────────────────────┘
```

#### Files to modify

**Frontend:**

| File | Lines | Change |
|------|-------|--------|
| `src/lib/auth.ts` | 9, 19, 26–28, 34–36, 43–44, 56–57, 64–65, 74–75 | Replace all `localStorage` usage with in-memory token |
| `src/lib/api.ts` | 52–55, 57–76, 88, 95, 115–116 | Read token from memory, drop refresh_token send |

**Backend** (separate PR recommended):

| File | Lines | Change |
|------|-------|--------|
| `crates/ui/web-api/src/routes/auth.rs` | login/register/OIDC handlers | Set `refresh_token` as `HttpOnly` cookie, remove it from response body |
| `crates/ui/web-api/src/routes/auth.rs` | `refresh` handler | Read `refresh_token` from cookie instead of request body |
| `crates/ui/web-api/src/routes/auth.rs` | `logout` handler | Clear the cookie via `Set-Cookie` with `Max-Age=0` |

#### Step-by-step (Frontend)

**Step 1 — Create in-memory token store** (`src/lib/auth.ts`, top of file)

Before:
```typescript
export const user = writable<User | null>(null);
export const loading = writable(true);
```

After:
```typescript
export const user = writable<User | null>(null);
export const loading = writable(true);

/** In-memory access token — intentionally NOT persisted to storage. */
let accessToken: string | null = null;

export function getAccessToken(): string | null {
    return accessToken;
}

export function setAccessToken(token: string | null): void {
    accessToken = token;
}
```

**Step 2 — Update `initialize()`** (`src/lib/auth.ts:8-23`)

Before:
```typescript
export async function initialize() {
    const token = localStorage.getItem('access_token');
    if (!token) {
        loading.set(false);
        return;
    }
    try {
        const u = await api.me();
        user.set(u);
    } catch {
        localStorage.removeItem('access_token');
        localStorage.removeItem('refresh_token');
    } finally {
        loading.set(false);
    }
}
```

After:
```typescript
export async function initialize() {
    // On page load, no access_token exists in memory.
    // Attempt a silent refresh via the HttpOnly cookie.
    try {
        const refreshed = await api.refreshAccessToken();
        accessToken = refreshed.access_token;
        const u = await api.me();
        user.set(u);
    } catch {
        accessToken = null;
    } finally {
        loading.set(false);
    }
}
```

Key change: instead of reading `localStorage`, the app attempts a `/auth/refresh` call. The browser automatically sends the `HttpOnly` cookie. If it succeeds, the access token is stored in memory. If it fails (no cookie, expired), the user is directed to log in.

**Step 3 — Update login/register/OIDC handlers** (`src/lib/auth.ts:25-77`)

Remove all `localStorage.setItem('refresh_token', …)` calls. Only store the access token in memory:

Before (e.g., `handleLogin`):
```typescript
export async function handleLogin(data: LoginRequest) {
    const res = await api.login(data);
    localStorage.setItem('access_token', res.access_token);
    localStorage.setItem('refresh_token', res.refresh_token);
    user.set(res.user);
}
```

After:
```typescript
export async function handleLogin(data: LoginRequest) {
    const res = await api.login(data);
    accessToken = res.access_token;
    // refresh_token is now set as HttpOnly cookie by the backend
    user.set(res.user);
}
```

Apply the same pattern to: `handleRegister`, `handleOidcCallback`, `handleOidcCompleteRegistration`, `handleOidcLink`.

**Step 4 — Update logout** (`src/lib/auth.ts:39-47`)

Before:
```typescript
export async function handleLogout() {
    try {
        await api.logout();
    } finally {
        localStorage.removeItem('access_token');
        localStorage.removeItem('refresh_token');
        user.set(null);
    }
}
```

After:
```typescript
export async function handleLogout() {
    try {
        await api.logout();
        // Backend clears the HttpOnly cookie via Set-Cookie: Max-Age=0
    } finally {
        accessToken = null;
        user.set(null);
    }
}
```

**Step 5 — Update `authHeaders()`** (`src/lib/api.ts:52-55`)

Before:
```typescript
function authHeaders(): Record<string, string> {
    const token = localStorage.getItem('access_token');
    return token ? { Authorization: `Bearer ${token}` } : {};
}
```

After:
```typescript
function authHeaders(): Record<string, string> {
    const token = getAccessToken();
    return token ? { Authorization: `Bearer ${token}` } : {};
}
```

Add `import { getAccessToken, setAccessToken } from './auth';` at the top of `api.ts`.

**Step 6 — Update `refreshAccessToken()`** (`src/lib/api.ts:59-76`)

Before:
```typescript
async function refreshAccessToken(): Promise<RefreshResponse> {
    const refreshToken = localStorage.getItem('refresh_token');
    if (!refreshToken) {
        throw new Error('No refresh token');
    }

    const res = await fetch(`${BASE}/auth/refresh`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ refresh_token: refreshToken })
    });
    // ...
}
```

After:
```typescript
export async function refreshAccessToken(): Promise<RefreshResponse> {
    const res = await fetch(`${BASE}/auth/refresh`, {
        method: 'POST',
        credentials: 'same-origin',    // sends the HttpOnly cookie
        headers: { 'Content-Type': 'application/json' }
        // No body — refresh_token comes from the cookie
    });

    if (!res.ok) {
        throw new Error('Refresh failed');
    }

    return res.json();
}
```

Key changes:
- No `localStorage` read.
- `credentials: 'same-origin'` ensures the browser includes the cookie.
- Empty body (the backend reads the token from the cookie header).
- Made `export` so `auth.ts:initialize()` can call it.

**Step 7 — Update the 401 refresh block** (`src/lib/api.ts:88-121`)

Before:
```typescript
if (res.status === 401 && localStorage.getItem('refresh_token')) {
    try {
        // ...
        localStorage.setItem('access_token', refreshed.access_token);
        // ...
    } catch {
        localStorage.removeItem('access_token');
        localStorage.removeItem('refresh_token');
        window.location.href = '/login';
        // ...
    }
}
```

After:
```typescript
if (res.status === 401) {
    try {
        if (!refreshPromise) {
            refreshPromise = refreshAccessToken();
        }
        const refreshed = await refreshPromise;
        setAccessToken(refreshed.access_token);

        // Retry original request with new token
        const retryRes = await fetch(`${BASE}${path}`, {
            headers: {
                'Content-Type': 'application/json',
                Authorization: `Bearer ${refreshed.access_token}`,
                ...(options.headers as Record<string, string> | undefined)
            },
            ...options
        });

        if (!retryRes.ok) {
            const message = await extractErrorMessage(retryRes);
            throw new Error(message);
        }
        if (retryRes.status === 204) return undefined as T;
        return retryRes.json();
    } catch {
        setAccessToken(null);
        window.location.href = '/login';
        throw new Error('Session expired');
    } finally {
        refreshPromise = null;
    }
}
```

Key changes:
- Removed `localStorage.getItem('refresh_token')` guard — we always attempt refresh on 401 (the cookie may or may not be present; the backend returns 401 if it's missing).
- Replaced `localStorage.setItem/removeItem` with `setAccessToken()`.

#### Step-by-step (Backend — outline only)

These backend changes should be done in a separate commit/PR:

1. **Login/register/OIDC handlers**: After generating tokens, set the refresh token as a cookie:
   ```rust
   let cookie = format!(
       "refresh_token={}; HttpOnly; Secure; SameSite=Strict; Path=/api/v1/auth; Max-Age={}",
       refresh_token, refresh_token_lifetime_secs
   );
   headers.insert(header::SET_COOKIE, cookie.parse().unwrap());
   ```
   Remove `refresh_token` from the JSON response body.

2. **Refresh handler**: Read the token from the cookie header instead of the JSON body:
   ```rust
   let refresh_token = req.cookies().get("refresh_token")
       .ok_or_else(|| error_response(StatusCode::UNAUTHORIZED, "No refresh token"))?;
   ```

3. **Logout handler**: Clear the cookie:
   ```rust
   let cookie = "refresh_token=; HttpOnly; Secure; SameSite=Strict; Path=/api/v1/auth; Max-Age=0";
   headers.insert(header::SET_COOKIE, cookie.parse().unwrap());
   ```

4. **Update `RefreshResponse` type**: Remove `refresh_token` field if it was present, or keep the struct as-is (it currently only has `access_token`, `expires_in`, `token_type` — already correct).

5. **Update `AuthResponse` type**: Remove `refresh_token` from the JSON body. The frontend type `AuthResponse` in `types.ts:22-28` should also drop the `refresh_token` field.

#### Dependencies

- **Backend changes are required first** — the frontend changes will break login if the backend still expects `refresh_token` in the JSON body for `/auth/refresh`.
- Recommended implementation order:
  1. Backend: add cookie-based refresh (support both cookie and body for backward compatibility).
  2. Frontend: switch to in-memory + cookie.
  3. Backend: remove body-based refresh support.

#### Verification

1. `npm run check` — no TypeScript errors.
2. `grep -r "localStorage" src/` returns zero matches (all storage removed).
3. Open DevTools → Application → Local Storage: no `access_token` or `refresh_token` keys.
4. Open DevTools → Application → Cookies: `refresh_token` cookie is present with `HttpOnly`, `Secure`, `SameSite=Strict`.
5. Login → verify `access_token` is in the response body but not in localStorage or cookies.
6. Refresh the page → the app silently re-authenticates via the cookie (no login redirect).
7. Logout → verify the `refresh_token` cookie is cleared.
8. Open a JavaScript console → `document.cookie` does not contain `refresh_token` (HttpOnly prevents JS access).
9. Simulate XSS: `localStorage.getItem('access_token')` returns `null` (token is not in storage).

---

### DFP-6: Validate OIDC `logo_url` — HTTPS-only (F-6, High)

**Goal**: Prevent XSS and SSRF vectors by validating that OIDC provider `logo_url` values are `https://` URLs before rendering them as `<img src>`.

#### Threat model

The `logo_url` is admin-configured and stored in the database. It is rendered on the login page for every visitor:

```svelte
<!-- src/routes/login/+page.svelte:246-248 -->
{#if provider.logo_url}
    <img src={provider.logo_url} alt="" class="h-5 w-5" />
{/if}
```

Risks:
- **SSRF**: A `logo_url` pointing to an internal IP (e.g., `http://192.168.1.1/admin`) causes every visitor's browser to make a GET request to that address.
- **Tracking**: A `logo_url` pointing to an attacker-controlled server leaks user IPs, `Referer` headers, and timing data.
- **Protocol confusion**: While `javascript:` URIs in `<img src>` are blocked by modern browsers, `data:` URIs could embed tracking pixels.

#### Files to modify

| File | Lines | Change |
|------|-------|--------|
| `src/routes/login/+page.svelte` | 246–248 | Add URL validation before rendering |
| `src/routes/settings/+page.svelte` | 803–804 | Add client-side validation on the OIDC logo_url input |
| `src/lib/utils.ts` | (new) | Create `isValidLogoUrl()` helper |

#### Step-by-step

**Step 1 — Create URL validation helper** (`src/lib/utils.ts`, new file or add to existing if created by other DFPs)

```typescript
/**
 * Validates that a URL is a safe logo URL (HTTPS only).
 * Returns true for valid https:// URLs, false for everything else.
 */
export function isValidLogoUrl(url: string | null | undefined): boolean {
    if (!url) return false;
    try {
        const parsed = new URL(url);
        return parsed.protocol === 'https:';
    } catch {
        return false;
    }
}
```

Using `new URL()` for parsing handles edge cases like `https:///evil.com`, `HTTPS://`, whitespace padding, etc.

**Step 2 — Guard the `<img>` on the login page** (`src/routes/login/+page.svelte:246-248`)

Before:
```svelte
{#if provider.logo_url}
    <img src={provider.logo_url} alt="" class="h-5 w-5" />
{/if}
```

After:
```svelte
{#if isValidLogoUrl(provider.logo_url)}
    <img src={provider.logo_url} alt="" class="h-5 w-5" />
{/if}
```

Add `import { isValidLogoUrl } from '$lib/utils';` to the `<script>` block.

**Step 3 — Add client-side validation to the OIDC form** (`src/routes/settings/+page.svelte:803-804`)

Before:
```svelte
<label class="label">
    <span>Logo URL</span>
    <input class="input" type="text" placeholder="https://..." bind:value={oidcForm.logo_url} />
</label>
```

After:
```svelte
<label class="label">
    <span>Logo URL</span>
    <input class="input" type="url" placeholder="https://..." bind:value={oidcForm.logo_url} />
    {#if oidcForm.logo_url && !isValidLogoUrl(oidcForm.logo_url)}
        <small class="text-error-500">Logo URL must use HTTPS</small>
    {/if}
</label>
```

Also add `import { isValidLogoUrl } from '$lib/utils';` to the settings page script.

**Step 4 — (Optional) Add `referrerpolicy` to the img tag** for defense-in-depth:

```svelte
<img src={provider.logo_url} alt="" class="h-5 w-5" referrerpolicy="no-referrer" />
```

This prevents the browser from sending a `Referer` header to the logo server, even for legitimate HTTPS logos.

#### Dependencies

- None. This plan is standalone.

#### Verification

1. `npm run check` — no TypeScript errors.
2. Set an OIDC provider's `logo_url` to `http://example.com/logo.png` → logo does not render on the login page.
3. Set `logo_url` to `javascript:alert(1)` → logo does not render.
4. Set `logo_url` to `data:image/svg+xml,...` → logo does not render.
5. Set `logo_url` to `https://example.com/logo.png` → logo renders normally.
6. In the settings OIDC form, enter a non-HTTPS URL → validation message appears.

---

### DFP-7: Type `User.permissions` as `Permission[]` (F-19, High) — IMPLEMENTED

**Goal**: Change `User.permissions` from `string[]` to `Permission[]` so that permission checks are type-safe and typos are caught at compile time.

#### Files to modify

| File | Lines | Change |
|------|-------|--------|
| `src/lib/types.ts` | 19 | Change `permissions: string[]` to `permissions: Permission[]` |

That's it — one line change. All existing call sites already use `Permission.ManageSettings` etc. with `.includes()`, so they will work without modification because `Permission` is a `string` enum.

#### Step-by-step

**Step 1 — Update the type** (`src/lib/types.ts:14-20`)

Before:
```typescript
export interface User {
	id: string;
	email: string;
	first_name: string;
	last_name: string;
	permissions: string[];
}
```

After:
```typescript
export interface User {
	id: string;
	email: string;
	first_name: string;
	last_name: string;
	permissions: Permission[];
}
```

#### Why this is safe

The `Permission` enum in `types.ts:6-12` is:
```typescript
export enum Permission {
	ViewSettings = 'view_settings',
	ManageSettings = 'manage_settings',
	ViewAgents = 'view_agents',
	ManageAgents = 'manage_agents',
	ManageGlobalSettings = 'manage_global_settings'
}
```

All 5 usage sites already pass `Permission.*` enum members to `.includes()`:
- `+layout.svelte:45` — `$user?.permissions.includes(Permission.ManageGlobalSettings)`
- `+layout.svelte:56` — `item.permission` (typed as `Permission`)
- `settings/+page.svelte:108` — `$user?.permissions.includes(Permission.ManageSettings)`
- `settings/global/+page.svelte:32` — `$user?.permissions.includes(Permission.ManageGlobalSettings)`
- `hosts/+page.svelte:106` — `$user?.permissions.includes(Permission.ManageAgents)`

TypeScript allows `Permission[].includes(Permission.X)` — the call is already type-correct. What changes is that code like `$user.permissions.includes('manage_settings')` (raw string) would now produce a type error, which is the desired outcome.

**Important caveat**: The backend might return permission strings that are not in the frontend enum (e.g., if a new permission is added server-side before the frontend is updated). With `Permission[]`, the JSON deserialization still works at runtime (TypeScript types are erased), but TypeScript will consider unknown permission strings as type errors if accessed directly. This is acceptable — unknown permissions are simply not checked in the UI.

#### Dependencies

- None. This is a one-line, zero-risk change.

#### Verification

1. `npm run check` — no TypeScript errors.
2. Try adding `$user?.permissions.includes('typo_permission')` anywhere → TypeScript produces an error (proves the type guard is working).
3. App behaves identically at runtime (no functional change).

---

### DFP-8: Use typed interfaces for MQTT form data (F-20, High) — IMPLEMENTED

**Goal**: Replace `Record<string, unknown>` with properly typed `CreateMqttClient` and `UpdateMqttClient` interfaces in the MQTT client form handler, restoring TypeScript's type checking.

#### Current state (unsafe)

In `src/routes/settings/+page.svelte:206-244`, the `saveMqttClient()` function builds untyped objects:

```typescript
// Update path (line 210-220)
const data: Record<string, unknown> = {
    url: mqttForm.url || undefined,
    enabled: mqttForm.enabled,
    client_id: mqttForm.client_id,
    username: mqttForm.username || null,
    topic_prefix: mqttForm.topic_prefix
};
if (mqttForm.password) {
    data.password = mqttForm.password;
}
const res = await updateMqttClient(editingMqttClient.id, data);

// Create path (line 228-236)
const data: Record<string, unknown> = {
    url: mqttForm.url,
    enabled: mqttForm.enabled,
    client_id: mqttForm.client_id || undefined,
    username: mqttForm.username || undefined,
    password: mqttForm.password || undefined,
    topic_prefix: mqttForm.topic_prefix || undefined
};
const res = await createMqttClient(data);
```

The `updateMqttClient()` signature expects `UpdateMqttClient` and `createMqttClient()` expects `CreateMqttClient`, but `Record<string, unknown>` is assignable to both (TypeScript structural typing). This bypasses all type checking — a misspelled field name, wrong type, or missing required field would not be caught.

#### Files to modify

| File | Lines | Change |
|------|-------|--------|
| `src/routes/settings/+page.svelte` | 206–244 | Replace `Record<string, unknown>` with typed interfaces |

#### Step-by-step

**Step 1 — Replace the update path** (`src/routes/settings/+page.svelte:209-220`)

Before:
```typescript
if (editingMqttClient) {
    const data: Record<string, unknown> = {
        url: mqttForm.url || undefined,
        enabled: mqttForm.enabled,
        client_id: mqttForm.client_id,
        username: mqttForm.username || null,
        topic_prefix: mqttForm.topic_prefix
    };
    if (mqttForm.password) {
        data.password = mqttForm.password;
    }
    const res = await updateMqttClient(editingMqttClient.id, data);
```

After:
```typescript
if (editingMqttClient) {
    const data: UpdateMqttClient = {
        url: mqttForm.url || undefined,
        enabled: mqttForm.enabled,
        client_id: mqttForm.client_id,
        username: mqttForm.username || null,
        topic_prefix: mqttForm.topic_prefix,
        ...(mqttForm.password ? { password: mqttForm.password } : {})
    };
    const res = await updateMqttClient(editingMqttClient.id, data);
```

Key changes:
- `Record<string, unknown>` → `UpdateMqttClient` (from `$lib/types`).
- The conditional `password` assignment is replaced with a spread expression so the object is constructed in a single statement.

**Step 2 — Replace the create path** (`src/routes/settings/+page.svelte:228-236`)

Before:
```typescript
const data: Record<string, unknown> = {
    url: mqttForm.url,
    enabled: mqttForm.enabled,
    client_id: mqttForm.client_id || undefined,
    username: mqttForm.username || undefined,
    password: mqttForm.password || undefined,
    topic_prefix: mqttForm.topic_prefix || undefined
};
const res = await createMqttClient(data);
```

After:
```typescript
const data: CreateMqttClient = {
    url: mqttForm.url,
    enabled: mqttForm.enabled,
    client_id: mqttForm.client_id || undefined,
    username: mqttForm.username || undefined,
    password: mqttForm.password || undefined,
    topic_prefix: mqttForm.topic_prefix || undefined
};
const res = await createMqttClient(data);
```

Key change: `Record<string, unknown>` → `CreateMqttClient`.

**Step 3 — Add imports** (`src/routes/settings/+page.svelte:25-35`)

Add `UpdateMqttClient` and `CreateMqttClient` to the existing imports from `$lib/types`:

Before:
```typescript
import {
    Permission,
    type RegistrationSettings,
    type AuthenticationSettings,
    type AgentCertificateSettings,
    type EnrollmentTokenStatus,
    type OidcProviderResponse,
    type CreateOidcProviderRequest,
    type UpdateOidcProviderRequest,
    type MqttClientResponse
} from '$lib/types';
```

After:
```typescript
import {
    Permission,
    type RegistrationSettings,
    type AuthenticationSettings,
    type AgentCertificateSettings,
    type CreateMqttClient,
    type EnrollmentTokenStatus,
    type MqttClientResponse,
    type OidcProviderResponse,
    type CreateOidcProviderRequest,
    type UpdateMqttClient,
    type UpdateOidcProviderRequest
} from '$lib/types';
```

#### Dependencies

- None. The types `CreateMqttClient` and `UpdateMqttClient` already exist in `types.ts:200-222`.

#### Verification

1. `npm run check` — no TypeScript errors.
2. `grep -n "Record<string, unknown>" src/routes/settings/+page.svelte` returns zero matches.
3. Try adding a misspelled field (e.g., `enbled: true`) to the data object → TypeScript produces an error.
4. MQTT client create and edit flows work identically in the browser.

---

### DFP-9: Extract settings page into sub-components (F-11, High)

**Goal**: Break the 952-line `settings/+page.svelte` monolith into focused sub-components, each managing a single settings section.

#### Current structure

The settings page (`src/routes/settings/+page.svelte`) contains 6 distinct sections, each with independent state, handlers, and templates:

| Section | State vars | Handlers | Template lines |
|---------|-----------|----------|----------------|
| Registration | `regMode`, `regToken`, `regRequireTokenForOidc` | `saveRegistration` | 508–542 |
| Authentication | `passwordAuthEnabled` | `saveAuthentication` | 544–554 |
| MQTT Clients | `mqttClients`, `showMqttModal`, `editingMqttClient`, `mqttForm`, `mqttDeleteConfirm` | `openCreateMqtt`, `openEditMqtt`, `closeMqttModal`, `saveMqttClient`, `requestDeleteMqtt`, `executeDeleteMqtt` | 556–641 + 863–924 (modal) |
| OIDC Providers | `oidcProviders`, `showOidcModal`, `editingProvider`, `oidcForm`, `slugTouched`, `deleteConfirm` | `openCreateOidc`, `openEditOidc`, `closeOidcModal`, `saveOidcProvider`, `requestDeleteOidc`, `executeDeleteOidc`, `toggleOidcActive`, `onOidcNameInput`, `slugify` | 643–714 + 780–861 (modal) + 926–950 (delete confirm) |
| Agent Certificates | `certLifetimeDays`, `certRenewalWindowHours` | `saveCertificates` | 716–746 |
| Enrollment Token | `enrollmentConfigured`, `generatedToken` | `handleGenerateToken`, `handleRevokeToken` | 748–777 |

Total: ~30 state variables, ~20 handler functions, 6 modals/confirms, all in one component.

#### Target structure

```text
src/routes/settings/
├── +page.svelte                 # Thin shell: auth guard, layout, notification display
├── RegistrationSettings.svelte  # Registration mode + token
├── AuthenticationSettings.svelte # Password auth toggle
├── MqttClientsSettings.svelte   # MQTT client CRUD + modal + delete confirm
├── AgentCertificateSettings.svelte # Certificate lifetime + renewal window
├── EnrollmentTokenSettings.svelte  # Enrollment token status + generate/revoke
└── OidcProvidersSettings.svelte # OIDC provider CRUD + modal + delete confirm
```

#### Files to modify/create

| File | Action | Description |
|------|--------|-------------|
| `src/routes/settings/+page.svelte` | Rewrite | Keep auth guard, notification system, and data loading. Render child components. |
| `src/routes/settings/RegistrationSettings.svelte` | Create | Extract lines 42–44 (state), 264–283 (handler), 508–542 (template) |
| `src/routes/settings/AuthenticationSettings.svelte` | Create | Extract line 46 (state), 285–297 (handler), 544–554 (template) |
| `src/routes/settings/MqttClientsSettings.svelte` | Create | Extract lines 56–68 (state), 174–261 (handlers), 556–641 + 863–924 (template) |
| `src/routes/settings/AgentCertificateSettings.svelte` | Create | Extract lines 48–49 (state), 299–315 (handler), 716–746 (template) |
| `src/routes/settings/EnrollmentTokenSettings.svelte` | Create | Extract lines 51–52 (state), 317–340 (handlers), 748–777 (template) |
| `src/routes/settings/OidcProvidersSettings.svelte` | Create | Extract lines 54, 70–103 (state), 342–475 (handlers), 643–714 + 780–861 + 926–950 (template) |

#### Step-by-step

**Step 1 — Define the component interface pattern**

Each child component receives callbacks for success/error notifications and loads its own initial data. Use Svelte 5 props:

```svelte
<!-- Example: RegistrationSettings.svelte -->
<script lang="ts">
    import { getRegistrationSettings, updateRegistrationSettings } from '$lib/api';
    import type { RegistrationSettings } from '$lib/types';

    let { onSuccess, onError }: {
        onSuccess: (msg: string) => void;
        onError: (msg: string) => void;
    } = $props();

    let regMode: 'open' | 'invite' | 'closed' = $state('open');
    let regToken: string = $state('');
    let regRequireTokenForOidc: boolean = $state(false);

    export async function load() {
        const settings = await getRegistrationSettings();
        regMode = settings.mode;
        regRequireTokenForOidc = settings.require_token_for_oidc;
    }

    async function saveRegistration() {
        try {
            const data = { mode: regMode } as any;
            if (regMode === 'invite' && regToken) data.token = regToken;
            if (regMode === 'invite') data.require_token_for_oidc = regRequireTokenForOidc;
            const res = await updateRegistrationSettings(data);
            regMode = res.mode;
            regRequireTokenForOidc = res.require_token_for_oidc;
            regToken = '';
            onSuccess('Registration settings saved.');
        } catch (e) {
            onError(e instanceof Error ? e.message : 'Failed to save registration settings');
        }
    }
</script>

<!-- Template: same as current lines 508-542, unchanged -->
```

**Step 2 — Rewrite the parent shell** (`src/routes/settings/+page.svelte`)

The parent page shrinks to ~80 lines:

```svelte
<script lang="ts">
    import { user } from '$lib/auth';
    import { goto } from '$app/navigation';
    import { Permission } from '$lib/types';
    import RegistrationSettings from './RegistrationSettings.svelte';
    import AuthenticationSettings from './AuthenticationSettings.svelte';
    import MqttClientsSettings from './MqttClientsSettings.svelte';
    import AgentCertificateSettings from './AgentCertificateSettings.svelte';
    import EnrollmentTokenSettings from './EnrollmentTokenSettings.svelte';
    import OidcProvidersSettings from './OidcProvidersSettings.svelte';

    let successMessage: string | null = $state(null);
    let errorMessage: string | null = $state(null);
    let loading: boolean = $state(true);

    const canManageSettings = $derived(
        $user?.permissions.includes(Permission.ManageSettings) ?? false
    );

    $effect(() => {
        if (!$user) goto('/login');
        else if (!canManageSettings) goto('/');
    });

    function showSuccess(msg: string) {
        successMessage = msg;
        setTimeout(() => { successMessage = null; }, 3000);
    }

    function showError(msg: string) {
        errorMessage = msg;
    }

    function clearError() {
        errorMessage = null;
    }

    // Each child component loads its own data independently
</script>

{#if $user && canManageSettings}
    <h1 class="h1 mb-6">Settings</h1>

    {#if successMessage}
        <aside class="mb-4 rounded-lg p-4 preset-filled-success-500">
            <p>{successMessage}</p>
        </aside>
    {/if}

    {#if errorMessage}
        <aside class="mb-4 flex items-center justify-between rounded-lg p-4 preset-filled-error-500">
            <p>{errorMessage}</p>
            <button class="btn btn-sm preset-filled" onclick={clearError}>Dismiss</button>
        </aside>
    {/if}

    <RegistrationSettings onSuccess={showSuccess} onError={showError} />
    <AuthenticationSettings onSuccess={showSuccess} onError={showError} />
    <MqttClientsSettings onSuccess={showSuccess} onError={showError} />
    <OidcProvidersSettings onSuccess={showSuccess} onError={showError} />
    <AgentCertificateSettings onSuccess={showSuccess} onError={showError} />
    <EnrollmentTokenSettings onSuccess={showSuccess} onError={showError} />
{/if}
```

**Step 3 — Move each section's code into its component**

For each new component file:
1. Move the relevant state variables from the parent `<script>` block.
2. Move the relevant handler functions.
3. Move the relevant template section (the `<div class="card mb-6 p-6">` block and any associated modals).
4. Add `onSuccess` and `onError` props to replace the parent's `showSuccess()` and `showError()` calls.
5. Add the necessary imports (`$lib/api`, `$lib/types`).
6. Each component loads its own data in an `$effect` or `onMount`.

The largest components are:
- **OidcProvidersSettings.svelte** (~280 lines): state (lines 54, 70–103), handlers (lines 342–475), template (lines 643–714 + 780–861 + 926–950)
- **MqttClientsSettings.svelte** (~230 lines): state (lines 56–68), handlers (lines 174–261), template (lines 556–641 + 863–924)

The smallest are:
- **AuthenticationSettings.svelte** (~40 lines)
- **RegistrationSettings.svelte** (~60 lines)

**Step 4 — Remove the monolithic `loadAllSettings()` function**

The current parent uses `Promise.allSettled()` to load all 6 data sources at once. After extraction, each child component loads its own data independently. This is slightly less efficient (6 sequential waterfalls vs. 1 parallel batch), but the trade-off is worth it for component isolation.

To preserve the parallel loading optimization, you can instead:
1. Keep `loadAllSettings()` in the parent.
2. Pass loaded data as props to each child.
3. Use `bind:this` with exported `setData()` methods.

The simpler approach (each child loads its own) is recommended first.

#### Dependencies

- None directly. However, if DFP-4 (MQTT enrollment tokens) and DFP-8 (typed MQTT form) are done first, the extraction will include those changes automatically.
- Recommended order: do DFP-8 first, then DFP-9.

#### Verification

1. `npm run check` — no TypeScript errors.
2. `wc -l src/routes/settings/+page.svelte` shows ~80 lines (down from 952).
3. Each sub-component file is under 300 lines.
4. All settings pages function identically: Registration, Authentication, MQTT, OIDC, Certificates, Enrollment Token.
5. Success/error notifications still display at the top of the page (managed by the parent).
6. Auth guard still works (unauthorized users are redirected).
7. Escape key still closes open modals.

---

### DFP-10: Centralize auth guards in layout (F-12, High)

**Goal**: Replace the duplicated `$effect(() => { if (!$user) goto('/login') })` pattern across 5 pages with a single auth guard in the root layout, eliminating the flash-of-content bug and preventing new pages from accidentally omitting the guard.

#### Current state (duplicated guards)

Every protected page independently implements auth checking:

| File | Lines | Guard |
|------|-------|-------|
| `src/routes/+page.svelte` | 5–9 | `if (!$user) goto('/login')` |
| `src/routes/agents/+page.svelte` | 16–20 | `if (!$user) goto('/login')` |
| `src/routes/hosts/+page.svelte` | 15–19 | `if (!$user) goto('/login')` |
| `src/routes/settings/+page.svelte` | 110–116 | `if (!$user) goto('/login')` + `if (!canManageSettings) goto('/')` |
| `src/routes/settings/global/+page.svelte` | 34–39 | `if (!$user) goto('/login')` + `if (!canManageGlobalSettings) goto('/')` |

Problems:
1. A new page that forgets the guard is unprotected.
2. The component renders briefly before the `$effect` fires, causing a flash of content.
3. Five copies of the same logic to maintain.

#### Target architecture

The root layout (`+layout.svelte`) already knows the current route and whether the user is authenticated. It already defines `publicRoutes`:

```typescript
// src/routes/+layout.svelte:50
const publicRoutes = new Set(['/login', '/register', '/device']);
```

The fix moves the guard into the layout so it runs before any page component renders.

#### Files to modify

| File | Lines | Change |
|------|-------|--------|
| `src/routes/+layout.svelte` | 39–67 | Add auth redirect logic after `initialize()` |
| `src/routes/+page.svelte` | 5–9 | Remove auth guard `$effect` |
| `src/routes/agents/+page.svelte` | 16–20 | Remove auth guard `$effect` |
| `src/routes/hosts/+page.svelte` | 15–19 | Remove auth guard `$effect` |
| `src/routes/settings/+page.svelte` | 110–116 | Remove auth guard `$effect`, keep permission check |
| `src/routes/settings/global/+page.svelte` | 34–39 | Remove auth guard `$effect`, keep permission check |

#### Step-by-step

**Step 1 — Add centralized auth guard to root layout** (`src/routes/+layout.svelte`)

Add a reactive `$effect` in the layout that runs after `initialize()` completes (i.e., after `$loading` becomes `false`):

After the existing `onMount` block (line 42), add:

```typescript
$effect(() => {
    if ($loading) return; // Wait for auth initialization

    const path = $page.url.pathname;
    const isPublic = publicRoutes.has(path);

    if (!$user && !isPublic) {
        goto('/login?redirect=' + encodeURIComponent(path + $page.url.search));
    }
});
```

Key behaviors:
- Waits for `initialize()` to finish (the `$loading` guard).
- Skips public routes (`/login`, `/register`, `/device`).
- Includes the current path as a `redirect` parameter so the user returns to where they were after login.

**Step 2 — Update the layout template to hide protected content before auth**

The layout already has `{#if $loading}…{:else}…{/if}`. Inside the `{:else}` branch, add an additional check before rendering `{@render children()}`:

Before (`+layout.svelte:155-157`):
```svelte
<div class="container mx-auto max-w-2xl p-4">
    {@render children()}
</div>
```

After:
```svelte
<div class="container mx-auto max-w-2xl p-4">
    {#if $user || publicRoutes.has($page.url.pathname)}
        {@render children()}
    {/if}
</div>
```

This prevents the flash-of-content by not rendering the page component until the user is confirmed authenticated (or the route is public).

**Step 3 — Remove auth guards from individual pages**

**`src/routes/+page.svelte`** — Remove the entire `$effect` block:

Before:
```svelte
<script lang="ts">
    import { user } from '$lib/auth';
    import { goto } from '$app/navigation';

    $effect(() => {
        if (!$user) {
            goto('/login');
        }
    });
</script>
```

After:
```svelte
<script lang="ts">
    import { user } from '$lib/auth';
</script>
```

The `goto` import is no longer needed (unless used elsewhere in the file).

**`src/routes/agents/+page.svelte:16-20`** — Remove:

```svelte
$effect(() => {
    if (!$user) {
        goto('/login');
    }
});
```

Also remove the `import { goto } from '$app/navigation';` line if `goto` is no longer used elsewhere in the file. In this case, `goto` is not used anywhere else, so it can be removed.

**`src/routes/hosts/+page.svelte:15-19`** — Remove the same pattern. Remove `goto` import.

**`src/routes/settings/+page.svelte:110-116`** — Simplify the auth `$effect`:

Before:
```svelte
$effect(() => {
    if (!$user) {
        goto('/login');
    } else if (!canManageSettings) {
        goto('/');
    }
});
```

After:
```svelte
$effect(() => {
    if ($user && !canManageSettings) {
        goto('/');
    }
});
```

The `!$user` → `/login` redirect is now handled by the layout. This `$effect` only needs to handle the **permission** check. The `goto` import remains because it's still used for the permission redirect.

**`src/routes/settings/global/+page.svelte:34-39`** — Same simplification:

Before:
```svelte
$effect(() => {
    if (!$user) {
        goto('/login');
    } else if (!canManageGlobalSettings) {
        goto('/');
    }
});
```

After:
```svelte
$effect(() => {
    if ($user && !canManageGlobalSettings) {
        goto('/');
    }
});
```

**Step 4 — Handle the redirect parameter on the login page**

After successful login in `src/routes/login/+page.svelte`, check for a `redirect` parameter:

In `onSubmit()` (line 87-96):

Before:
```typescript
await handleLogin({ email, password });
goto('/');
```

After:
```typescript
await handleLogin({ email, password });
const redirect = $page.url.searchParams.get('redirect');
goto(redirect && redirect.startsWith('/') && !redirect.startsWith('//') ? redirect : '/');
```

The validation (`startsWith('/')` and `!startsWith('//')`) prevents open redirect attacks.

Apply the same pattern to `onOidcLogin` and `onSubmitRegistrationToken` success paths.

#### Dependencies

- None. This plan is standalone.
- Can be combined with DFP-9 (settings refactor) — the settings page's auth guard simplification applies regardless.

#### Verification

1. `npm run check` — no TypeScript errors.
2. `grep -rn "if (!\\$user)" src/routes/` returns only the layout file (no individual pages).
3. Navigate to `/agents` while logged out → redirected to `/login?redirect=%2Fagents`.
4. After logging in → redirected back to `/agents` (not `/`).
5. Navigate to `/settings` without `ManageSettings` permission → redirected to `/`.
6. Navigate to `/login` while logged in → redirected to `/`.
7. No flash of page content before redirect on any protected route.
8. Public routes (`/login`, `/register`, `/device`) remain accessible without authentication.
