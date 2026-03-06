# Frontend Code Review

**Scope:** SvelteKit 2 SPA — Svelte 5, Skeleton UI v4, Tailwind CSS v4
**Branch:** `docs/codereview-frontend`
**Date:** 2026-03-06

---

## Table of Contents

- [1. Architecture](#1-architecture)
- [2. Security and Safety](#2-security-and-safety)
- [3. Code Quality](#3-code-quality)
- [4. Tests and Coverage](#4-tests-and-coverage)
- [5. High Availability and Resilience](#5-high-availability-and-resilience)
- [6. Coding Standards](#6-coding-standards)
- [7. Extensibility](#7-extensibility)
- [8. Code and Logic Consistency](#8-code-and-logic-consistency)
- [9. Maintainability](#9-maintainability)

---

## 1. Architecture

### Strengths

- The `adapter-static` / CSR-only configuration (`ssr = false`, `prerender = false`, `fallback: 'index.html'`) is the correct choice for an authenticated dashboard SPA.
- Route nesting is shallow and maps cleanly to the product's information architecture: `hosts/`, `hosts/[id]/`, `hosts/[id]/packages/`, `software/`, `software/[id]/`, `settings/`, `settings/global/`, `extensions/[id]/`, etc.
- The lib split is well-reasoned: `api.ts`, `auth.svelte.ts`, `utils.ts`, `sse.ts`, `notifications.svelte.ts`, `extensions.svelte.ts`, `theme.svelte.ts`, `stores/events.svelte.ts`, `stores/network.svelte.ts` each have a single, clearly-named responsibility.
- All authenticated HTTP traffic flows through `authenticatedFetch` in `api.ts`. No route file bypasses the centralized client.
- Token refresh deduplication via `refreshPromise` is correctly implemented. A single in-flight refresh is shared across concurrent 401 requests.
- `AbortSignal.any([callerSignal, AbortSignal.timeout(30_000)])` provides defence-in-depth: callers can cancel independently while a hard 30-second ceiling prevents hung connections.
- The CSP configuration in `svelte.config.js` is well-considered: hash mode, `object-src: none`, `base-uri: self`, `form-action: self`. The `vite.config.ts` disables the modulePreload polyfill to avoid `blob:` URLs weakening `script-src`. The `app.html` theme-init script is loaded as an external file under `script-src 'self'`.
- Settings sub-components (`AgentCertificateSettings.svelte`, etc.) demonstrate component extraction is an established and working pattern in this codebase.
- `src/lib/utils.ts` contains only pure functions with no side effects and no framework imports, making it fully unit-testable.
- `Promise.allSettled` in `settings/+page.svelte` and `settings/global/+page.svelte` loads all sections in parallel with per-section error handling.
- Shared modal infrastructure (`ModalBackdrop` -> `Modal` -> `ConfirmDialog`) is a clean three-level hierarchy with correct responsibilities.
- The SSE implementation uses `fetch()` with `ReadableStream` instead of native `EventSource`, enabling Bearer token authentication on SSE connections.
- The centralized event store implements lazy SSE connection management with automatic connect-on-first-subscriber and disconnect-on-last-unsubscribe, with 200ms debounce deduplication.
- Svelte 5 rune adoption with module-level reactive singletons (`auth.svelte.ts`, `notifications.svelte.ts`, `extensions.svelte.ts`, `theme.svelte.ts`) uses the getter/setter pattern — idiomatic Svelte 5.
- Permission-based navigation with extension integration: layout merges built-in and extension-provided navigation entries by priority.

### Issues

**ARCH-01 — God component routes (400-677 lines) mixing unrelated concerns**

Several route files combine data fetching, SSE subscriptions, multiple modals, context menus, pagination, and rendering into a single component. Files exceeding 400 lines: `software/+page.svelte` (~617 lines), `plugin-configs/+page.svelte` (~677 lines), `hosts/[id]/+page.svelte` (~573 lines), `software/[id]/+page.svelte` (~491 lines), `services/+page.svelte` (~470 lines), `system-services/+page.svelte` (~430 lines), `history/+page.svelte` (~410 lines), `hosts/[id]/packages/+page.svelte` (~400 lines). Testing any one behaviour requires navigating the full file.

**ARCH-02 — `AssignToHostModal.svelte` silently truncates at hardcoded limits**

`src/lib/components/AssignToHostModal.svelte` fetches hosts with `getHosts(1, 200)` and plugin configs with `getPluginConfigs(1, 500)`. If either collection exceeds these limits, the modal silently shows an incomplete list with no indication that results are truncated. In large deployments users could silently miss hosts or configs that exist. The `SchemaForm` component already implements proper paginated fetching with a `do...while` loop, demonstrating the pattern is known in this codebase.

**ARCH-03 — Mixed `$app/stores` and `$app/state` usage**

Six files import `page` from the deprecated `$app/stores` instead of `$app/state` (Svelte 5 API): `+layout.svelte`, `login/+page.svelte`, `+error.svelte`, `software/[id]/+page.svelte`, `extensions/[id]/+page.svelte`, `device/+page.svelte`. All other detail pages use `$app/state`. The dual access pattern (`$page.url` vs `page.url`) makes grep-based refactoring unreliable. See also [STD-S1](#std-s1--appstores-import-instead-of-appstate-6-files).

**ARCH-04 — `getCredentialWarnings` duplicated verbatim in two route files**

`src/routes/services/+page.svelte` lines 129-144 and `src/routes/system-services/+page.svelte` lines 129-144 contain identical functions. Any new capability requires updating both files. Should be extracted to `src/lib/utils.ts`.

**ARCH-05 — Missing `encodeURIComponent` on path parameters in ~13 API functions**

In `src/lib/api.ts`, approximately 13 functions interpolate `id` parameters directly into URL paths without `encodeURIComponent()`. Every other update/delete function in the same file applies it. Affected functions include `updateSoftwareItem`, `triggerSoftwareUpdate`, `checkSoftwareItemVersionsHost`, `getUpdateHistoryEntry`, `getSchedulerTask`, `updateSchedulerTask`, `triggerSchedulerTask`, `getPluginConfig`, `updatePluginConfig`, `deletePluginConfig`, `triggerPluginConfigDiscovery`, `discardPluginConfigDiscovered`, `revokeApiToken`. While harmless today with UUID IDs, this inconsistency creates a latent path traversal risk if ID formats change. See also [SEC-06](#sec-06--medium-missing-encodeURIComponent-on-path-parameters).

**ARCH-06 — SSE stream-reading logic duplicated**

`src/lib/sse.ts` contains two nearly identical stream-reading loops (`readSseStream` and `readAdminEventStream`) with the same buffering, line splitting, event field accumulation, and `parseSseEvent` call. A wire format change must be applied to both independently. Similarly, `connectOutputStream` and `connectEventStream` duplicate the connection setup, error handling, and exponential backoff reconnection pattern.

**ARCH-07 — `SystemServicesSettings.svelte` heading mislabelled**

`src/routes/settings/SystemServicesSettings.svelte` displays `<h2>System Services</h2>` but actually manages system enrollment tokens. The component name, heading, and content are misaligned. Users see "System Services" but are managing enrollment tokens.

**ARCH-08 — Root page (`/`) has no content**

`src/routes/+page.svelte` renders only a greeting card. After login users land on a page that provides no actionable information. There is no redirect to a meaningful dashboard route.

**ARCH-09 — History page trigger modal requires manual UUID entry**

`src/routes/history/+page.svelte` contains the comment `// Host selection requires loading the software item detail. Enter the host UUID manually if needed.` — users must type a raw host UUID by hand. This is unfinished developer-workaround UX left in a user-facing interface.

**ARCH-10 — Audit log tab default ignores user permissions**

`src/routes/audit-logs/+page.svelte` `resolveTab()` defaults to `'tenant'` regardless of whether the user has `ViewAuditLogs`. A user with only `ViewSystemAuditLogs` permission lands on the tenant tab, sees an access-denied or empty result, and must manually switch.

**ARCH-11 — xterm.js packages in `devDependencies`**

In `package.json`, `@xterm/xterm`, `@xterm/addon-fit`, and `@xterm/addon-web-links` are listed under `devDependencies`. These are imported and bundled for production UI. The classification is semantically wrong and will mislead automated tooling.

**ARCH-12 — `history/+page.svelte` uses `<ModalBackdrop>` directly instead of `<Modal>`**

This bypasses the standard title/footer chrome, `maxWidth` prop, and accessibility structure provided by `<Modal>`. All other modals in the application use `<Modal>`.

**ARCH-13 — `EnrollmentTokenSettings.svelte` does not use the `<Pagination>` component**

It renders a static "Page X of Y" text label with no navigation controls. All other paginated lists in the application use `src/lib/components/Pagination.svelte`.

**ARCH-14 — `profile/+page.svelte` uses a local timestamp for the newly created token**

`handleCreate` pushes the token onto the local list using `new Date().toISOString()` for `created_at` rather than the server-returned value. If the server-assigned timestamp differs (clock skew, time zone), the displayed value is wrong until the user refreshes.

**ARCH-15 — Duplicated context menu positioning logic across four route pages**

The `toggleMenu` function with identical `getBoundingClientRect` positioning logic is copy-pasted across `services/+page.svelte`, `hosts/+page.svelte`, `software/+page.svelte`, and `system-services/+page.svelte`. Each independently calculates `rect` from the button element and sets `menuX`/`menuY` state. Positioning bugs must be fixed in four places. This should be extracted into the `ContextMenu.svelte` component or a shared composable function. The `left: rect.right - 160` also hardcodes a menu width assumption.

**ARCH-16 — Repeated `setInterval` + `onDestroy` polling pattern without abstraction**

Eight route pages independently implement the same polling pattern: declare `refreshInterval`, start `setInterval` in `onMount`, clear in `onDestroy`. This cross-cutting concern should be extracted into a reusable `usePolling(callback, intervalMs)` utility that automatically handles cleanup. See also [STD-S9](#std-s9--duplicated-polling-pattern-with-setinterval--manual-cleanup).

**ARCH-17 — `api.ts` monolith at ~1029 lines with 80+ exported functions**

`src/lib/api.ts` contains ~83 exported functions covering authentication, services, hosts, packages, software items, update history, scheduler, plugin configs, notifications, audit logs, and extensions. Consider splitting into domain-specific API modules (e.g., `api/auth.ts`, `api/hosts.ts`, `api/software.ts`) importing shared fetch infrastructure from `api/client.ts`.

**ARCH-18 — Fire-and-forget async calls inside `$effect` without lifecycle guards**

Multiple extension components use `void` to fire-and-forget async operations inside `$effect` (e.g., `SchemaTable.svelte`, `SchemaKeyValue.svelte`, `ServiceSelector.svelte`). If the component unmounts before the async operation completes, state updates on the unmounted component occur silently. Consider tracking a mounted flag or using an `AbortController` pattern tied to the effect's cleanup function.

**ARCH-19 — No centralized error boundary strategy beyond root `+error.svelte`**

Any unhandled error in any route page navigates to the root error page, losing all application state. SvelteKit supports `+error.svelte` at any route level. Adding error boundaries at the authenticated layout level would allow showing error states inline while preserving navigation and session state. This is particularly important for extension pages where extension-provided content could cause rendering errors.

**ARCH-20 — Notification system lacks message queuing and stacking**

`src/lib/notifications.svelte.ts` stores a single `successMessage` and a single `errorMessage`. Multiple operations completing in quick succession overwrite earlier messages. For parallel operations (like bulk host assignment), partial failures show only the last error.

---

## 2. Security and Safety

### Strengths

- The access token is stored exclusively in a module-level `let` variable (`auth.svelte.ts:24`) — never in `localStorage`, `sessionStorage`, or JavaScript-accessible cookies. This eliminates the most common XSS token-exfiltration vector.
- Token refresh is correctly deduplicated with a shared `refreshPromise` preventing thundering-herd re-authentication.
- `{@html ...}` is not used anywhere in the entire `src/` tree. All server-supplied strings are rendered through Svelte's safe text interpolation.
- `utils.ts:30-35` implements `safeRedirect()` which rejects `//`-prefixed and `http(s)://` URLs. Applied consistently at all post-login redirect sites.
- `handleOidcLogin` enforces `startsWith('https://')` before assigning `window.location.href`, preventing protocol-downgrade or `javascript:` redirects.
- All mutating requests use `credentials: 'same-origin'`. The `Authorization: Bearer` header on every authenticated request is itself a CSRF mitigation.
- A strict CSP is configured in `svelte.config.js`: `default-src 'self'`, `script-src 'self'` (no `unsafe-inline`), `object-src 'none'`, `form-action 'self'`.
- Both SSE connections send `Authorization: Bearer` via a custom `fetch()` header — the correct approach since native `EventSource` cannot send custom headers.
- `package-lock.json` is present. The project is `"private": true`. No CDN-loaded third-party scripts exist, eliminating a class of supply chain attacks.
- Error messages from the server are truncated to 500 characters (`api.ts:89-93`) before display, limiting information leakage.
- Registration code is passed via URL hash fragment (not query string), preventing it from appearing in server logs, referrer headers, or browser history.
- OIDC `isValidLogoUrl()` validates HTTPS-only URLs before rendering in `<img>` tags. The login page re-validates before rendering.
- Device code input validated against a strict regex pattern (`/^[BCDFGHJKLMNPQRSTVWXZ]{4}-[BCDFGHJKLMNPQRSTVWXZ]{4}$/`) before submission.
- Sensitive credential fields (MQTT passwords, OIDC client secrets) use `type="password"` and the API returns only `has_password: bool` flags rather than actual secrets.
- `TerminalOutput.svelte` initializes xterm.js with `disableStdin: true`, preventing user input injection.

### Issues

**SEC-01 — Medium: Extension system allows server-controlled arbitrary API dispatch**

`ActionButton.svelte:57-61` calls `apiSubmitRequest(def.path, def.method, body)` where `path` and `method` come from the server-returned extension manifest. `SchemaForm.svelte:49-65` calls `apiGet(f.select_source.path)` for the same reason. A compromised or tenant-controlled manifest can cause the frontend to make authenticated API calls to any same-origin endpoint with attacker-controlled bodies — including destructive ones like CA rotation or host deactivation. This is an intentional architectural trade-off but the trust model is undocumented.

**SEC-02 — Medium: `ActionButton` template substitution does not validate field values**

`ActionButton.svelte:28-50` `applyTemplate()` has three issues: (1) `csv_array` coercion splits on commas with no element count limit — a user can submit thousands of commas generating an oversized array; (2) `number` coercion produces `NaN` for non-numeric input which serialises to `null` in JSON; (3) unknown placeholder field names silently produce empty strings.

**SEC-03 — Low: OIDC `link_token` passed in URL query parameters**

`login/+page.svelte:83-88` — when account linking is required, `link_token` is placed in the query string and recorded in browser history, server access logs, and potentially `Referer` headers. The `registration_code` for OIDC completion is correctly placed in the URL hash fragment (never sent to the server). The same pattern should be applied to `link_token`.

**SEC-04 — Low: `SchemaForm` `select_source` path is not validated against a prefix allowlist**

`SchemaForm.svelte:49-65` calls `apiGet(path)` where `path` comes directly from the server-supplied manifest. While `apiGet` is same-origin only, an extension can read from sensitive endpoints not intended to populate a select field. The response fields are rendered in option labels and become visible to the user.

**SEC-05 — Low: `vite.config.ts` dev proxy uses `secure: false` without documentation**

The development proxy disables TLS certificate validation for the `https://localhost:8443` target. This is correct for self-signed dev certs but is undocumented. A developer who points this at a remote host would silently bypass certificate validation.

**SEC-06 — Medium: Missing `encodeURIComponent` on path parameters in multiple API functions**

In `src/lib/api.ts`, approximately 13 functions interpolate `id`, `itemId`, or `hostId` path parameters directly into URL strings without `encodeURIComponent()`. While most IDs are server-generated UUIDs (low practical risk), inconsistent encoding creates a latent path traversal / URL injection risk. Other functions in the same file (e.g., `deleteAutodiscoveryIgnore`, `getHost`) correctly use `encodeURIComponent`. See [ARCH-05](#arch-05--missing-encodeuricomponent-on-path-parameters-in-13-api-functions) for the full list.

**SEC-07 — Medium: Unvalidated `release_url` rendered as clickable href**

In `software/[id]/+page.svelte`, `release_url` from server-stored release metadata is rendered directly as `<a href={meta.release_url}>` without protocol validation. If a malicious plugin source writes a `javascript:` URL into release metadata, clicking the link would execute arbitrary JavaScript. The application already has `isValidLogoUrl()` for validating HTTPS-only image URLs — a similar `isValidExternalUrl()` is absent here.

**SEC-08 — Low: OIDC error parameter reflected with raw interpolation**

In `login/+page.svelte:67`, when `oidc_error` does not match a known key, the raw query parameter value is interpolated: `error = errorMessages[oidcError] || 'Authentication error: ${oidcError}'`. While Svelte auto-escapes (not XSS), attacker-controlled text appears in the UI enabling phishing (e.g., `?oidc_error=Your+session+expired.+Re-enter+password+at+evil.com`). Replace with a generic message.

**SEC-09 — Low: Extension navigation items rendered without `required_permission` check**

In `+layout.svelte`, built-in nav items are filtered by permission, but extension nav items injected via `getPageExtensions().map(...)` skip the `required_permission` check. The extension detail page at `extensions/[id]/+page.svelte` also does not check `required_permission` before rendering content. See also [EXT-13](#ext-13--extension-permission-check-missing-on-page-type-extensions).

**SEC-10 — Low: NATS URL potentially displayed with embedded credentials**

In `settings/global/+page.svelte`, `natsCurrentUrl` is displayed as plaintext. NATS URLs can contain embedded credentials (`nats://user:password@host:4222`). The backend should return a pre-masked URL, or the frontend should mask the password component.

**SEC-11 — Low: Extension page reflects unsanitized extension ID in user-visible text**

In `extensions/[id]/+page.svelte:37`, the `extensionId` from the URL route parameter is rendered in `"The extension "{extensionId}" is not available."`. Svelte auto-escapes (no XSS), but attackers can craft misleading URLs. Consider truncating or omitting the raw ID.

---

## 3. Code Quality

### Strengths

- All API calls go through `api.ts` `request<T>()` or `requestVoid()`. No raw `fetch()` in route files.
- `Promise.allSettled` used for parallel independent fetches.
- Confirm/submit handlers follow the `submitting = true` -> `finally` reset pattern consistently, preventing double-submission.
- Full focus management in `ModalBackdrop.svelte` (Tab/Shift+Tab, Escape, focus restoration) and full keyboard navigation in `ContextMenu.svelte`.
- Permission checks use `$derived` and the `Permission` enum throughout.
- Comprehensive permission gating: all pages check fine-grained permissions and conditionally render both UI elements and action buttons.
- Well-structured settings page delegates to focused sub-components each with their own state management and API interactions.
- Consistent error catch pattern: `e instanceof Error ? e.message : 'Fallback message'` applied uniformly.

### Issues

**CQ-01 — High: `extensions/[id]/+page.svelte` form submit handler is a no-op**

`src/routes/extensions/[id]/+page.svelte` renders `<SchemaForm onsubmit={async () => {}} />`. The form renders and accepts input, but submission silently does nothing. Users can fill out the form and click Submit with no feedback and no effect.

**CQ-02 — High: `EnrollmentTokenSettings.svelte` and `SystemServicesSettings.svelte` revoke without confirmation**

Both `handleRevoke()` functions call the revoke API directly on button click with no `ConfirmDialog`. Token revocation is irreversible. The `ConfirmDialog` component exists and is used in peer pages for destructive actions.

**CQ-03 — High: Legacy `$app/stores` import causes incorrect reactivity in Svelte 5**

`software/[id]/+page.svelte:4`, `extensions/[id]/+page.svelte:2`, `device/+page.svelte:4` import `page` from `$app/stores`. Reactive reads inside `$effect` or `$derived` blocks will not correctly track URL changes. `software/[id]/+page.svelte` mixes `$derived($page.params.id)` (store subscription) with rune derivation, which produces subtly different runtime behaviour from the rest of the codebase.

**CQ-04 — Medium: `SchemaForm.svelte` `$effect` resets all user input on referential field change**

`SchemaForm.svelte:34-45` resets the entire `values` object to defaults whenever `fields` or `extraParams` changes. If a parent re-renders and passes a referentially new but semantically identical `fields` array (common with `$derived`), all in-progress user input is silently discarded with no warning. Since `extraParams` is an object literal constructed inline in `SchemaTable.svelte`, it creates a new reference on every render cycle.

**CQ-05 — Medium: `TerminalOutput.svelte` double-write on initial mount**

When `output` is non-empty on mount, both `onMount` and the `$effect` tracking `output` write to the terminal. The `onMount` write is immediately followed by the `$effect` clearing and rewriting. This causes a visible flicker when opening a terminal modal with pre-loaded output.

**CQ-06 — Medium: `MqttClientsSettings.svelte` — API call inside prop-sync `$effect` and non-null assertion**

The `$effect` that syncs the `clients` prop fires `loadMqttLimit()` on every parent re-render. Additionally, `limitInput = mqttLimit!` is executed synchronously after an async call — `mqttLimit` is `null` at assignment time, making the non-null assertion always incorrect on first run.

**CQ-07 — Medium: `software/[id]/+page.svelte` — `setTimeout(0)` for SSE connection timing is fragile**

`setTimeout(() => connectOutputStream(...), 0)` relies on browser task-queue ordering which is not a Svelte-guaranteed contract. If terminal mount is delayed, the SSE connection opens before the terminal exists and initial data is lost. The correct approach is a callback or `$bindable` prop from `TerminalOutput` signalling readiness.

**CQ-08 — Medium: `history/+page.svelte` — `terminalRefs` is a plain non-reactive object**

`let terminalRefs: Record<string, TerminalOutput> = {};` — Svelte 5 does not track mutations to plain objects. If any reactive expression reads `terminalRefs` to conditionally call `.write()` or `.fit()`, it will not rerun after DOM mounts populate the bindings. This should be `$state({})` if reactivity is needed.

**CQ-09 — Medium: `audit-logs/+page.svelte` — `datetime-local` values not converted to RFC 3339**

`<input type="datetime-local">` produces `YYYY-MM-DDTHH:MM` (local time, no timezone). These values are passed directly to the API. If the backend expects RFC 3339, the request may fail silently or return incorrect time boundaries.

**CQ-10 — Medium: `plugin-configs/+page.svelte` — no `submitting` guard on Save**

The Save button for plugin configuration has no `submitting` flag to disable it during the in-flight request. Rapid double-clicks send two concurrent PATCH requests, with the second potentially overwriting the first's response.

**CQ-11 — Medium: `getCredentialWarnings` duplicated verbatim (also ARCH-04)**

Identical function in `services/+page.svelte:129-144` and `system-services/+page.svelte:129-144`.

**CQ-12 — Low: `matchMedia` listener in `theme.svelte.ts` is never removed**

`initTheme()` registers `window.matchMedia(...).addEventListener('change', handler)` but returns no cleanup and the caller stores no teardown. During development with Vite HMR, each module reload accumulates listeners.

**CQ-13 — Low: `AgentCertificateSettings.svelte` Save button missing `getIsOnline()` guard**

`AuthenticationSettings.svelte` and `RegistrationSettings.svelte` both disable their Save buttons when `!getIsOnline()`. The agent certificate save button does not, producing inconsistent UX for the same failure mode across three panels on the same page.

**CQ-14 — Low: `copied` timeout not cleared on component destroy**

`EnrollmentTokenSettings.svelte` and `SystemServicesSettings.svelte` both use `setTimeout(() => { copied = false; }, 2000)` in `handleCopy` without storing the handle or clearing it in `onDestroy`. If the component is unmounted before 2000 ms, the callback fires against a destroyed component's state.

**CQ-15 — Low: `profile/+page.svelte` uses `navigator.clipboard` directly, bypassing the utility**

`src/lib/utils.ts` exports `copyToClipboard` which handles the `document.execCommand` fallback for non-HTTPS or restricted clipboard contexts. This one call site bypasses the fallback silently.

**CQ-16 — Low: Floating promise `fetchAlerts()` in `+layout.svelte`**

`fetchAlerts()` is called as a bare expression inside an `$effect`. Any synchronous error thrown before its first `await` is silently swallowed. The `void` prefix should be used as in other fire-and-forget calls throughout the codebase.

**CQ-17 — Low: `redundant async` on several `api.ts` functions**

Several exported functions are declared `async` but their bodies contain no `await` — they return the result of `request()` / `requestVoid()` directly. Examples: `updateSoftwareItem`, `triggerSoftwareUpdate`, `listUpdateHistory`. The `async` wrapper adds an unnecessary `Promise` layer and masks future `return` vs `return await` bugs. See also [STD-S4](#std-s4--inconsistent-async-keyword-on-api-wrapper-functions).

**CQ-18 — Low: `extensions.svelte.ts` swallows load errors without user notification**

On fetch failure, `loadExtensions` logs to `console.error` and sets `loaded = true` but shows no toast. Users cannot distinguish "no extensions configured" from "extensions failed to load."

**CQ-19 — Medium: `services/+page.svelte` and `system-services/+page.svelte` are structurally near-identical**

These two pages duplicate: URL sync `$effect`, filter button rendering, context menu/confirm dialog patterns, `toggleMenu`/`closeMenu`/`requestConfirm`/`cancelConfirm`/`executeConfirmed`/`openPingDialog`/`cancelPingEdit`/`executePingEdit`/`handleWindowClick` functions, and the `confirmLabels` object. The only material differences are the API functions called, the filter type, and the merge dialog (services only). Consider extracting a shared composable or wrapper component.

**CQ-20 — Medium: `SystemServicesSettings.svelte` tokens not loaded on mount**

The component requires the user to click "Load Tokens" to fetch system enrollment tokens. Every other settings sub-component loads its data automatically on mount. This inconsistency forces an extra user action.

**CQ-21 — Medium: No `submitting` guard on `saveOidcProvider` in `OidcProvidersSettings.svelte`**

The Save/Update button is only disabled by `!getIsOnline()`, not by a submitting state. Rapid clicks could create duplicate providers or fire concurrent update requests.

**CQ-22 — Medium: No `submitting` guard on `saveMqttClient` in `MqttClientsSettings.svelte`**

Same issue as CQ-21. The save button is only disabled by `!getIsOnline()`.

**CQ-23 — Medium: `login/+page.svelte` `$effect` fires side effects on every URL parameter change**

The `$effect` depends on `$page.url.searchParams` and `$page.url.hash`. If `oidc_code` is present, it calls `handleOidcCallback` again on every URL change. The code lacks a guard to prevent re-processing the same `oidc_code`. Consider adding an `oidcProcessed` guard flag similar to `hasRedirected`.

**CQ-24 — Medium: `history/+page.svelte` SSE `termRef` captured at connection time may be stale**

In `connectSse`, `termRef` is captured from `terminalRefs[updateHistoryId]` at call time. If the terminal component remounts, the closure points to a stale/destroyed instance. Subsequent `.write()` calls write to a disposed xterm. Consider reading `terminalRefs[updateHistoryId]` inside the `onOutput` callback instead.

**CQ-25 — Medium: `settings/+page.svelte` double `$effect` may call `loadAllSettings` twice**

The `$effect` that calls `loadAllSettings()` runs every time `canManageSettings` changes. If the user object is re-set (e.g., after token refresh), `canManageSettings` could momentarily transition false->true, causing duplicate loads. Consider adding a `loaded` flag or using `onMount`.

**CQ-26 — Medium: `software/+page.svelte` `executeUpdate` does not reset `triggeringUpdate` in error path**

`triggeringUpdate = true` on line 253 but `triggeringUpdate = false` is outside any `finally` block. If `updateModalItem!` non-null assertion throws, `triggeringUpdate` stays `true` permanently and the button is disabled.

**CQ-27 — Low: `software/[id]/+page.svelte` `getReleaseMeta` called three times per host row**

In the host table row template, `getReleaseMeta(host)` is called three times for the same host. Use `{@const meta = getReleaseMeta(host)}` at the start of the row block to compute once.

**CQ-28 — Low: `history/+page.svelte` filter buttons hardcode status list, missing `'queued'`**

The filter buttons iterate over a hardcoded array `['all', 'pending', 'in_progress', 'completed', 'failed']` which omits `'queued'` from the `STATUS_FILTER_VALUES` constant. The "queued" filter is available via URL parameter but not clickable in the UI.

**CQ-29 — Low: `settings/+page.svelte` error sections rendered 6 times with near-identical markup**

Six consecutive `<aside>` blocks display error variables with identical class strings. When `getCombinedSettings` fails, four separate identical-looking error asides appear. Consider an `ErrorBanner` component or a loop over error states.

---

## 4. Tests and Coverage

### Configuration and Setup

The Vitest configuration is correct and minimal. `svelte({ hot: false })` prevents HMR transform false positives. The `browser` resolve condition correctly loads the client runtime. Coverage thresholds are enforced at 70% lines/functions and 65% branches for `src/lib/**`. Route-level code is excluded from the coverage gate entirely — there is no enforcement that route tests remain in place.

### Existing Test Files — Assessment

| File                    | Quality  | Assessment                                                                                                                                                                                                        |
| ----------------------- | -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/lib/api.test.ts`   | High     | Comprehensive: `extractErrorMessage` all paths, `authenticatedFetch` 401/refresh/deduplicate/timeout. Minor gap: `apiSubmitRequest` BASE-stripping, `oidcExchange` not exercised.                                 |
| `src/lib/auth.test.ts`  | Good     | Covers `initialize`, `handleLogin`, `handleLogout`, `handleOidcCallback`. Missing: `handleRegister`, `handleOidcLogin` (security-sensitive `https://` guard), `handleOidcCompleteRegistration`, `handleOidcLink`. |
| `src/lib/utils.test.ts` | High     | All 7 exported functions covered including XSS-relevant `isValidLogoUrl` and `safeRedirect` edge cases.                                                                                                           |
| `ConfirmDialog.test.ts` | Good     | Covers render, button labels, callbacks, `confirmDisabled`. Missing: keyboard dismiss, `warnings` prop.                                                                                                           |
| `ContextMenu.test.ts`   | High     | Exemplary keyboard navigation testing (Escape, ArrowDown wrap, ArrowUp, Home, End, Enter).                                                                                                                        |
| `Modal.test.ts`         | Good     | Covers render variants and Escape. Minor: CSS class assertions are brittle (tests CSS framework choices rather than behaviour).                                                                                   |
| `ModalBackdrop.test.ts` | High     | Full focus trap testing (Tab wrap, Shift+Tab, backdrop click, child click no-close).                                                                                                                              |
| `Pagination.test.ts`    | Good     | Covers navigation, disabled states, callbacks. Missing: `totalPages === 0` edge case.                                                                                                                             |
| `hosts.test.ts`         | Good     | Covers render, empty state, error+retry, permissions, discovery flow. Missing: Edit Name submit, Deactivate confirm->call, pagination, retry re-fires API call.                                                   |
| `host-detail.test.ts`   | Good     | Thorough detail page test. Missing: Edit Name/Deactivate flows, SSE unsubscribe on destroy, discovery error path.                                                                                                 |
| `services.test.ts`      | Adequate | Thin. Missing: approve/reject/delete/merge actions, `showSuccess`/`showError` verification, Rejected/Deactivated status badges.                                                                                   |

### Critical Testing Gaps (prioritized)

1. **`src/lib/sse.ts` — no tests at all (Critical)**
   - `parseSseEvent`: pure function, trivially testable — multi-line `data:`, comment lines, missing data, leading space stripping.
   - `connectOutputStream` reconnection: backoff calculation, max-attempts exhaustion, clean disconnect preventing further reconnects.
   - `connectEventStream`: reconnects on clean close (distinct from `connectOutputStream`), attempt counter reset on success.

2. **`src/lib/stores/events.svelte.ts` — no tests (High)**
   - SSE open on first subscriber, close on last unsubscribe (ref-counting).
   - Debounce deduplication (requires fake timers).
   - `dispatchEvent` entity ID extraction fallback chain (`data.id`, `data.host_id`, `data.task_id`).

3. **`ActionButton.svelte` — `applyTemplate` logic untested (High)**
   - All coercion types: `bool`, `number`, `csv_array`.
   - Nested object template, missing field fallback.
   - Security-adjacent: forms the body of extension API calls.

4. **`src/lib/auth.test.ts` — `handleOidcLogin` security check untested (High)**
   - The `https://`-only URL validation should be explicitly tested with an `http://` redirect and a non-URL value.

5. **`src/lib/notifications.svelte.ts` — no tests (Medium)**
   - `showSuccess` auto-clear timer (fake timers needed), double-call timer cancellation.

6. **`SchemaForm.svelte` — no tests (Medium)**
   - Dynamic `select_source` pagination loop, hidden field from `_row`, `initiatedKeys` deduplication guard.

7. **`src/lib/extensions.svelte.ts` — no tests (Medium)**
   - Filter functions with placement type discrimination.
   - `loadExtensions` error path.

8. **`services.test.ts` — approve/reject flows missing (Medium)**

9. **`AssignToHostModal.svelte` — submit, validation, double-submit guard (Medium)**

### New Test Issues

**TEST-01 — High: `auth.test.ts` does not test `handleRegister`, `handleOidcCompleteRegistration`, or `handleOidcLink`**

All three are exported functions with conditional logic (e.g., `handleOidcLink` has conditional password inclusion). `sessionExpired` state lifecycle is also untested across login/logout/OIDC transitions.

**TEST-02 — High: `auth.test.ts` does not test the `initialize` error path when `me()` fails**

The test covers "refresh succeeds + me succeeds" and "refresh fails", but not "refresh succeeds + me fails" (the outer catch block). This is a distinct error path from refresh failure.

**TEST-03 — High: `api.test.ts` module-level `refreshPromise` leaks between tests**

The `refreshPromise` variable at module scope is shared across all tests. If a test triggers a refresh that is still settling when the next test runs, the pending promise leaks. The `afterEach` only calls `vi.unstubAllGlobals()` but does not reset the module's `refreshPromise` state.

**TEST-04 — Medium: `api.test.ts` does not test the `requestVoid` error handling path**

`requestVoid` has identical error handling to `request` (TypeError -> network error, DOMException -> timeout) but is never tested. Functions like `logout`, `deleteService`, `deleteHostPackage` use it.

**TEST-05 — Medium: `api.test.ts` does not test network error handling (TypeError path)**

The `request` function catches `TypeError` from fetch and translates it to "Network error: Unable to connect to the server." This path is untested.

**TEST-06 — Medium: `theme.svelte.ts` has no tests**

Contains testable pure logic: `getStored()` validates localStorage values against a whitelist, `applyTheme()` has conditional class toggle logic.

**TEST-07 — Medium: `network.svelte.ts` has no tests**

Has module-level side effects (registering event listeners) and exposes `getIsOnline()`. Event listener registration and response to online/offline events is testable.

**TEST-08 — Medium: `hosts.test.ts` and `host-detail.test.ts` do not test deactivate/edit flows**

Tests verify button visibility based on permissions but never test the actual interactions despite mocking `updateHost` and `deactivateHost`.

**TEST-09 — Medium: Coverage thresholds exclude route-level test files**

The coverage `include` is `['src/lib/**']` — route page components containing substantial conditional logic are not tracked or enforced.

**TEST-10 — Medium: `auth.test.ts` uses `beforeEach` at module scope without matching cleanup**

No `afterEach` cleanup; if a test stubs globals, the stub persists. `host-detail.test.ts` mutates `page.params` without cleanup, contaminating shared mock state.

**TEST-11 — Medium: `app-navigation.ts` mock `goto` is a no-op with no assertion capability**

The `goto` mock is `async function goto() {}` — it cannot be spied on to verify navigation calls. Should be `vi.fn()`.

**TEST-12 — Medium: `app-state.ts` mock is too minimal and lacks reset capability**

The mock exports a plain object with no function to reset to known state. Mutations persist across tests.

**TEST-13 — Medium: `Modal.test.ts` tests CSS class names rather than behavior (philosophy violation)**

Lines 32-33 assert `bg-surface-50` and `dark:bg-surface-900` class application. Line 58-61 tests footer absence via `.querySelector('.justify-end')`. These test CSS framework choices, not component behavior.

**TEST-14 — Low: `ConfirmDialog.test.ts` does not test the `warnings` prop rendering**

The component accepts a `warnings` array and renders each inside an aside element. No test passes warnings.

**TEST-15 — Low: `formatVersion` test has incorrect description**

`utils.test.ts:194` says "truncates a sha256 string that is exactly 19 chars after prefix" but the input has only 12 chars after the prefix.

**TEST-16 — Low: `AddSoftwareModal.svelte` has no tests despite containing validation logic**

Has guard logic for double submission, name trimming, non-empty validation, and API error handling.

**TEST-17 — Low: `ToastNotifications.svelte` has no tests**

Renders success/error messages with dismiss buttons and conditional action links for system alerts.

**TEST-18 — Low: `Pagination.test.ts` missing `afterEach(cleanup)`**

Unlike other component test files, no explicit cleanup. `test-setup.ts` also does not configure global cleanup.

**TEST-19 — Low: `services.test.ts` has unused mock functions**

Mocks `mergeService` and `updateService` that are never called in any test, suggesting planned tests never written.

### Testing Philosophy

Existing tests largely adhere to "test your own logic, not framework behaviour." Component tests use `@testing-library/svelte` and assert on rendered DOM and user interactions. The violations: `Modal.test.ts` asserts CSS class names (TEST-13), and `ContextMenu.test.ts` Home/End tests bypass internal focus tracking via manual `.focus()` calls.

### Test Isolation Issues

- `api.test.ts` calls `vi.mocked(setAccessToken).mockReset()` in `beforeEach` without resetting `setSessionExpired` — inconsistency that could cause cross-test contamination in future (TEST-03).
- `host-detail.test.ts` sets `page.params.id = 'host-001'` in `beforeEach` without resetting in `afterEach`. If another test file imports `$app/state` expecting clean params, it would see a stale value (TEST-10).

---

## 5. High Availability and Resilience

### Strengths

- Token refresh deduplication coalesces concurrent 401 responses into a single refresh call.
- `RefreshError` status distinguishes network timeouts (keep session), 5xx (keep session), and 4xx (invalidate, show banner). Session expiry shows a banner rather than hard redirect, preserving unsaved form state.
- `AbortSignal.timeout(30_000)` on all requests; shorter `REFRESH_TIMEOUT_MS = 10_000` for refresh.
- `connectEventStream` uses `maxReconnectAttempts: Infinity`, capped 30s delay, and resets the attempt counter on successful reconnection.
- Centralized SSE store: single connection shared across all subscribers, opened on first subscriber, closed on last.
- 200ms debounce on admin events keyed by `"type:entityId"` protects against thundering-herd reloads.
- `network.svelte.ts` tracks `navigator.onLine` — offline banner shown, login buttons disabled when offline.
- Background refreshes never overwrite visible error state and never show a loading spinner.
- 5-minute `setInterval` poll fires only when `document.visibilityState === 'visible'`.
- `Promise.allSettled` for settings page parallel loads.
- `extensions.svelte.ts` fails open — extension load errors set `extensions = []` and mark `loaded = true`.
- `TerminalOutput.svelte` correctly disconnects `ResizeObserver` and `MutationObserver` and disposes the xterm instance in `onDestroy`.
- MQTT polling uses exponential backoff capped at 5 minutes, with randomized jitter to prevent thundering herd.
- Error message truncation at 500 characters prevents DOM bloat.

### Issues

**HA-01 — High: SSE connection never refreshes token on 401; loops forever with stale token**

`sse.ts:222-239` reads the access token once at connection time. When the token expires the server returns 401. The `connectEventStream` catch handler sees this as an error, increments `attempt`, and schedules a reconnect — but it reads the same stale token from `getAccessToken()` because the SSE layer never drives a token refresh (that logic lives only in `api.ts:authenticatedFetch`). The SSE layer will retry indefinitely with the invalid token, never recovering. Pages relying on SSE for real-time updates will show stale data indefinitely until the user manually refreshes.

**HA-02 — Medium: Output stream reconnect replays already-seen lines**

`sse.ts:90-113` — if the stream errors before a `completed` event, reconnection re-requests the full output stream from the beginning. The `seq` field on `OutputLineEvent` is never used for deduplication. The `TerminalOutput` component receives the same output lines multiple times, producing duplicate terminal content.

**HA-03 — Medium: Single `submitting` flag shared across unrelated modal operations**

`services/+page.svelte:31` — one `submitting` boolean is shared across `executeConfirmed` (approve/reject/delete), `executeMerge`, and `executePingEdit`. The Confirm and Merge dialogs show "Processing..." even when a different concurrent operation is in progress. Same pattern in `system-services/+page.svelte`.

**HA-04 — Medium: Race condition between filter change and background SSE refresh**

`services/+page.svelte:56-67` — when `capabilityFilter` changes and `loadServices` fires, an in-flight background SSE-triggered `loadServices` from the previous filter may still be outstanding. When it resolves, it writes its results (old filter) into `services`, momentarily showing filtered-out services before the new filter's fetch lands.

**HA-05 — Medium: `terminalRefs` holds stale disposed xterm references after row collapse**

`history/+page.svelte:36-39, 107-148` — if the user collapses a row while the SSE stream is still active, the `TerminalOutput` component is destroyed (xterm disposed), but `termRef` in the SSE closure still holds the stale reference. Subsequent `termRef.write(line.text)` calls write to a disposed xterm instance; lines are silently lost.

**HA-06 — Medium: `software/[id]/+page.svelte` live SSE stream not disconnected on same-route re-navigation**

`onDestroy` calls `closeLiveModal()` -> `liveDisconnect?.()`. However, if SvelteKit performs a client-side navigation to the same route (same `[id]`) with a different item, `onDestroy` is not called and the component is reused. The `$derived(id)` update triggers `loadItem()` but does not reset `liveDisconnect`. An SSE stream from the previous item keeps running.

**HA-07 — Low-Medium: `history/+page.svelte` `toggleExpand` races rapid expand/collapse**

`setTimeout(() => connectSse(id), 0)` is used to defer SSE connection until `TerminalOutput` has mounted. If the user collapses the row before the `setTimeout` fires, `disconnectStream()` runs synchronously, but `connectSse` fires in the next task anyway, opening an orphaned SSE connection that cannot be cleaned up.

**HA-08 — Low-Medium: Error notifications persist across page navigation**

`errorMessage` in `notifications.svelte.ts` is module-level state. There is no navigation hook that clears it. Error toasts from page A remain visible on page B after navigation.

**HA-09 — Low: Token refresh network error leaves `loading = true` permanently**

`auth.svelte.ts:45-66` — if `api.refreshAccessToken()` throws a network error, the catch block sets `user = null` and returns without reaching the `finally` block that sets `loading = false`. On an intermittent network outage at page load time, the app is permanently stuck at the loading screen with no retry button.

**HA-10 — Low: `settings/global/+page.svelte` silently uses hardcoded defaults if settings load fails**

`loadGlobalSettings` uses `Promise.allSettled` but only checks `results[n].status === 'fulfilled'`. Rejected results are silently ignored. If `getNetworkSettings()` fails, fields like `trustedProxiesText` and `realIpHeader` retain hardcoded defaults. A user who saves would silently overwrite the server configuration with incorrect values.

**HA-11 — Low: `+error.svelte` gives identical "Go to Home" for all error types**

No differentiation between 404 (a "Back" button would help), 401/403 (a "Log in" button), or 5xx (a "Retry" button). Client-side render errors also display `null:` or `undefined:` status with no useful recovery path.

**HA-12 — Low: `history/+page.svelte` filter buttons not locked during load**

`loadHistory` sets `loading = true` but does not prevent filter buttons from being clicked during a load. Two concurrent `listUpdateHistory` calls can race; the slower one (old filter) may resolve last and overwrite the displayed items.

**HA-13 — High: SSE admin event stream does not recover after session expiry and re-login**

The SSE connection is opened once when the first subscriber calls `subscribeToEvent`. After the user logs back in, the stale SSE connection keeps retrying with the old (now invalidated) token indefinitely. There is no mechanism to tear down and recreate the SSE connection when `accessToken` changes. Subscribers from pages mounted before re-login never receive events again; the user must do a full page reload.

**HA-14 — Medium: `$effect` in filter-driven pages creates new `setInterval` on every filter change without cancelling prior pending API calls**

`services/+page.svelte:56-67`, `system-services/+page.svelte:59-70` — each filter change fires a new `loadServices` call. The in-flight call from the previous filter is not cancelled. If the user rapidly changes filters, multiple concurrent calls race; the last to resolve wins and may not correspond to the current filter value.

**HA-15 — Medium: `AssignToHostModal` uses `Promise.all` instead of `Promise.allSettled` for mixed assign/unassign**

`AssignToHostModal.svelte:100-121` — if any single unassign fails, `Promise.all` rejects immediately. Successful operations have already committed server-side, but the UI shows generic failure. The user has no way to know which operations succeeded.

**HA-16 — Medium: No request cancellation on page navigation for in-flight API calls**

All list pages issue API calls in `onMount` or `$effect` but never pass an `AbortSignal` tied to the component lifecycle. When the user navigates away before completion, the response handler still runs, setting state on a potentially-destroying component.

**HA-17 — Medium: `SchemaForm` rest_api select source loads all pages sequentially with no timeout or page cap**

`SchemaForm.svelte:56-95` paginates through all pages (`do { ... page++ } while (page <= totalPages)`) with no cancellation, no upper limit, and no `AbortController`. A misconfigured extension could hang indefinitely or issue hundreds of requests.

**HA-18 — Medium: `history/+page.svelte` has no SSE subscription for real-time update status changes**

Unlike all other list pages which subscribe to SSE events, the update history page only loads data in `onMount` with no SSE subscription for `update_started`, `update_completed`, or `batch_host_package_update_completed` events. No background polling either.

**HA-19 — Medium: `scheduler/+page.svelte` has no data refresh mechanism at all**

Loads tasks once in `onMount` with no polling, no SSE subscription, and no refresh button. The `scheduler_task_completed` SSE event type exists but is never subscribed to. This is the only data-listing page with no refresh mechanism.

**HA-20 — Medium: Settings page MQTT polling starts unconditionally and duplicates initial fetch**

`loadAllSettings` already fetches `getMqttClients()`. Separately, `startMqttPolling` schedules the first poll after 10 seconds, also calling `getMqttClients()` — duplicate fetch within the first 10 seconds.

**HA-21 — Low-Medium: `hosts/[id]/+page.svelte` uses `Promise.all` — failure in either request hides both results**

`Promise.all([getHost(id), listUpdateHistory(...)])` rejects if either call fails. If `listUpdateHistory` fails but `getHost` succeeds, the user sees a generic error with no host data. `Promise.allSettled` with per-section error handling would show host details even when history loading fails.

**HA-22 — Low: `+layout.svelte` `fetchAlerts` has no retry or periodic refresh**

System alerts are fetched once. No polling, no SSE subscription, no retry on failure. New alerts after page load are never displayed until full reload.

**HA-23 — Low: `profile/+page.svelte` API token list has no loading state or error recovery**

No loading indicator, no retry button on failure, no background refresh. Failed initial load shows an empty token list with no indication of error.

---

## 6. Coding Standards

### Strengths

- `strict: true` in `tsconfig.json`. `checkJs: true` for mixed TS/JS.
- All components use `$props()` with explicit TypeScript types. No legacy `export let` prop declarations anywhere.
- `$derived` used for all pure computed values. `$effect` used only for side effects.
- All event handlers use the Svelte 5 `onevent` property syntax. No legacy `on:event` directives.
- `SvelteMap` and `SvelteSet` from `svelte/reactivity` used where deep reactivity is needed.
- Full accessibility implementation: skip-link in `app.html`, `aria-modal` + `aria-labelledby` in `Modal`, `role="status"`/`role="alert"` with `aria-live` in `ToastNotifications`, full keyboard navigation in `ContextMenu`.
- Tabs for indentation, single quotes, 120-character print width — consistent throughout.
- No `console.log` or `console.warn` calls in any production source file.
- All components use Svelte 5 `Snippet` props for content projection, avoiding the legacy `<slot>` pattern entirely.
- `parseUrlParam` utility uses `readonly T[]` with const assertions for compile-time enum validation.

### Systemic Issues

**STD-S1 — `$app/stores` import instead of `$app/state` (6 files)**

The following files import `page` from the deprecated `$app/stores` instead of `$app/state`:

- `src/routes/+layout.svelte:5`
- `src/routes/+error.svelte:2`
- `src/routes/login/+page.svelte:3`
- `src/routes/software/[id]/+page.svelte:3`
- `src/routes/extensions/[id]/+page.svelte:2`
- `src/routes/device/+page.svelte:4`

The test alias in `vitest.config.ts` stubs `$app/state` but has no corresponding stub for `$app/stores`, making these routes harder to test.

**STD-S2 — `table-container` (Skeleton v3) instead of `table-wrap` (Skeleton v4)**

`EnrollmentTokenSettings.svelte:234` and `SystemServicesSettings.svelte:207` use `class="table-container"`. All other table wrappers in the codebase use `class="table-wrap"` (Skeleton v4). If `table-container` is absent from Skeleton v4's stylesheet, responsive overflow behavior will silently not apply.

**STD-S3 — Dynamic Tailwind class construction via string concatenation**

`MqttClientsSettings.svelte` `connectionColor()` constructs classes like `bg-success-500` and `bg-error-500` via string concatenation. Tailwind's scanner performs static analysis and will not detect classes built at runtime. These classes must be in the codebase as complete strings elsewhere, or added to the `safelist` in Tailwind config, or they will be purged from production builds.

**STD-S4 — Inconsistent `async` keyword on API wrapper functions**

Approximately half the exported API functions in `api.ts` use `async` while the other half return the promise from `request<T>()` directly. The `async` keyword on functions that just `return request(...)` adds an unnecessary extra promise wrapper. Pick one convention — preferably drop `async` on simple pass-through returns.

**STD-S5 — `location.pathname` used instead of SvelteKit's `page.url.pathname` in URL sync effects**

Eight route pages use `location.pathname` directly when constructing URLs for `goto()`. Since all these pages already import `page` from `$app/state`, they should use `page.url.pathname` to stay within SvelteKit's routing abstraction. Using raw `location` bypasses SvelteKit's client-side router state.

Affected: `services/+page.svelte`, `hosts/+page.svelte`, `hosts/[id]/packages/+page.svelte`, `software/+page.svelte`, `history/+page.svelte`, `system-services/+page.svelte`, `audit-logs/+page.svelte`, `plugin-configs/+page.svelte`.

**STD-S6 — Plain `Set<string>` with `$state` instead of `SvelteSet` for reactive set tracking**

Multiple route pages declare `let someSet: Set<string> = $state(new Set())` and mutate by creating new `Set` instances. This is O(n) per mutation. `SvelteSet` (already used in `AssignToHostModal.svelte` and `events.svelte.ts`) provides fine-grained reactivity on `.add()`, `.delete()`, `.has()` without full reconstruction.

Affected: `+layout.svelte` (`dismissedAlerts`), `software/+page.svelte` (`selectedHostIds`), `hosts/[id]/packages/+page.svelte` (`togglingIds`), `hosts/+page.svelte` (`discoveringHostIds`).

**STD-S7 — Error `<aside>` elements missing `role="alert"` for screen reader announcement**

~20 error banners rendered as `<aside class="... preset-filled-error-500">` across route pages lack `role="alert"`. Without it, screen readers will not announce dynamically rendered error messages. `ToastNotifications.svelte` correctly uses `role="alert"` and `aria-live`, so the pattern exists but is not applied to inline error asides.

**STD-S8 — Duplicated polling pattern with `setInterval` + manual cleanup**

Seven route pages implement an identical polling pattern without abstraction. A shared `usePolling(fn, ms)` helper would eliminate boilerplate and centralize the cleanup guarantee. See [ARCH-16](#arch-16--repeated-setinterval--ondestroy-polling-pattern-without-abstraction).

**STD-S9 — Callback prop naming inconsistency between settings sub-components and shared components**

Shared components use lowercase callback names (`onclose`, `onconfirm`, `oncancel`). All seven settings sub-components use camelCase (`onSuccess`, `onError`). Extension components use `onComplete`. The project should pick one convention.

**STD-S10 — `<table>` elements across pages lack `aria-label` or `<caption>` for accessibility**

~22 `<table class="table">` elements across route pages include no `aria-label`, `aria-labelledby`, or `<caption>`. Screen readers announce tables by their accessible name; without one, users hear only "table."

### One-Off Issues

**STD-O1 — `$state(undefined!)` non-null assertion in `ContextMenu.svelte:17`**

```ts
let menuEl: HTMLDivElement = $state(undefined!);
```

The `!` forces `undefined` to pass as `HTMLDivElement`, suppressing a genuine type error. The correct type is `HTMLDivElement | undefined`.

**STD-O2 — `.then()/.catch()` chains in `login/+page.svelte`**

`onMount` (lines 93-101) and the OIDC callback `$effect` (lines 46-50) use promise chains instead of async/await or `void asyncFn()`. All other `onMount` and `$effect` usages in the codebase use the async/await pattern.

**STD-O3 — Possible Skeleton v3 class in `MqttClientsSettings.svelte:377`**

`rounded-container-token` is a Skeleton v3 token-based class name. Skeleton v4 uses plain Tailwind `rounded-*` utilities. This class may have no effect in a Skeleton v4 build.

**STD-O4 — Duplicate label association on toggle input in `SchemaForm.svelte`**

The toggle's `<label>` wraps the `<input>` (implicit association) and the `<input>` also has `id={field.key}`. A top-level `<label for={field.key}>` at line 112 matches by `id`, creating two labels for the same control. Screen readers will announce the field label twice.

**STD-O5 — `<table>` in `SchemaTable.svelte` has no `aria-label` or `<caption>`**

The table element has no accessible name. For dynamic extension-driven tables whose column content is entirely runtime-defined, a caption or label is important for screen reader context.

**STD-O6 — `onComplete` vs lowercase callback prop naming inconsistency**

`ActionButton.svelte` and `SchemaTable.svelte` use `onComplete` (camelCase). All other components use fully lowercase callback props (`onsubmit`, `onclose`, `onsave`, `oncancel`). The project should pick one convention.

**STD-O7 — `console.error` in `extensions.svelte.ts:20`**

This is the only `console.error` call in any production source file. Should use `showError()` as all other error paths do.

**STD-O8 — `SvelteMap` used for non-reactive debounce timers in events store**

In `stores/events.svelte.ts:31`, `debounceTimers` is declared as `new SvelteMap<...>()` but is an internal implementation detail never read reactively. A plain `Map` would suffice and avoid Svelte's reactivity tracking overhead during rapid SSE dispatch.

**STD-O9 — `apiSubmitRequest` return type uses `Record<string, unknown>` losing type safety**

`api.ts:1015-1022` returns `Promise<Record<string, unknown>>`, discarding all type information. Adding a generic parameter (like `apiGet<T>`) would preserve type safety at call sites.

---

## 7. Extensibility

### Strengths

- **Schema-driven, server-declared manifests** — extensions are fully described by the backend. No per-plugin frontend code is required. Adding a backend extension does not require a frontend deployment.
- **Clean singleton store** in `extensions.svelte.ts` — minimal API (`loadExtensions`, `clearExtensions`, four filter helpers), lifecycle tied to auth state.
- **Error containment** — failed extension load falls back to `[]` and sets `loaded = true`. Pages degrade gracefully.
- **Priority-based nav injection** — page extensions merged with built-in nav items and sorted by a numeric `priority` field.
- `ActionButton`, `SchemaTable`, `SchemaForm`, `SchemaKeyValue` all manage their own loading/data state independently.
- The `universal` / `targeted` distinction is clean. `contentReady` gate correctly delays rendering until `serviceLoaded` is true.
- Auto-selection when `providers.length === 1` provides good UX for the common single-provider case.
- Extension components use the same Skeleton utility classes as the rest of the app — they automatically inherit the visual style and light/dark mode.
- **Action library with indirection** — extensions carry a top-level `actions: ActionDef[]` catalogue and reference actions by ID string, allowing action reuse across primary actions, row actions, and context selectors.
- **Dual submit path** — `ActionButton` supports both extension proxy (`invokeExtensionAction`) and direct REST API calls via `api_submit` with JSON body templating.
- **Context selector with add-action loop** — `SchemaTable` supports a context selector that loads options from either plugin configs or an arbitrary action, and supports create-then-select workflows via `response_id_field`.
- **Dynamic select source with pagination** — `SchemaForm` loads select options from REST API endpoints with automatic pagination handling.
- **Forward-compatible field type definition** — `FieldType` has a trailing `| string` escape hatch; unknown types fall through to text input as degraded-mode fallback.

### Issues

**EXT-01 — Three of four placement types have no rendering implementation**

Of the four defined placement types, only `page` is operational:

- `panel` — `getPanelExtensions` filter helper exists, but it is never called in any reviewed route. None of the reviewed page routes render any panel extension component.
- `context_menu_group` — `getContextMenuExtensions` helper exists but is never called. Context menus in Services and Software pages are static with no slot for extension-injected items.
- `table_columns` — `getTableExtensions` helper exists but is never called. No table queries it to append columns.

Extension authors who declare these placements will see nothing rendered. This is a critical gap for the declared extension model.

**EXT-02 — `wizard` UI type is defined but unimplemented**

`ActionDef.ui` supports `{ type: 'wizard'; steps: WizardStep[] }` in `types.ts:828`. `ActionButton.svelte` only handles `type === 'form'`. A backend extension declaring a wizard action will render a button that opens no modal and silently fires the action or does nothing.

**EXT-03 — No version field on extension manifests**

`ExtensionManifest` and `ExtensionResponse` have no `api_version`, `schema_version`, or `min_frontend_version` field. When a new `ExtensionUi` type is introduced, extensions built against older schemas will silently render nothing with no error message. The `[id]/+page.svelte` template has no `{:else}` fallback branch for unknown UI types.

**EXT-04 — No refresh mechanism after initial load**

Extensions are loaded once on login. If a backend service starts providing new extensions after a user is already logged in, the user must log out and back in. There is no `extensions_changed` SSE event and no background polling.

**EXT-05 — All `SchemaForm` field values stored as strings**

`let values: Record<string, string> = $state({})` stores toggle (boolean), number, and select values all as strings. For `invokeExtensionAction` (the extension proxy path), the backend receives `{"myNumber": "42"}` as a string. Extension authors on the proxy path must handle string coercion on the backend. Only `api_submit` path actions benefit from the `applyTemplate` coercions.

**EXT-06 — No cascading/dependent selects in `SchemaForm`**

`select_source` options are fetched once at form-open time and never refreshed. If two fields have a dependency (field B's options depend on field A's selection), there is no mechanism for cascading lookups.

**EXT-07 — `SchemaTable` uses row index as `{#each}` key**

`{#each rows as row, i (i)}` — Svelte DOM diffing is positional. If the backend returns rows in a different order after an action, transitions are incorrectly applied to the wrong rows. Rows with checkboxes or focused inputs lose their DOM identity.

**EXT-08 — `SchemaKeyValue` displays raw object keys as labels**

`Object.entries(data)` renders keys directly (e.g. `"created_at"`) without any label formatting. The backend must send pre-labelled keys or accept that raw key names will be shown to users.

**EXT-09 — `ServiceSelector` swallows load errors silently**

`catch { providers = []; }` in `ServiceSelector.svelte:23` — a network error looks identical to "no providers available". No error is shown to the user or logged.

**EXT-10 — Plugin configs use raw JSON textarea with no schema-driven editor**

`plugin-configs/+page.svelte` presents a `<textarea>` pre-populated with `JSON.stringify(config.config, null, 2)`. There is no typed form for plugin config fields. The existing `SchemaForm` infrastructure could provide this, but is not applied here.

**EXT-11 — Limited field type support in `SchemaForm`**

Supported: `text`, `password`, `number`, `textarea`, `select` (static + `rest_api`), `toggle`, `hidden`. Missing: `date`, `datetime`, `file`, `multiselect`, `radio group`, dependent selects, regex-validated text, number range.

**EXT-12 — Typed cell rendering not supported in `SchemaTable`**

All row values render as `String(row[col.key] ?? '')`. There is no way to render a cell as a badge, link, formatted date, boolean indicator, or truncated text. The `TableColumn.sortable` field exists in `types.ts` but is unused.

**EXT-13 — Extension permission check missing on page-type extensions**

`ExtensionManifest` includes `required_permission?: string`, and page extensions are filtered into navigation in `+layout.svelte`, but the permission is never checked. Built-in nav items filter by permission, but extension nav items skip this check. The extension detail page at `extensions/[id]/+page.svelte` also renders content without any permission guard. See also [SEC-09](#sec-09--low-extension-navigation-items-rendered-without-required_permission-check).

**EXT-14 — Action `timeout_seconds` declared but never enforced**

`ActionDef` includes `timeout_seconds?: number`, but `ActionButton.svelte` uses no abort signal or timeout mechanism. Both calls rely on the global 30-second timeout. An extension declaring `timeout_seconds: 300` for a long-running operation would still be cut off at 30 seconds.

**EXT-15 — `SchemaKeyValue` runs `loadData()` on every render via unconditional `$effect`**

The `$effect` calls `void loadData()` without any dependency guard or deduplication. If props change rapidly, multiple concurrent fetches fire without cancellation.

**EXT-16 — No error boundary around extension rendering**

`extensions/[id]/+page.svelte` renders extension UI components directly with no error boundary. If any component throws during rendering (malformed manifest data, null `ui.columns`), the entire route crashes. Each extension component should be wrapped in a containment boundary.

**EXT-17 — `applyTemplate` only handles full-field placeholders, not inline interpolation**

The regex `^\{\{(\w+)(?::(\w+))?\}\}$` requires the placeholder to be the entire string. A template value like `"prefix-{{name}}"` would not substitute `name`. Either document the limitation or support partial substitution.

**EXT-18 — Navigation does not render extension icons**

Page placement includes `icon?: string`, but the layout sidebar renders only `item.label`. The icon field is not propagated to the merged nav item structure. Extensions providing icons have them silently ignored.

**EXT-19 — Extension state is a module-level singleton with no tenant/session scoping**

`extensions.svelte.ts` stores extensions in module-level `$state`. `clearExtensions()` is called on logout, but if a user switches tenants without full reload, stale extensions persist. No mechanism to detect server-side extension changes during a session.

**EXT-20 — `nav_section` field defined but unused**

Page placement includes `nav_section: string` intended to group extensions. The layout sidebar renders a flat list with no section grouping. Extensions specifying `nav_section` see no effect.

**EXT-21 — No validation of extension manifest structure at load time**

`listExtensions()` deserializes the server response directly to `ExtensionResponse[]` with no runtime validation. Malformed manifests (missing `ui`, unknown `placement.type`, null columns) surface as runtime crashes deep in rendering components rather than being caught at the boundary.

---

## 8. Code and Logic Consistency

### Strengths

- All API calls go through `api.ts`. No route file uses raw `fetch()`.
- All list routes use the `Pagination` component (except `EnrollmentTokenSettings` — see below).
- All date rendering calls `formatDate()` from `$lib/utils.ts`.
- All `{#each}` blocks have `{:else}` branches with descriptive empty-state messages.
- All context menus use `ContextMenu` and are closed before any action is initiated.
- Filter values are synchronised to URL parameters using `parseUrlParam()` / `parseUrlPage()` from `$lib/utils.ts`.
- All list pages implement at least one refresh mechanism (polling interval or SSE subscription).
- `onDestroy` consistently clears intervals and SSE subscriptions.
- Filter tab buttons use a consistent visual pattern: `btn btn-sm` with `preset-filled-primary-500` for active and `preset-tonal` for inactive.
- Consistent `$effect` cleanup for intervals with `onDestroy` as a safety net.

### Issues

**CON-01 — High: Broken host select in trigger update modal**

`history/+page.svelte:369-378` — the trigger update modal contains a `<select>` element populated with only an empty default option (no `{#each}` over a hosts list). Below it is a text `<input>` for manual UUID entry as a workaround. The select has no functional purpose.

**CON-02 — High: `AssignToHostModal.svelte` hardcoded page size limits (also ARCH-02)**

Silent truncation at 200 hosts / 500 plugin configs with no indication.

**CON-03 — High: No `ConfirmDialog` on token revoke in `EnrollmentTokenSettings` and `SystemServicesSettings`**

Irreversible actions without confirmation. Consistent with the revoke/destructive pattern established in peer pages which do use `ConfirmDialog`.

**CON-04 — High: `ActionButton` destructive actions have no confirmation**

The `action.destructive` flag changes button styling to error colours but does not trigger a `ConfirmDialog`. Extension-defined destructive actions execute immediately on click.

**CON-05 — Medium: `services/+page.svelte` and `system-services/+page.svelte` use inline error state, no success toasts**

Both pages set `error = e.message` directly for action failures (rendered inline above the table) and show no `showSuccess()` call for successful actions. All other pages use `showError()` / `showSuccess()` from the notifications store. Users performing service actions get no positive confirmation of success.

**CON-06 — Medium: `hosts/+page.svelte` mixes inline error and toast patterns**

`executeEdit()` sets `error = ...` inline for failures, while `triggerDiscovery()` uses `showSuccess()` / `showError()`. Two error patterns on the same page.

**CON-07 — Medium: `EnrollmentTokenSettings` pagination is text-only with no navigation**

Renders "Page X of Y (N total)" as static text with no navigation controls. All other paginated surfaces use `<Pagination>`. Users cannot navigate to additional pages.

**CON-08 — Medium: `SystemServicesSettings.svelte` heading mislabelled "System Services" (also ARCH-07)**

The dedicated system services page (`/system-services`) already uses "System Services". Users in global settings see a second "System Services" section that actually manages enrollment tokens.

**CON-09 — Medium: Filter auto-apply behaviour is inconsistent across pages**

- `hosts/[id]/packages/+page.svelte`: select filters auto-apply on change, search auto-applies with debounce.
- `audit-logs/+page.svelte`: all filters require an explicit "Apply" button click.
- `services/+page.svelte`, `system-services/+page.svelte`: status filters are immediate tab-style buttons.

No unified convention for when filters apply immediately vs. on explicit action.

**CON-10 — Medium: Status badge/label inconsistency for the same update status values**

- `history/+page.svelte` `statusBadgeClass()`: `'queued'` -> `'preset-tonal-surface'`
- `hosts/[id]/+page.svelte` badge rendering: `'queued'` -> `'preset-tonal'`

`preset-tonal` and `preset-tonal-surface` render differently. Status label strings also differ between the two surfaces for `in_progress` (`'Running'` vs raw value).

**CON-11 — Medium: `history/+page.svelte` uses `<ModalBackdrop>` directly instead of `<Modal>` (also ARCH-12)**

**CON-12 — Low-Medium: Deactivated system services open an empty context menu**

`system-services/+page.svelte:380-391` — the actions button renders for `canManage` users even for deactivated services, but clicking it opens a `ContextMenu` with no items.

**CON-13 — Low: `table-container` vs `table-wrap` CSS class (also STD-S2)**

`EnrollmentTokenSettings.svelte:234` and `SystemServicesSettings.svelte:207`.

**CON-14 — Low: `listUpdateHistory` falsy page check skips page `0`**

`api.ts:833` uses `if (opts?.page)` instead of `if (opts?.page != null)` like all other paginated functions.

**CON-15 — Low: `MqttClientsSettings.svelte` passes compound class string to `Modal` `maxWidth` prop**

`maxWidth` receives `"max-w-2xl max-h-[90vh] overflow-y-auto"` — a compound string including height and overflow rules. The prop is documented as a single width class.

**CON-16 — Medium: No submitting guard on `saveNetworkSettings` in global settings**

`settings/global/+page.svelte` — the `saveNetworkSettings()` function has no `saving` guard and the Save button has no `disabled` attribute. Double-clicking fires duplicate API calls. Compare with `saveNatsUrl()` on the same page which correctly tracks `natsSaving` state.

**CON-17 — Medium: No submitting guard on `saveRegistration`, `saveAuthentication`, and `saveCertificates`**

Three settings save functions lack double-submit protection. Other save flows in the codebase (scheduler `saveEdit`, MQTT, OIDC, system-services) all use a `submitting`/`saving` guard.

**CON-18 — Medium: Scheduler page does not subscribe to `scheduler_task_completed` SSE event (also HA-19)**

The event type exists in the SSE event store but is never subscribed to. Stale `last_run_at` and `next_run_at` values shown until manual refresh.

**CON-19 — Medium: `OidcProvidersSettings` and `MqttClientsSettings` modals use inline `<div>` footer instead of `{#snippet footer()}` pattern**

Other modals (scheduler, system-services ping dialog, host edit) use the `{#snippet footer()}` slot. These inline footers scroll with content instead of being fixed at the modal bottom.

**CON-20 — Medium: Audit logs tab bar uses border-underline styling while all other filter tabs use button styling**

`audit-logs/+page.svelte` uses `border-b-2` underline styling. Every other page with tab-like filters uses `btn btn-sm` with `preset-filled-primary-500` / `preset-tonal` toggling.

**CON-21 — Low: Inconsistent URL parameter encoding in `$effect` URL sync blocks**

`audit-logs/+page.svelte` applies `encodeURIComponent()` to URL parameter values. All other pages use raw string interpolation. No clear convention for whether values should be encoded.

**CON-22 — Low: Login and register forms have no submitting guard preventing double-submission**

`login/+page.svelte` `onSubmit`, `onLinkWithPassword`, `onSubmitRegistrationToken`, and `register/+page.svelte` `onSubmit` all lack a `submitting` guard. The OIDC login on the same page correctly manages `oidcLoading` state.

**CON-23 — Low: `services/+page.svelte` and `system-services/+page.svelte` use `$effect` for initial data load while most pages use `onMount`**

Creates a subtle behavioral difference. The `$effect` pattern creates a new `setInterval` on every filter change (old one cleaned up by return function). Other pages use `onMount` for initial load with explicit function calls for filter changes.

---

## 9. Maintainability

### Strengths

- **Centralized API layer** — all backend communication flows through `api.ts`. Adding an endpoint is a one-function, one-type-interface operation.
- **Centralized type definitions** — `types.ts` is the single source of truth for every request and response shape.
- **Svelte 5 rune adoption is complete** — no Svelte 4 store patterns in lib code.
- **SSE resource management is consistent** — every page registers subscriptions in `unsubscribers[]` and tears down in `onDestroy`.
- **Build configuration is minimal and clean** — `vite.config.ts` (23 lines), `svelte.config.js` (39 lines), `tsconfig.json` (15 lines). Not over-engineered.
- **All dependencies are on current major versions** — Svelte 5, SvelteKit 2, Tailwind v4, Skeleton v4, Vite 7, Vitest 4. The project is already on the new Tailwind v4 paradigm (no `tailwind.config.js`).
- **Complex flows are commented at decision points** — token refresh deduplication, CSP hash mode rationale, MQTT polling jitter, and `initiatedKeys` non-reactive trick all have inline explanations.
- **Coverage thresholds are enforced** on `src/lib/**`.
- **Well-decomposed settings page** — seven focused sub-components with uniform `{settings, onSuccess, onError}` props contract.
- **Token display security pattern** — both enrollment token components follow the one-time-display pattern with "copy it now" warning.
- **Consistent component prop contracts** using typed `$props()` destructuring with inline TypeScript annotations across 30+ components.

### Issues

**MAINT-01 — High: No API contract enforcement**

All types in `types.ts` are hand-maintained mirrors of Rust backend types. There is no code generation, no OpenAPI spec consumption, and no runtime validation (`zod` or equivalent) at the API boundary. `request<T>()` is a trusted cast. A backend field rename (e.g. `friendly_name` -> `display_name` in `HostResponse`) breaks at minimum: `types.ts`, `api.ts`, and all consuming route and component files with no automated detection.

**MAINT-02 — High: `$app/stores` deprecation creates a scheduled breaking change (also STD-S1)**

When `$app/stores` is removed from SvelteKit, `+layout.svelte`, `login/+page.svelte`, `software/[id]/+page.svelte`, and `extensions/[id]/+page.svelte` will all break simultaneously. Migration should happen proactively.

**MAINT-03 — High: `AdminEventType` manually mirrors backend SSE event emission**

`sse.ts:184-198` contains a manually-maintained union of event type strings. If the backend adds or renames an event type, the frontend receives an unchecked cast value and subscriptions silently never fire. No automated detection.

**MAINT-04 — High: No tests for `sse.ts` and `events.svelte.ts`**

These are non-trivial implementations (reconnection backoff, debouncing, subscription lifecycle) with zero test coverage. A regression here silently breaks real-time page updates across the entire application.

**MAINT-05 — Medium: Non-functional extension `form` page type and missing `wizard` action type**

`extensions/[id]/+page.svelte` renders a form with a no-op submit handler. `ActionButton.svelte` has no wizard rendering path. These are shipped gaps that manifest as user-facing bugs when backend-configured extensions use these UI types.

**MAINT-06 — Medium: Open union types in extension schema defeat TypeScript exhaustiveness**

`FieldType` is `'text' | ... | string` (`types.ts:786`) and `PanelPosition.type` is `'tab' | 'below' | 'above' | string`. The open `| string` tail makes the discriminant useless — TypeScript cannot narrow on it. Each new backend field type or placement type requires a manual exhaustiveness audit of all rendering code.

**MAINT-07 — Medium: `types.ts` (912 lines) is a flat monolith**

All request/response types share a single file with no domain grouping. As the API grows, this file grows linearly. Extension types (lines 758-912) are already complex enough to warrant their own module. Domain-splitting into `types/auth.ts`, `types/host.ts`, `types/software.ts`, `types/extensions.ts` would reduce onboarding friction significantly.

**MAINT-08 — Medium: Skeleton UI class names pervasive with no abstraction layer**

`preset-*` classes appear across every template. A Skeleton major version upgrade will require template-level changes in nearly every file. The partial abstractions (`agentStatusClass()`, `historyStatusClass()`) exist only in single route files, not as shared utilities. A shared `statusBadgeClass(status, type)` utility would make a Skeleton upgrade a single-file change.

**MAINT-09 — Medium: Auth guard pattern is undocumented**

Instead of SvelteKit's standard `+layout.ts` load guards (unavailable with `adapter-static` + SSR disabled), auth is enforced in `+layout.svelte` via a `$effect`. A new developer familiar with standard SvelteKit patterns will look for a `+layout.ts` guard and not find it. The `publicRoutes` constant is a hardcoded set that is non-obvious to update.

**MAINT-10 — Low: `HostPackageDetailResponse.recent_updates` is typed but has no rendering path**

`getHostPackage()` in `api.ts:410` is typed and exposed, but `hosts/[id]/packages/+page.svelte` never calls it. The per-package detail (including update history) is dead code in the current UI.

**MAINT-11 — Low: No error tracking or structured production logging**

There is no error tracking integration (Sentry, etc.) and no structured logging. Background refresh failures are silently suppressed. The MQTT polling loop explicitly uses `catch { // Suppress polling errors }`. Intermittent backend connectivity issues are invisible to operators.

**MAINT-12 — Low: Navigation priority numbers have no documented convention**

Built-in nav items use priorities 100, 200, ..., 1100 with no documented convention for gaps. A developer inserting a new item must choose a priority value without guidance.

**MAINT-13 — High: Duplicated token management components**

`SystemServicesSettings.svelte` is a near-identical copy of `EnrollmentTokenSettings.svelte`. Functions `loadTokens`, `handleCreate`, `handleRevoke`, `handleCopy`, `resetForm`, `formatUsage`, `tokenStatus`, `statusBadgeClass` are duplicated verbatim or with trivial name changes. Template structure is structurally identical. Should be refactored into a single generic `TokenManagement.svelte` component. Any bug fix or UX improvement must be applied in two places.

**MAINT-14 — Medium: Extension components lack test coverage entirely**

Five extension components (`ActionButton`, `SchemaForm`, `SchemaTable`, `SchemaKeyValue`, `ServiceSelector`) have zero test files. This is the most architecturally complex part of the frontend with dynamic form rendering, JSON body templating, paginated REST-sourced selects, and type coercion logic. The shared UI components have thorough tests, making this gap conspicuous.

**MAINT-15 — Medium: `location.pathname` used instead of SvelteKit router state (also STD-S5)**

Eight route pages use `location.pathname` directly, bypassing SvelteKit's routing abstraction. This creates coupling to the browser's `location` API that would break in SSR or testing contexts.

**MAINT-16 — Low: `OidcProvidersSettings` does not validate logo URL before submission**

The template shows a validation error when `isValidLogoUrl` returns false, but `saveOidcProvider` does not check before submitting. The visual error is purely informational — invalid URLs can still be submitted.

**MAINT-17 — Low: E2E test files are stubs without assertions**

`tests/e2e/` contains `auth.test.ts`, `hosts.test.ts`, and `services.test.ts` as placeholders. `playwright.config.ts` configures only Chromium. Placeholder files provide false sense of completeness.

**MAINT-18 — Low: OIDC role mapping edited as raw JSON without structural validation**

The OIDC role mapping textarea accepts any valid JSON. `JSON.parse` validates syntax but not the expected `Record<string, string>` shape. Arrays, numbers, and deeply nested objects are all accepted.

---

## Cross-Cutting Summary

### Highest-Priority Fixes

| Priority | Issue                                                                | Location                                                          |
| -------- | -------------------------------------------------------------------- | ----------------------------------------------------------------- |
| Critical | SSE never refreshes token on 401 — loops forever                     | `sse.ts:222-239`                                                  |
| Critical | SSE does not recover after re-login                                  | `stores/events.svelte.ts:60-74`                                   |
| Critical | No tests for `sse.ts` reconnection/parsing logic                     | `src/lib/sse.ts`                                                  |
| High     | Extension form submit is a no-op                                     | `extensions/[id]/+page.svelte`                                    |
| High     | Token revoke without confirmation                                    | `EnrollmentTokenSettings.svelte`, `SystemServicesSettings.svelte` |
| High     | `AssignToHostModal` silently truncates at 200/500                    | `AssignToHostModal.svelte`                                        |
| High     | Audit log tab default ignores user permissions                       | `audit-logs/+page.svelte`                                         |
| High     | Broken host select in history trigger modal                          | `history/+page.svelte:369-378`                                    |
| High     | `$app/stores` -> `$app/state` migration (6 files)                    | See STD-S1                                                        |
| High     | No tests for `events.svelte.ts` subscription lifecycle               | `stores/events.svelte.ts`                                         |
| High     | Duplicated token management components                               | `EnrollmentTokenSettings`, `SystemServicesSettings`               |
| High     | Extension permission check missing                                   | `+layout.svelte`, `extensions/[id]/+page.svelte`                  |
| Medium   | Missing `encodeURIComponent` on ~13 API functions                    | `api.ts`                                                          |
| Medium   | `SchemaForm` discards user input on referential `fields` change      | `SchemaForm.svelte:34-45`                                         |
| Medium   | Services/system-services no success toasts, inline error not cleared | `services/+page.svelte`, `system-services/+page.svelte`           |
| Medium   | `datetime-local` values not converted to RFC 3339                    | `audit-logs/+page.svelte`                                         |
| Medium   | Output SSE stream replays duplicate lines on reconnect               | `sse.ts:90-113`                                                   |
| Medium   | `auth.svelte.ts` stuck at loading on network error at startup        | `auth.svelte.ts:45-66`                                            |
| Medium   | Dynamic Tailwind class construction may be purged from production    | `MqttClientsSettings.svelte`                                      |
| Medium   | `release_url` rendered as clickable href without protocol validation | `software/[id]/+page.svelte`                                      |
| Medium   | Multiple save buttons lack double-submit guards                      | Settings components, global settings, login/register              |
| Medium   | Scheduler page has no data refresh mechanism                         | `scheduler/+page.svelte`                                          |

### Quick Wins (low effort, high clarity)

| Fix                                                             | Location                                                          |
| --------------------------------------------------------------- | ----------------------------------------------------------------- |
| Move xterm.js to `dependencies`                                 | `package.json`                                                    |
| Extract `getCredentialWarnings` to `utils.ts`                   | `services/`, `system-services/`                                   |
| Add `void` to `fetchAlerts()`                                   | `+layout.svelte`                                                  |
| Replace `console.error` with `showError()`                      | `extensions.svelte.ts:20`                                         |
| Fix `table-container` -> `table-wrap`                           | `EnrollmentTokenSettings.svelte`, `SystemServicesSettings.svelte` |
| Add `showError()` to `ServiceSelector` catch block              | `ServiceSelector.svelte:23`                                       |
| Fix `listUpdateHistory` falsy page check                        | `api.ts:833`                                                      |
| Fix non-null assertion `$state(undefined!)`                     | `ContextMenu.svelte:17`                                           |
| Document `publicRoutes` auth guard pattern                      | `+layout.svelte`                                                  |
| Add `role="alert"` to inline error asides                       | ~20 locations across routes                                       |
| Use `SvelteSet` instead of plain `Set` with reconstruction      | `+layout.svelte`, `software/+page.svelte`, `hosts/`               |
| Use `SvelteMap` -> plain `Map` for non-reactive debounce timers | `stores/events.svelte.ts`                                         |
| Add `aria-label` or `<caption>` to tables                       | ~22 `<table>` elements                                            |
| Fix `apiSubmitRequest` return type to generic `<T>`             | `api.ts:1015-1022`                                                |
