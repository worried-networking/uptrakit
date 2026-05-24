# Settings UI/UX Improvements

**Date:** 2026-05-24 **Status:** Spec — awaiting implementation plan

---

## Overview

Three coordinated improvements to the Settings area:

1. **Navigation** — add a dedicated "MCP Access" tab (replacing the buried button in General), positioned immediately after General.
2. **Form standardization** — all editable forms on the General tab adopt the draft-pattern: Save disabled when unchanged, Discard button when dirty,
   accent left-border on changed fields, label width fix.
3. **Backend merge** — the separate `/api/v1/settings/registration` and `/api/v1/settings/authentication` endpoints collapse into a single
   `/api/v1/settings/access` endpoint; CLI follows.

No backwards compatibility is required for any removed endpoints or routes.

---

## 1. Tab Navigation

### MCP Access tab

- Add `mcp-access` to `BUILTIN_TAB_IDS` in `settings/+page.svelte`.
- Insert "MCP Access" tab immediately after "General" in `tabItems`. Guard: `canManageOAuthClients` (`ManageAuthSettings` permission), same as the
  current OAuth Clients button.
- The tab panel renders a new `McpAccessTab.svelte` component (see §3.4).
- Add `{:else if activeTab === 'mcp-access'}` branch to the tab render chain in `+page.svelte`, positioned after the `general` branch.
- The tab id `mcp-access` is reflected in the URL as `?tab=mcp-access`.

### Removed route

Delete the entire `frontend/src/routes/settings/authentication/` directory including:

- `oauth-clients/+page.svelte`
- `oauth-clients/RegisterClientDialog.svelte`

No redirect. No backwards compatibility.

---

## 2. Form Standardization

### 2.1 Label width fix

**Root cause:** `@plugin '@tailwindcss/forms'` (base strategy, in `app.css`) injects `label { display: block; }` globally. The `<label for={inputId}>`
inside `FormFieldRow.svelte` therefore spans the full 20 rem label column, so clicking empty space beside the label text fires the associated control.

**Fix:** Add `class="w-fit"` to the `<label>` element in `FormFieldRow.svelte`. This constrains label hit area to text width only. No other changes to
`FormFieldRow` layout.

### 2.2 Changed field highlight

Add an optional `dirty` prop to `FormFieldRow`:

```svelte
let {
  label,
  hint,
  error,
  inputId,
  required = false,
  dirty = false,   // ← new
  children
}: { ...; dirty?: boolean; children: Snippet } = $props();
```

When `dirty` is true, the outer grid `div` gains `border-l-2 border-[var(--accent)] pl-2`. The accent border runs the full height of the field row,
clearly marking which fields differ from the server-committed value.

The `pl-2` padding compensates for the 2px border so the label column does not visually shrink — the grid columns remain stable.

Callers pass `dirty={form.isFieldDirty('someField')}`.

### 2.3 Shared draft utility: `createFormDraft`

**Location:** `frontend/src/lib/forms/draft.svelte.ts`

A single reusable Svelte 5 reactive utility that encapsulates the draft pattern. All editable settings forms import this instead of reimplementing the
state logic.

```typescript
// frontend/src/lib/forms/draft.svelte.ts

export interface FormDraft<T extends Record<string, unknown>> {
  readonly draft: T;
  readonly serverValues: T;
  readonly isDirty: boolean;
  isFieldDirty(key: keyof T): boolean;
  update<K extends keyof T>(key: K, value: T[K]): void;
  load(values: T): void;
  commit(updated: T): void;
  discard(): void;
}

export function createFormDraft<T extends Record<string, unknown>>(initial: T): FormDraft<T> {
  let serverValues = $state<T>({ ...initial });
  let draft = $state<T>({ ...initial });

  const isDirty = $derived((Object.keys(serverValues) as (keyof T)[]).some((k) => draft[k] !== serverValues[k]));

  return {
    get draft() {
      return draft;
    },
    get serverValues() {
      return serverValues;
    },
    get isDirty() {
      return isDirty;
    },
    isFieldDirty(key) {
      return draft[key] !== serverValues[key];
    },
    update(key, value) {
      draft[key] = value;
    }, // property mutation — preserves Svelte 5 fine-grained reactivity
    load(values) {
      serverValues = { ...values };
      draft = { ...values };
    },
    commit(updated) {
      serverValues = { ...updated };
      draft = { ...updated };
    },
    discard() {
      draft = { ...serverValues };
    },
  };
}
```

