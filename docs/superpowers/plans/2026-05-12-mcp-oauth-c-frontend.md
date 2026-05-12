# MCP OAuth — Plan C: Frontend Surfaces (Phase 2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship three SvelteKit routes — the consent screen, the end-user "Authorized Apps" view, and the Operator OAuth
Clients management page — using only the existing UI primitives and semantic Tailwind tokens. No new design tokens, no
new primitive components. All mutations submit via `fetch()` with `Authorization: Bearer <dashboard_jwt>` (this codebase
is a pure SPA on `@sveltejs/adapter-static`; SvelteKit form actions are not in scope).

**Architecture:** Three new routes under `frontend/src/routes/oauth/consent/[request_id]/`,
`frontend/src/routes/settings/account/authorized-apps/`, and
`frontend/src/routes/settings/authentication/oauth-clients/`. New API-client functions in
`frontend/src/lib/api/oauth.ts`. A `DisplayString` newtype-like helper in `frontend/src/lib/oauth/htmlEscape.ts`
enforces HTML escape-by-construction for attacker-controlled fields. Typed-confirmation input for unverified clients
matches the redirect URI hostname (not client_name) per the spec's contrarian-hardening.

**Tech Stack:** SvelteKit static adapter + TypeScript strict + Tailwind semantic tokens + Lucide named static imports +
existing primitives (`PageShell`, `SectionCard`, `Button`, `Callout`, `StatusBadge`, `DataTable`, `EmptyState`).

**Spec:** `docs/superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md` (commit `b7ee4a852`).

**Status:** Draft → Ready for review.

---

## Prerequisites

- **Plan A** (foundation) merged.
- **Plan B** (AS routes) merged, providing the `/api/oauth/clients`, `/api/oauth/consents`, and `/oauth/consent/{id}`
  API surfaces this plan consumes.

## Snapshot binding

- "no hardcoded hex/rgb colors in component or route files where semantic token exists" — every color reference uses
  `--bg-surface`, `--text-primary`, `--accent`, etc.
- "no arbitrary pixel values" — heights/typography use the named token classes
- "no Tailwind palette utilities" — no `text-zinc-500` / `bg-slate-900`
- "do not use preset-filled-_, preset-tonal-_, text-surface-_, bg-surface-_, border-surface-\* Skeleton utilities" — use
  `<Callout>` / `<StatusBadge>` instead
- "always use `<Button>` primitive; never `<a class='btn'>` Skeleton patterns"
- "always use `<StatusBadge>` for status labels"
- "always use `<Callout>` for inline notices"
- "PageShell: eyebrow, h1 title at 20px bold, description, actions slot, body content"
- "SectionCard: optional h2 title, description, actions, body; separator line below header"
- "DataTable: header bg-[var(--bg-raised)] text-table-header uppercase; rows bordered; even rows bg-raised; hover
  bg-hover"
- "import lucide icons as named static imports only" — `import { ShieldAlert, KeyRound } from 'lucide-svelte';`
- "Input height: h-8 (32px); Textarea minimum: 4rem, resizable vertically"
- "focus ring: outline none + box-shadow 0 0 0 3px rgba(var(--accent-rgb),.25); show on :focus-visible only"
- "TypeScript strict: true; forceConsistentCasingInFileNames: true"
- "Prettier: useTabs: true, singleQuote: true, trailingComma: none, printWidth: 120"
- "ESLint: @typescript-eslint/no-unused-vars error with `argsIgnorePattern: ^_` and `varsIgnorePattern: ^_`"
- "ESLint: svelte/no-navigation-without-resolve off (SPA uses static adapter)"

## File Structure

**New files:**

- `frontend/src/routes/oauth/consent/[request_id]/+page.svelte` — consent screen.
- `frontend/src/routes/settings/account/authorized-apps/+page.svelte` — end-user view.
- `frontend/src/routes/settings/authentication/oauth-clients/+page.svelte` — Operator view.
- `frontend/src/routes/settings/authentication/oauth-clients/RegisterClientDialog.svelte` — manual register form.
- `frontend/src/lib/api/oauth.ts` — typed API client helpers.
- `frontend/src/lib/oauth/htmlEscape.ts` — `DisplayString` HTML escape helper.

