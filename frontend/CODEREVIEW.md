# Frontend Code Review

**Scope**: `frontend/` — SvelteKit SPA (Skeleton UI v4 + Tailwind CSS v4)
**Reviewer**: Claude Opus 4.6
**Date**: 2026-02-08
**Files reviewed**: All 14 source files (4 lib modules, 8 route components, 2 config/style files)

---

## Executive Summary

The frontend is a compact, well-structured SvelteKit SPA for the Uptrakit controller dashboard. It handles authentication (password + OIDC), device authorization, agent/host management, and system settings. The codebase uses modern Svelte 5 runes and Skeleton UI v4 consistently.

However, the review identifies **28 findings** across 5 categories, including **critical API path mismatches** that would prevent the frontend from functioning against the current backend, **security concerns** around token storage and XSS vectors, and significant **architectural debt** from duplicated patterns across pages.

### Severity Distribution

| Severity | Count |
|----------|-------|
| Critical | 4 |
| High | 8 |
| Medium | 10 |
| Low | 6 |

---

## Category 1: Critical — API Contract Mismatches

### F-1: Frontend uses `/agents` endpoints but backend serves `/services` (Critical)

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

**Fix plan (FP-1)**: Rename all `/agents` API paths to `/services` in `api.ts`. Update `getAgents()` to pass `?type=agent` filter. The backend `ListServicesQuery` supports `type` and `status` query params.

### F-2: `AgentResponse` type does not match `ServiceResponse` from backend (Critical)

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

**Fix plan (FP-2)**: Rename to `ServiceResponse`, add missing fields, and update all consuming components. Consider keeping a `type AgentResponse = ServiceResponse` alias during migration.

### F-3: Agents page does not filter by service type (Critical)

**File**: `src/routes/agents/+page.svelte:31`, `src/lib/api.ts:194-197`

`getAgents()` calls the list endpoint without a `type=agent` filter. Once the path is fixed to `/services`, this will return both agents AND MQTT services mixed together, which is incorrect for the "Agents" page.

**Fix plan (FP-3)**: Pass `?type=agent` as a default filter in `getAgents()`. Consider adding an MQTT Services page as well.

### F-4: Missing MQTT enrollment token endpoints (Critical)

**File**: `src/lib/api.ts`

The settings page manages agent enrollment tokens but has no support for MQTT enrollment tokens. Per AGENTS.md, MQTT enrollment tokens use `?type=mqtt` on the same `/services/enrollment-token` endpoints. The settings page should distinguish between agent and MQTT enrollment tokens.

**Fix plan (FP-4)**: Add `type` parameter to enrollment token API functions. Add MQTT enrollment token section to the settings page.

---

## Category 2: Security & Safety

### F-5: JWT tokens stored in `localStorage` — vulnerable to XSS (High)

**Files**: `src/lib/auth.ts:27-28`, `src/lib/api.ts:53`

Both `access_token` and `refresh_token` are stored in `localStorage`. Any XSS vulnerability (including from third-party dependencies or OIDC provider `logo_url` injection) gives an attacker full access to steal both tokens.

`localStorage` is synchronous, accessible from any script in the same origin, and persists indefinitely. The `refresh_token` is particularly dangerous because it can mint new access tokens.

**Fix plan (FP-5)**: Move `refresh_token` to an `HttpOnly`, `Secure`, `SameSite=Strict` cookie managed by the backend. The `access_token` can remain in memory (not `localStorage`) — store it in a module-level variable that is lost on page refresh. On refresh, the `HttpOnly` cookie silently provides a new access token via the refresh endpoint.

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

### F-19: `User.permissions` typed as `string[]` instead of `Permission[]` (High)

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

**Fix plan (FP-19)**: Change `permissions: string[]` to `permissions: Permission[]`.

### F-20: MQTT form uses `Record<string, unknown>` instead of typed interface (High)

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

**Fix plan (FP-20)**: Build a properly typed `UpdateMqttClient` object directly. Use conditional spreading for optional fields.

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

| ID | Severity | Effort | Description |
|----|----------|--------|-------------|
| FP-1 | Critical | Small | Fix API paths: `/agents` → `/services` |
| FP-2 | Critical | Small | Align `AgentResponse` type with backend `ServiceResponse` |
| FP-3 | Critical | Small | Add `?type=agent` filter to service list calls |
| FP-4 | Critical | Medium | Add MQTT enrollment token support |
| FP-5 | High | Large | Move refresh token to HttpOnly cookie |
| FP-6 | High | Small | Validate OIDC logo URLs (https-only) |
| FP-7 | Medium | Small | Validate redirect parameter, validate device code format |
| FP-8 | Medium | Small | Add Content Security Policy |
| FP-9 | Medium | Small | Add copy-to-clipboard for enrollment token |
| FP-10 | Low | Small | Preserve redirect path on session expiry |
| FP-11 | High | Medium | Extract settings page into sub-components |
| FP-12 | High | Medium | Centralize auth guards in layout |
| FP-13 | High | Medium | Extract shared menu/modal/confirm components |
| FP-14 | Medium | Small | Create shared notification system |
| FP-15 | Medium | Small | Use `onMount` for data fetching, not `$effect` |
| FP-16 | Medium | Small | Add loading states to action buttons |
| FP-17 | Medium | Medium | Add pagination controls to hosts page |
| FP-18 | Medium | Medium | Fix agents pagination support |
| FP-19 | High | Small | Type `User.permissions` as `Permission[]` |
| FP-20 | High | Small | Use typed interfaces for MQTT form data |
| FP-21 | Medium | Small | Fix refresh token race condition |
| FP-22 | Low | Small | Fix 204 response type handling |
| FP-23 | Low | Small | Remove unused `OidcCompleteRegistrationRequest` or use it |
| FP-24 | High | Medium | Add ARIA roles to modals |
| FP-25 | Medium | Medium | Implement focus trapping in modals |
| FP-26 | Medium | Medium | Use robust menu positioning |
| FP-27 | Low | Small | Add helpful empty states |
| FP-28 | Low | Small | Deduplicate theme initialization logic |

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