**Assumptions:** All tracked fields are primitive values (string, number, boolean). This covers all current settings forms. Forms with nested objects
or arrays must compare those fields explicitly and are out of scope.

**Reactivity warning:** Do not destructure the return value of `createFormDraft`. `const { draft } = form` takes a snapshot at destructure time —
`draft` will not update when the underlying `$state` changes. Always access through `form.draft`, `form.isDirty`, `form.isFieldDirty(...)`, etc. This
applies everywhere the draft is used: component `<script>` blocks, `$derived` expressions, and templates.

**Usage in a component:**

```svelte
<script lang="ts">
  import { createFormDraft } from '$lib/forms/draft.svelte';

  // Initialise with empty/default values; populated via load() when server data arrives
  const form = createFormDraft({ mode: 'open', passwordAuthEnabled: true, ... });

  $effect(() => {
    if (settings) form.load({ mode: settings.mode, ... });
  });

  async function save() {
    const res = await updateAccessSettings({ ...form.draft });
    form.commit({ mode: res.mode, ... });
  }
</script>

<FormFieldRow label="Registration Mode" dirty={form.isFieldDirty('mode')}>
  ...
</FormFieldRow>

<Button disabled={!form.isDirty || saving} onclick={save}>Save</Button>
{#if form.isDirty}
  <Button variant="ghost" onclick={form.discard}>Discard</Button>
{/if}
```

Export `createFormDraft` from `frontend/src/lib/forms/index.ts` (create file if absent). The canonical import path is `$lib/forms/draft.svelte` or
`$lib/forms`. Do **not** re-export from `$lib/components/forms/index.ts` — that barrel is for UI components only; mixing a utility factory into it
breaks tree-shaking boundaries.

**Rolling out to other tabs:** Any future settings form adopts the pattern by calling `createFormDraft(defaults)` and wiring `load` / `commit` /
`discard`. No copy-paste of state logic required.

**Scope on the General tab (this spec):**

| Form                                   | Gets draft pattern                         |
| -------------------------------------- | ------------------------------------------ |
| Registration & Authentication (merged) | ✅                                         |
| Agent Certificates                     | ✅                                         |
| OIDC Providers                         | ✗ list-based (add/remove actions, no Save) |
| Enrollment Tokens                      | ✗ list-based                               |
| Danger Zone                            | ✗ destructive actions only                 |

**MCP Access tab** (the lifted OAuth Clients page) migrates its ad-hoc draft to `createFormDraft` as part of this spec — see §3.4.

---

## 3. Component Changes

### 3.1 New: `RadioCardGroup.svelte`

Location: `frontend/src/lib/components/forms/RadioCardGroup.svelte`

Horizontal card-tile selector for mutually exclusive options. No radio button indicators — selection is conveyed by accent border + background tint
only.

**Props:**

```typescript
type RadioCardOption<T extends string> = {
  value: T;
  label: string;
  description?: string;
};

let {
  name,
  value, // currently selected value
  options, // array of RadioCardOption
  onchange, // (value: T) => void
  disabled = false,
}: {
  name: string;
  value: string;
  options: RadioCardOption<string>[];
  onchange?: (value: string) => void;
  disabled?: boolean;
} = $props();
```

**Rendering:** `display: grid; grid-template-columns: repeat(N, 1fr)` where N = `options.length`. Each card:

- Unselected: `border border-[var(--border-subtle)] bg-transparent text-[var(--text-secondary)]`
- Selected: `border-2 border-[rgba(var(--accent-rgb),0.6)] bg-[rgba(var(--accent-rgb),0.07)] text-[var(--accent-bright)]`
- Transition: `transition-[background,border-color,color] duration-fast` (canonical triplet from `--duration-fast` token)
- Keyboard: each card is a `<button type="button">`. Arrow keys cycle selection (same pattern as `TabStrip`).
- Accessibility: `role="radiogroup"` on container; each card has `role="radio"`, `aria-checked`, `aria-label`. (`aria-checked` is only valid on
  elements with an explicit `role="radio"`; omitting it is an ARIA authoring error.)