**Modified files:**

- `frontend/src/routes/login/+page.svelte` — pass through `_auth_context=oauth` query param to telemetry / analytics
  calls if present (one-line change).

---

## Tasks

### Task 1: DisplayString HTML escape helper

**Files:**

- Create: `frontend/src/lib/oauth/htmlEscape.ts`
- Create: `frontend/src/lib/oauth/htmlEscape.spec.ts`

- [ ] **Step 1: Write tests**

```typescript
import { describe, expect, it } from "vitest";
import { htmlEscape } from "./htmlEscape";

describe("htmlEscape", () => {
  it("escapes script tags", () => {
    expect(htmlEscape("<script>alert(1)</script>")).toBe(
      "&lt;script&gt;alert(1)&lt;/script&gt;",
    );
  });

  it("escapes ampersands first", () => {
    expect(htmlEscape("a & b")).toBe("a &amp; b");
  });

  it("returns plain text unchanged", () => {
    expect(htmlEscape("Cursor IDE")).toBe("Cursor IDE");
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend && npm run test -- htmlEscape` Expected: FAIL (module not found).

- [ ] **Step 3: Implement**

```typescript
// frontend/src/lib/oauth/htmlEscape.ts
/**
 * HTML-escape an attacker-controlled string. The OAuth consent screen renders
 * client_name and client_uri values that are supplied by DCR registrants or
 * fetched from CIMD documents — both are attacker-controlled. Every binding
 * site must funnel these strings through `htmlEscape()` to prevent HTML
 * injection in the consent prompt.
 */
export function htmlEscape(input: string): string {
  return input
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}
```

- [ ] **Step 4: Run tests to verify pass**

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(frontend): DisplayString HTML escape helper for OAuth consent

Per spec §11.0. All attacker-controlled fields (client_name, client_uri)
must funnel through htmlEscape() at the binding site."
```

### Task 2: OAuth API client helpers

**Files:**

- Create: `frontend/src/lib/api/oauth.ts`

- [ ] **Step 1: Write API helpers**

```typescript
import { apiFetch } from "$lib/api/client"; // existing helper that adds Bearer header

export interface ConsentDetails {
  client_id: string;
  client_name: string;
  client_uri: string | null;
  redirect_uri: string;
  redirect_uri_host: string;
  scopes: string[];
  created_via: "dcr" | "cimd_cache" | "manual";
  trusted_at: string | null;
  requires_typed_confirmation: boolean;
  typed_confirmation_value: string;
  metadata_change_diff: MetadataDiff | null;
}

export interface MetadataDiff {
  redirect_uris?: { from: string[]; to: string[] };
  client_name?: { from: string; to: string };
  client_uri?: { from: string | null; to: string | null };
}

export async function getConsentDetails(
  requestId: string,
): Promise<ConsentDetails> {
  return apiFetch(`/oauth/consent/${encodeURIComponent(requestId)}`);
}

export async function approveConsent(
  requestId: string,
  typedConfirmation: string | null,
): Promise<{ redirect_to: string }> {
  return apiFetch(`/oauth/consent/${encodeURIComponent(requestId)}/approve`, {
    method: "POST",
    body: JSON.stringify({ typed_confirmation: typedConfirmation }),
  });
}

export async function denyConsent(
  requestId: string,
): Promise<{ redirect_to: string }> {
  return apiFetch(`/oauth/consent/${encodeURIComponent(requestId)}/deny`, {
    method: "POST",
  });
}

export interface OAuthClient {
  id: string;
  client_name: string;
  client_uri: string | null;
  created_via: "dcr" | "cimd_cache" | "manual";
  created_at: string;
  last_used_at: string | null;
  revoked_at: string | null;
  trusted_at: string | null;
  redirect_uris: string[];
}

export async function listOAuthClients(): Promise<OAuthClient[]> {
  return apiFetch("/api/oauth/clients");
}

export async function revokeOAuthClient(clientId: string): Promise<void> {
  await apiFetch(`/api/oauth/clients/${encodeURIComponent(clientId)}`, {
    method: "DELETE",
  });
}

export async function trustOAuthClient(clientId: string): Promise<void> {
  await apiFetch(`/api/oauth/clients/${encodeURIComponent(clientId)}/trust`, {
    method: "POST",
  });
}

export async function manualRegisterClient(
  body: Omit<
    OAuthClient,
    | "id"
    | "created_at"
    | "last_used_at"
    | "revoked_at"
    | "trusted_at"
    | "created_via"
  > & {
    default_scope: string;
  },
): Promise<OAuthClient> {
  return apiFetch("/api/oauth/clients", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export interface OAuthConsent {
  id: string;
  client_id: string;
  client_name: string;
  scopes: string[];
  granted_at: string;
  last_used_at: string | null;
}

export async function listMyConsents(): Promise<OAuthConsent[]> {
  return apiFetch("/api/oauth/consents");
}

export async function revokeMyConsent(consentId: string): Promise<void> {
  await apiFetch(`/api/oauth/consents/${encodeURIComponent(consentId)}`, {
    method: "DELETE",
  });
}
```

- [ ] **Step 2: Type-check**

Run: `cd frontend && npm run check`

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(frontend): OAuth API client helpers

Per spec §5.1 + §11.5 + §12.4 + §12.5. Typed wrappers around apiFetch."
```

### Task 3: Consent route — load + render details

**Files:**

- Create: `frontend/src/routes/oauth/consent/[request_id]/+page.svelte`

- [ ] **Step 1: Write the page using existing primitives**

```svelte
<script lang="ts">
    import { onMount } from 'svelte';
    import { page } from '$app/stores';
    import { goto } from '$app/navigation';
    import { ShieldAlert, BadgeCheck, ExternalLink, AlertTriangle } from 'lucide-svelte';
    import PageShell from '$lib/components/PageShell.svelte';
    import SectionCard from '$lib/components/SectionCard.svelte';
    import Button from '$lib/components/Button.svelte';
    import Callout from '$lib/components/Callout.svelte';
    import StatusBadge from '$lib/components/StatusBadge.svelte';
    import { getConsentDetails, approveConsent, denyConsent, type ConsentDetails } from '$lib/api/oauth';
    import { htmlEscape } from '$lib/oauth/htmlEscape';

    let details = $state<ConsentDetails | null>(null);
    let loadError = $state<string | null>(null);
    let typedConfirmation = $state('');
    let submitting = $state(false);

    const requestId = $derived($page.params.request_id);

    onMount(async () => {
        try {
            details = await getConsentDetails(requestId);
        } catch (e) {
            loadError = e instanceof Error ? e.message : String(e);
        }
    });

    const allowDisabled = $derived(
        submitting ||
        !details ||
        (details.requires_typed_confirmation &&
         typedConfirmation.trim().toLowerCase() !== details.typed_confirmation_value.toLowerCase())
    );

    async function handleAllow() {
        if (!details) return;
        submitting = true;
        try {
            const resp = await approveConsent(
                requestId,
                details.requires_typed_confirmation ? typedConfirmation : null
            );
            window.location.href = resp.redirect_to;
        } catch (e) {
            submitting = false;
            loadError = e instanceof Error ? e.message : String(e);
        }
    }

    async function handleDeny() {
        if (!details) return;
        submitting = true;
        try {
            const resp = await denyConsent(requestId);
            window.location.href = resp.redirect_to;
        } catch (e) {
            submitting = false;
            loadError = e instanceof Error ? e.message : String(e);
        }
    }

    type BadgeTone = 'danger' | 'warning' | 'neutral';
    interface Badge { tone: BadgeTone; text: string }

    function clientBadge(d: ConsentDetails): Badge {
        if (d.trusted_at === null) {
            return { tone: 'danger', text: 'Unverified client' };
        }
        switch (d.created_via) {
            case 'dcr':
                return { tone: 'warning', text: 'Newly registered (DCR)' };
            case 'cimd_cache':
                return { tone: 'neutral', text: 'Open client metadata' };
            case 'manual':
                return { tone: 'neutral', text: 'Manually registered' };
            default: {
                // Exhaustive guard — server schema is a known union; the wildcard arm satisfies
                // TypeScript strict's no-implicit-return and gives an audit-trail-friendly fallback.
                const _exhaustive: never = d.created_via;
                return { tone: 'neutral', text: 'Unknown client source' };
            }
        }
    }
</script>

<PageShell eyebrow="Authorize Access" title={details ? `${htmlEscape(details.client_name)} wants access to your uptrakit account` : 'Loading…'}>
    {#if loadError}
        <Callout tone="danger">{loadError}</Callout>
    {:else if details}
        <SectionCard title="Client">
            <div class="flex items-center gap-3">
                <BadgeCheck size={20} />
                <div>
                    <div class="text-page-title">{@html htmlEscape(details.client_name)}</div>
                    {#if details.client_uri}
                        <a
                            href={details.client_uri}
                            target="_blank"
                            rel="noopener noreferrer"
                            class="text-secondary inline-flex items-center gap-1"
                        >
                            {@html htmlEscape(details.client_uri)}
                            <ExternalLink size={14} />
                        </a>
                    {/if}
                </div>
                <StatusBadge tone={clientBadge(details).tone}>{clientBadge(details).text}</StatusBadge>
            </div>
        </SectionCard>

        <SectionCard title="Redirect URI">
            <div class="font-mono">{details.redirect_uri_host}</div>
            {#if ['localhost', '127.0.0.1', '[::1]'].includes(details.redirect_uri_host)}
                <Callout tone="warning">
                    <AlertTriangle size={16} />
                    This client is asking to receive credentials at a local-only address. Make sure
                    you are running it on this machine right now.
                </Callout>
            {/if}
        </SectionCard>

        {#if details.metadata_change_diff}
            <SectionCard title="Metadata change notice">
                <Callout tone="warning">
                    This client's published metadata has changed since you last authorized it.
                    Review the new details before continuing.
                </Callout>
                <!-- render diff -->
            </SectionCard>
        {/if}

        <SectionCard title="Permissions requested">
            <ul class="space-y-2">
                {#each details.scopes as scope}
                    <li>
                        <StatusBadge tone="neutral">{scope}</StatusBadge>
                        <span class="text-secondary">{scopeDescription(scope)}</span>
                    </li>
                {/each}
            </ul>
            <Callout tone="info">
                {htmlEscape(details.client_name)} will act using your existing permissions —
                it cannot do anything you cannot already do.
            </Callout>
        </SectionCard>

        {#if details.requires_typed_confirmation}
            <SectionCard title="Confirm redirect URI">
                <p class="text-secondary mb-2">
                    Type the redirect hostname below to confirm you have verified it matches what
                    you expect.
                </p>
                <label class="block">
                    <span class="text-secondary text-sm">Expected: {details.typed_confirmation_value}</span>
                    <input
                        type="text"
                        bind:value={typedConfirmation}
                        class="h-8 w-full mt-1"
                        autocomplete="off"
                    />
                </label>
            </SectionCard>
        {/if}

        <div class="flex gap-2 justify-end mt-4">
            <Button variant="secondary" onclick={handleDeny} disabled={submitting}>Deny</Button>
            <Button variant="primary" onclick={handleAllow} disabled={allowDisabled}>Allow access</Button>
        </div>
    {:else}
        <Callout tone="neutral">Loading…</Callout>
    {/if}
</PageShell>

<script lang="ts" module>
    function scopeDescription(scope: string): string {
        if (scope === 'mcp:read') return 'Read your uptrakit data (update history, host info, account profile)';
        if (scope === 'mcp:write') return 'Trigger software updates on your behalf';
        return scope;
    }
</script>
```

- [ ] **Step 2: Type-check + lint**

Run: `cd frontend && npm run check && npm run lint && npm run format:check`

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(frontend): OAuth consent screen route

Per spec §12.4. Unverified-client typed-confirmation against redirect_uri
hostname per contrarian-pass-2 hardening. All UI uses existing primitives
and semantic tokens."
```

### Task 4: E2E test for consent flow + UI parity snapshot or waiver

**Files:**

- Create: `frontend/tests/oauth-consent.spec.ts` (Playwright)
- Modify or create: `frontend/tests/e2e/ui-parity-waivers.json` (if the project uses one — check first)

- [ ] **Step 1: Write E2E test**

Test scenarios: unverified-client typed-confirmation gates the Allow button; allow with correct typed value redirects;
deny redirects with error; localhost redirect shows warning callout.

- [ ] **Step 2: Run**

Run: `cd frontend && npm run test:e2e -- oauth-consent` Note: E2E snapshots regenerate only on macOS+Chromium per
`docs/development/testing.md`.

- [ ] **Step 3: UI parity snapshot decision**

Search the repo for an existing UI parity waivers convention:

```bash
find frontend -type f -name 'ui-parity-waivers.json' -o -name '*.parity.spec.ts' 2>/dev/null
git log --oneline --since="6 months" -- frontend/tests/ | grep -i parity | head
```

If the project DOES use Playwright parity snapshots: generate the consent-screen snapshot on macOS+Chromium and commit
the artefact. If the project does NOT (no waivers file, no parity convention in test files): no waiver needed — skip
this step. Do NOT add a waivers file the project does not already maintain. If the search finds the convention but
generating the snapshot is blocked by another platform, add the waiver entry with `scope`, `owner`, `expiry_date`,
`capture_region`, `justification`, and `review_ref` fields.

- [ ] **Step 4: Commit**

```bash
git commit -m "test(frontend): E2E consent screen flow + parity decision

Per spec §12.4 and docs/development/testing.md E2E conventions."
```

### Task 5: Authorized Apps route

**Files:**

- Create: `frontend/src/routes/settings/account/authorized-apps/+page.svelte`

- [ ] **Step 1: Write the route**

Render a `PageShell` titled "Authorized applications", a `SectionCard` containing a `DataTable` of consents (columns:
Name, Granted, Last used, Scopes, ""), with a Revoke action per row. EmptyState with `ShieldCheck` Lucide icon when no
consents exist. All mutations use `revokeMyConsent`.

Uses the same primitives + semantic tokens as Task 3.

- [ ] **Step 2: Type-check + lint**

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(frontend): Authorized Apps end-user view

Per spec §12.5. DataTable with revoke action; empty state."
```

### Task 6: OAuth Clients Operator view

**Files:**

- Create: `frontend/src/routes/settings/authentication/oauth-clients/+page.svelte`
- Create: `frontend/src/routes/settings/authentication/oauth-clients/RegisterClientDialog.svelte`

- [ ] **Step 1: Write the list page**

`PageShell` titled "OAuth Clients". `SectionCard` titled "Clients with access" containing a `DataTable` (columns: Name,
Source, Status, Created, Last used, Actions). Row actions: View details, Trust (when `trusted_at === null`), Revoke.
EmptyState. Register button opens `RegisterClientDialog`.

Status column uses `StatusBadge`:

- `revoked_at !== null` → tone=`neutral`, text "Revoked"
- `trusted_at === null` → tone=`danger`, text "Unverified"
- `created_via === 'dcr'` → tone=`warning`, text "DCR"
- `created_via === 'cimd_cache'` → tone=`neutral`, text "CIMD"
- `created_via === 'manual'` → tone=`neutral`, text "Manual"

Source column shows the same value as text.

- [ ] **Step 2: Write the dialog**

`RegisterClientDialog.svelte` with form fields: client_name (required), client_uri (optional), redirect_uris (CSV input,
validated as URLs HTTPS-or-localhost), default_scope (select between `mcp:read`, `mcp:write`),
token_endpoint_auth_method (radio: `none` default, `client_secret_basic`).

Submit calls `manualRegisterClient`. On success, returns the created `OAuthClient`.

- [ ] **Step 3: Type-check + lint**

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(frontend): Operator OAuth Clients management page

Per spec §11.5. List + trust + revoke + manual register."
```

### Task 7: Settings sub-section toggles (DCR / CIMD / canonical_host)

**Files:**

- Modify or create: settings page under `frontend/src/routes/settings/authentication/oauth-clients/+page.svelte` (or
  sibling tab) for OAuth-wide toggles

- [ ] **Step 1: Add toggle controls**

UI controls for `oauth.mcp_enabled`, `oauth.dcr_enabled`, `oauth.cimd_enabled`. When `oauth.mcp_enabled` is being
flipped from false → true, show a Callout warning that `oauth.canonical_host` must be set first; pre-flight the setting
before allowing the toggle.

Use existing settings-toggle primitive if present (check `frontend/src/lib/components/`) — otherwise a `<label>` +
checkbox styled via semantic tokens. No new component design.

Gate this section on `ManageGlobalSettings` permission via existing permission-check helper.

**Phase-ordering safety: this Task MUST NOT MERGE until Plan D ships.** If Plan C lands while Plan D is still in flight,
the Operator-visible `oauth.mcp_enabled` toggle would activate the AS routes (Plan B) without the RS layer + PRM
endpoint (Plan D) — flipping it on would mint tokens that nothing accepts. Either land Plan C Task 7 in a separate
follow-up PR after Plan D merges, or temporarily wrap the toggle UI in an
`{#if browser && import.meta.env.VITE_OAUTH_TOGGLE_PREVIEW}` block that defaults off until Plan D's release commit
explicitly enables it. The settings keys themselves are registered in Plan B Task 21 for code-internal consumption (boot
validation reads them) — this Task only adds the UI affordance.

- [ ] **Step 2: Type-check + lint**

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(frontend): OAuth settings toggles

Per spec §11.0. Pre-flights canonical_host before allowing master switch flip."
```

### Task 8: Login page passthrough for \_auth_context=oauth

**Files:**

- Modify: `frontend/src/routes/login/+page.svelte`

- [ ] **Step 1: Pass through analytics flag**

When `?_auth_context=oauth` is present in the URL on login mount, surface it to whatever analytics / telemetry hook the
login page uses today (likely `getAuthMethods()` or similar — check first). One-line change.

- [ ] **Step 2: Commit**

```bash
git commit -m "feat(frontend): login screen recognises _auth_context=oauth

Per spec §13 Model A delegation."
```

### Task 9: Run frontend quality gates

- [ ] **Step 1: Run gates**

```bash
cd frontend
npm run lint
npm run format:check
npm run check
npm run test
npm run test:e2e -- oauth
npm run build
```

Expected: every command exits 0.

- [ ] **Step 2: Fix any lint or test failures inline**

If ESLint or `npm run check` complains, fix the root cause — do not add `eslint-disable` comments. Per snapshot, prefer
fixing the underlying issue idiomatically.

- [ ] **Step 3: Final commit if any**

---

## Self-review checklist

- [ ] **Snapshot conformance**: every color is a CSS variable token; no Tailwind palette utilities; every Skeleton
      `preset-*` replaced with `<Callout>` or `<StatusBadge>`; every icon imported by name (no `<Icon name="...">`
      wrapper); inputs use `h-8`; focus ring uses `--accent-rgb` not `outline`.
- [ ] **Idiomatic pattern check**: SPA-only — no `+page.server.ts` files; mutations submit via `fetch()` with
      `Authorization: Bearer`; no SvelteKit form actions or cookie-tied CSRF tokens; no `<a class="btn">` patterns.
- [ ] **Documentation completeness**: no docs deliverables in this plan (Plan E owns docs).
- [ ] **Task atomicity**: each task is a single coherent change with its own commit.
- [ ] **Phase ordering**: requires Plan A + Plan B merged. Plan C can land independently of Plans D and E.