Export from `frontend/src/lib/components/forms/index.ts`.

### 3.2 New: `AccessSettings.svelte`

Location: `frontend/src/routes/settings/AccessSettings.svelte`

Replaces the two separate `RegistrationSettings.svelte` and `AuthenticationSettings.svelte` components. Single `SectionCard` titled **"Registration &
Authentication"**.

**Fields:**

| Field                   | Control                                        |
| ----------------------- | ---------------------------------------------- |
| Registration Mode       | `RadioCardGroup` — Open / Invite Only / Closed |
| Registration Token      | `Input` — visible only when mode = `invite`    |
| OIDC First Login        | `Checkbox` — visible only when mode = `invite` |
| Password Authentication | `Checkbox`                                     |
| Require Two-Factor Auth | `Checkbox`                                     |

**Draft state shape:**

```typescript
interface AccessDraft {
  mode: "open" | "invite" | "closed";
  token: string; // transient; not round-tripped from server
  requireTokenForOidc: boolean;
  passwordAuthEnabled: boolean;
  twoFactorRequired: boolean;
}
```

`token` is always empty string on load (server never returns it).

**`isDirty` override:** `token` is excluded from the global dirty check because it is always "new input" (empty string on load, never equal to a
server value). `createFormDraft.isDirty` would permanently return true once the user types a token. Instead, `AccessSettings.svelte` defines its own
`isDirty` derived that excludes `token`:

```typescript
const form = createFormDraft<AccessDraft>({
  mode: "open",
  token: "",
  requireTokenForOidc: false,
  passwordAuthEnabled: true,
  twoFactorRequired: false,
});

const isDirty = $derived(
  form.isFieldDirty("mode") ||
    form.isFieldDirty("requireTokenForOidc") ||
    form.isFieldDirty("passwordAuthEnabled") ||
    form.isFieldDirty("twoFactorRequired"),
  // token intentionally excluded
);
```

**API call on Save:** `PUT /api/v1/settings/access` (see §4.2). Passes `etag` stored from the last `getAccessSettings()` call. On success,
`form.commit({ ...response, token: '' })` — the spread preserves the full `AccessDraft` shape (including `token: ''`) since `AccessSettingsResponse`
has no `token` field and `commit` replaces the entire draft state.

**Dirty per-field:** each `FormFieldRow` receives `dirty={form.isFieldDirty('fieldName')}` where applicable. Token field always receives
`dirty={false}` — it is new input, not a change of persisted state.

**Props:**

```typescript
let {
  onSuccess,
  onError,
}: {
  onSuccess: (msg: string) => void;
  onError: (msg: string) => void;
} = $props();
```

`AccessSettings.svelte` manages its own data load via `getAccessSettings()` on mount (returns `{data, etag}`). The parent page's combined load is no
longer passed as a prop — this avoids ETag ownership ambiguity. The combined load still populates the rest of the General tab; `AccessSettings` is
self-contained.

### 3.3 Modified: `AgentCertificateSettings.svelte`

Add draft pattern. `serverValues` captures `lifetime_days`, `renewal_window_hours_override`, `effective_renewal_window_hours` from the server
response. `isDirty` compares the resolved fields. Discard button shown when dirty.

No changes to the fields, layout, or API call shape.

### 3.4 New: `McpAccessTab.svelte`

Location: `frontend/src/routes/settings/McpAccessTab.svelte`

Content extracted from the deleted `authentication/oauth-clients/+page.svelte`, with these adjustments:

- Remove `PageShell` wrapper — the tab already lives inside the main settings `PageShell`.
- The `RegisterClientDialog` component moves to `frontend/src/routes/settings/RegisterClientDialog.svelte`.
- The ad-hoc draft state (`draft`, `oauthSettings`, `isDirty`, `handleDiscard`) is replaced with `createFormDraft`.

**Migration of OAuth settings draft:**

The existing `DraftOAuthSettings` interface and manual `isDirty` derived become:

```typescript
interface OAuthSettingsDraft {
  mcp_enabled: boolean;
  dcr_enabled: boolean;
  cimd_enabled: boolean;
  canonical_host: string | null;
}

const oauthDraft = createFormDraft<OAuthSettingsDraft>({
  mcp_enabled: false,
  dcr_enabled: false,
  cimd_enabled: false,
  canonical_host: null,
});
```

`loadOAuthSettings` calls `oauthDraft.load(...)` on success. `handleDiscard` becomes `oauthDraft.discard()`. `handleSettingsChange` calls
`oauthDraft.commit(response)` on success. `isDirty` is replaced by `oauthDraft.isDirty`. `canonical_host` comparison uses
`(draft.canonical_host ?? null) !== serverValues.canonical_host` — expose this as a custom field-level check since null coercion is not purely `!==`;
the `isFieldDirty` helper does not handle this edge case. Use `oauthDraft.draft.canonical_host !== oauthDraft.serverValues.canonical_host` directly
(both are already `string | null` after migration, no coercion needed).

`oauthSettingsEtag` remains separate `$state` — it is request metadata, not a form field.

**`canonical_host` coercion on save:** The input field emits an empty string `''` when the user clears the field, but the server expects `null` to
mean "no canonical host". The save handler must explicitly map this before calling the API:

```typescript
canonical_host: oauthDraft.draft.canonical_host === "" ? null : oauthDraft.draft.canonical_host;
```

`isFieldDirty('canonical_host')` correctly detects the change (`'' !== null`), so dirty tracking requires no special handling — only the outgoing API
payload needs the mapping.

**Data loading:** `McpAccessTab` loads data lazily on component mount (same as the existing `+page.svelte` which calls `loadClients()` and
`loadOAuthSettings()` in `onMount`). Re-fetches on each tab activation (Svelte conditionally mounts/unmounts the tab panel on each tab switch). No
pre-fetch from the parent page.

**Permission gate:** the OAuth settings `SectionCard` (checkboxes + `canonical_host`) is guarded by `canManageGlobalSettings`
(`Permission.ManageGlobalSettings`) inside the tab — this dual-permission gate is preserved verbatim from the existing page. Users with
`ManageAuthSettings` but not `ManageGlobalSettings` see the client list but not the settings form.

### 3.5 Modified: `settings/+page.svelte`

- Add `'mcp-access'` to `BUILTIN_TAB_IDS`.
- In `tabItems`, insert `{ id: 'mcp-access', label: 'MCP Access' }` after `{ id: 'general', ... }`, guarded by `canManageOAuthClients`.
- Replace `<RegistrationSettings>` and `<AuthenticationSettings>` with `<AccessSettings ...>` (no `settings` prop — component self-loads).
- Remove the OAuth Clients `SectionCard` block (lines 324–331 in current file).
- Add `{:else if activeTab === 'mcp-access'}<McpAccessTab />{/if}`.
- Import `AccessSettings` and `McpAccessTab`; remove imports of `RegistrationSettings` and `AuthenticationSettings`.
- The page's combined load continues to use `GET /api/v1/settings/combined` for `agent_certificates`, `enrollment_tokens`, `multi_tenancy_enabled`. It
  no longer passes registration/auth data to `AccessSettings` (component self-loads). Remove destructuring of `combined.registration` and
  `combined.authentication`; remove `registrationSettings`, `authSettings` state variables and their error/retry handling from the page.

### 3.6 Deleted components

- `settings/RegistrationSettings.svelte`
- `settings/RegistrationSettings.test.ts`
- `settings/AuthenticationSettings.svelte`
- `settings/AuthenticationSettings.test.ts`

---

## 4. Backend Changes

### 4.1 New types in `web-api-types`

New file: `crates/shared/web-api-types/src/settings_access.rs`

```rust
#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(validator::Validate))]
pub struct UpdateAccessSettingsRequest {
    pub mode: RegistrationMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<SecretString>,  // SecretString: Debug output is masked — safe to derive Debug
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_token_for_oidc: Option<bool>,
    pub password_auth_enabled: Option<bool>,
    pub two_factor_required: Option<bool>,
}

#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AccessSettingsResponse {
    pub mode: RegistrationMode,
    pub require_token_for_oidc: bool,
    pub password_auth_enabled: bool,
    pub two_factor_required: bool,
}
```

`Validate` impl: the cross-field constraint (`mode != Invite` → `token` and `require_token_for_oidc` must be absent) cannot be expressed via
`validator::Validate` derive alone. Implement a custom `#[validate(custom(function = "validate_invite_fields"))]` function on the struct. This
function checks `mode != RegistrationMode::Invite && (token.is_some() || require_token_for_oidc.is_some())` and returns a `ValidationError` if
violated. Existing validation in the deleted registration handler used manual checks; this moves that logic into the type's `Validate` impl so the
handler just calls `req.validate()` and returns a 400 on failure.

### 4.2 New handler: `settings_access.rs`

Location: `crates/ui/web-api/src/routes/settings_access.rs`

**`GET /api/v1/settings/access`**

Returns `AccessSettingsResponse` with an `ETag` header (same `W/"settings-v{version}"` format as the former registration endpoint). Permission:
`CanViewSettings`.

**ETag flow:** The frontend's `AccessSettings.svelte` calls `getAccessSettings()` on mount to get both the current values and an ETag, independent of
the parent page's combined load. The combined load is still used to pre-fill data quickly, but `getAccessSettings()` is the authoritative source for
the ETag needed by PUT. The ETag is stored in local `$state` and passed on PUT (same pattern as `McpAccessTab` uses for `oauthSettingsEtag`).

**`PUT /api/v1/settings/access`**

Handler entry: call `req.validate()` immediately; return 400 on failure before any business logic.

**Transaction order (critical):** Safety checks that require reads must run _before_ opening the `BEGIN IMMEDIATE` transaction — holding an IMMEDIATE
lock while awaiting a pooled-connection read creates an unnecessarily long lock window and may deadlock. The correct sequence:

1. Run all pre-condition checks (OIDC provider query, session-type check for `password_auth_enabled`) on the existing connection.
2. Return early with a 4xx if any check fails.
3. Open one `BEGIN IMMEDIATE` transaction.
4. Write both registration settings and authentication settings inside that single transaction.
5. Bump `settings_version_cache` unconditionally (regardless of which fields changed) — the existing auth handler never bumped the cache, so auth-only
   changes would produce a stale ETag without this step.
6. Commit, then emit audit events.

- Permission: `CanManageAuthSettings`
- `IfMatch<SettingsVersion>` guard (same as existing endpoints)
- Emits separate audit events for registration and authentication changes (preserving existing audit granularity — one event per subsystem)
- Safety checks from auth endpoint preserved: cannot disable password auth while current session uses password; at least one auth method must remain
- Returns `AccessSettingsResponse`

### 4.3 Updated: `CombinedSettingsResponse`

In `settings_combined.rs`, remove the `registration` and `authentication` fields entirely. Do **not** replace them with an `access` field —
`AccessSettings.svelte` self-loads via `GET /api/v1/settings/access` and the combined endpoint no longer needs to carry that data:

```rust
pub struct CombinedSettingsResponse {
    pub agent_certificates: AgentCertificateSettingsResponse,
    pub enrollment_tokens: EnrollmentTokensSummary,
    pub multi_tenancy_enabled: bool,
}
```

Update `get_combined_settings` handler: remove the registration and authentication source queries. The combined endpoint is now used only for the
remaining General tab data (agent certificates, enrollment tokens, multi-tenancy). This eliminates the redundant fetch that would otherwise occur on
every page load (AccessSettings already issues its own GET).

### 4.4 Deleted backend items

- `GET /api/v1/settings/registration` and `PUT /api/v1/settings/registration`
- `GET /api/v1/settings/authentication` and `PUT /api/v1/settings/authentication`
- Handler file `crates/ui/web-api/src/routes/settings.rs` (contains only registration handlers; entire file deleted)
- Handler file `crates/ui/web-api/src/routes/settings_auth.rs`
- Types `RegistrationSettingsResponse`, `UpdateRegistrationSettingsRequest` from `web-api-types/src/settings.rs`
- Types `AuthenticationSettingsResponse`, `UpdateAuthenticationSettingsRequest` from `web-api-types/src/settings_auth.rs`
- Corresponding OpenAPI schema registrations in `router.rs`

### 4.5 OpenAPI client (`openapi-client` crate)

- Add `AccessSettingsResponse` and `UpdateAccessSettingsRequest` to `openapi-client/src/types/`
- Remove `RegistrationSettingsResponse`, `UpdateRegistrationSettingsRequest`, `AuthenticationSettingsResponse`, `UpdateAuthenticationSettingsRequest`
- Add `get_access_settings()` and `update_access_settings()` to client
- Remove `get_registration_settings()`, `update_registration_settings()`, `get_authentication_settings()`, `update_authentication_settings()`

---

## 5. CLI Changes

### Merged `settings access` subcommand

New file: `crates/ui/cli/src/commands/settings/access.rs`

```text
settings access show
settings access update [--mode open|invite|closed]
                       [--token <string>]
                       [--require-token-for-oidc <bool>]
                       [--password-auth-enabled <bool>]
                       [--two-factor-required <bool>]
```

`show` calls `GET /api/v1/settings/access`. `update` calls `PUT /api/v1/settings/access`.

`HumanOutput` for `AccessSettingsResponse` emits all four fields.

### Deleted CLI items

- `crates/ui/cli/src/commands/settings/registration.rs`
- `crates/ui/cli/src/commands/settings/authentication.rs`
- `RegistrationCommands` and `AuthenticationCommands` enum variants from `settings/mod.rs`

---

## 6. Frontend API layer

In `frontend/src/lib/api/`:

- Add `getAccessSettings()` → `GET /api/v1/settings/access` → `{ data: AccessSettingsData; etag: string | null }` (same pattern as
  `getOAuthSettings()`)
- Add `updateAccessSettings(req, etag)` → `PUT /api/v1/settings/access` → `AccessSettingsData`
- Remove `updateRegistrationSettings()` and `updateAuthenticationSettings()`
- Update `getCombinedSettings()` return type: remove `registration` and `authentication` fields. Do not add `access` — the combined response no longer
  carries access data (see §4.3)

Frontend type `AccessSettingsData` in `src/lib/types.ts`:

```typescript
export interface AccessSettingsData {
  mode: "open" | "invite" | "closed";
  require_token_for_oidc: boolean;
  password_auth_enabled: boolean;
  two_factor_required: boolean;
}
```

---

## 7. Test changes

- Add `RadioCardGroup` unit tests (selection, keyboard navigation, aria attributes) in `RadioCardGroup.test.ts`
- Add `AccessSettings` component tests replacing `RegistrationSettings.test.ts` and `AuthenticationSettings.test.ts`
- Add integration tests for `GET /PUT /api/v1/settings/access` in `web-api/src/integration_tests/settings_access.rs`; mirror coverage from deleted
  `settings.rs` and `settings_auth.rs` integration tests
- Update `settings_panels.test.ts`: remove assertions that reference the OAuth Clients button block; preserve remaining panel tests
- Update `settings_combined.rs` round-trip test: remove assertions on `combined.registration.mode` and `combined.authentication.*` — these fields are
  gone from `CombinedSettingsResponse` (§4.3). Assert the response contains only `agent_certificates`, `enrollment_tokens`, and
  `multi_tenancy_enabled`
- Update `surface-tabs.test.ts`: the mock for `getCombinedSettings` currently returns `{ registration: ..., authentication: ... }` — update to
  `{ agent_certificates: ..., enrollment_tokens: ..., multi_tenancy_enabled: ... }`. Do not add an `access` field; the combined response no longer
  carries access data
- Run `npm run test:e2e` (macOS + Chromium) before merging: changes to `FormFieldRow` (`border-l-2` dirty highlight, `w-fit` label) affect rendered
  layout and may produce snapshot diffs that need updating

---

## 8. Documentation deliverables

| Document                            | Change                                                                                                                                |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| `docs/development/ui/README.md`     | Add `RadioCardGroup` to the form primitives list                                                                                      |
| `docs/development/ui/primitives.md` | Add `RadioCardGroup` entry (props, usage example, accessibility note); add `createFormDraft` entry (location, interface, when to use) |
| `CONTEXT.md`                        | No glossary changes needed                                                                                                            |

No ADR required — this is a UI/UX improvement and endpoint consolidation, not a reversible architectural decision.

---

## 9. Out of scope / deferred

- Dirty tracking for forms on other tabs (Global Settings, Plugin Configs, Scheduler, Notification Rules) — `createFormDraft` makes each a one-day
  effort
- `RadioCardGroup` usage beyond Registration Mode
- Any visual regression snapshot updates (follow existing waivers process)
