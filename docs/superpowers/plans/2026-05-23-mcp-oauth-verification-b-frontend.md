# MCP OAuth Verification — Plan B: Frontend Settings UI

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the OAuth settings card's raw `<input>` elements and live-API-on-change pattern
with a draft state model, design-system primitives, and an explicit Save/Discard button pair.

**Architecture:** Local `draft` state (`$state`) mirrors `UpdateOAuthSettingsRequest` writable
fields only. `isDirty` is a `$derived` field-by-field comparison. `onMount` initialises `draft` from
the loaded settings. Save calls `handleSettingsChange(draft)` once. Discard resets `draft` to a
`structuredClone` of `oauthSettings`. Design primitives: `Checkbox` + `FormFieldRow` + `Input` +
`Button`. `restart_required` Callout reads from `oauthSettings` (persisted state), never from draft.

**Tech Stack:** Svelte 5 runes, Tailwind v4, semantic CSS tokens, `$lib/components/forms`,
`$lib/components/Button.svelte`.

---

## File Map

| Action | Path                                                                     |
| ------ | ------------------------------------------------------------------------ |
| Modify | `frontend/src/routes/settings/authentication/oauth-clients/+page.svelte` |

This is a single-file change. The existing `listOAuthClients` / `handleTrust` / `handleRevoke` /
`loadClients` / `RegisterClientDialog` logic is untouched.

---

## Task 1: Introduce draft state model and `isDirty` derived

**Files:**

- Modify: `frontend/src/routes/settings/authentication/oauth-clients/+page.svelte`

- [ ] **Step 1: Add `DraftOAuthSettings` interface and draft/isDirty state to `<script>`**

In the `<script lang="ts">` block, after the existing `let showRegisterDialog` state declarations
(around line 37), add:

```ts
// OAuth settings draft state — mirrors UpdateOAuthSettingsRequest writable fields only.
// restart_required is server-computed; never in draft.
interface DraftOAuthSettings {
  mcp_enabled: boolean;
  dcr_enabled: boolean;
  cimd_enabled: boolean;
  canonical_host: string | null;
}

let draft = $state<DraftOAuthSettings | null>(null);

const isDirty = $derived(
  draft !== null &&
    oauthSettings !== null &&
    (draft.mcp_enabled !== oauthSettings.mcp_enabled ||
      draft.dcr_enabled !== oauthSettings.dcr_enabled ||
      draft.cimd_enabled !== oauthSettings.cimd_enabled ||
      (draft.canonical_host ?? null) !== oauthSettings.canonical_host),
);
```

- [ ] **Step 2: Update `loadOAuthSettings` to initialise draft**

Replace the existing `loadOAuthSettings` function:

```ts
async function loadOAuthSettings() {
  settingsLoading = true;
  settingsError = null;
  try {
    oauthSettings = await getOAuthSettings();
    // Explicit field extraction — avoids carrying server-computed `restart_required`
    // into the draft where it would be serialised back on Save.
    draft = {
      mcp_enabled: oauthSettings!.mcp_enabled,
      dcr_enabled: oauthSettings!.dcr_enabled,
      cimd_enabled: oauthSettings!.cimd_enabled,
      // Normalise undefined → null: server may omit the field when unset, which
      // would produce undefined here and make isDirty true immediately on load.
      canonical_host: oauthSettings!.canonical_host ?? null,
    };
  } catch (e) {
    settingsError = e instanceof Error ? e.message : "Failed to load OAuth settings";
  } finally {
    settingsLoading = false;
  }
}
```

- [ ] **Step 3: Update `handleSettingsChange` to sync draft on success**

Replace the existing `handleSettingsChange` function:

```ts
async function handleSettingsChange(patch: DraftOAuthSettings) {
  savingSettings = true;
  settingsError = null;
  try {
    // Convert null → undefined: UpdateOAuthSettingsRequest.canonical_host is
    // `string | undefined` (optional field); null is not assignable to that type.
    oauthSettings = await updateOAuthSettings({
      ...patch,
      canonical_host: patch.canonical_host ?? undefined,
    });
    draft = {
      mcp_enabled: oauthSettings!.mcp_enabled,
      dcr_enabled: oauthSettings!.dcr_enabled,
      cimd_enabled: oauthSettings!.cimd_enabled,
      canonical_host: oauthSettings!.canonical_host ?? null,
    };
  } catch (e) {
    settingsError = e instanceof Error ? e.message : "Failed to save OAuth settings";
  } finally {
    savingSettings = false;
  }
}
```

- [ ] **Step 4: Add `handleDiscard` function**

```ts
function handleDiscard() {
  if (oauthSettings !== null) {
    draft = {
      mcp_enabled: oauthSettings!.mcp_enabled,
      dcr_enabled: oauthSettings!.dcr_enabled,
      cimd_enabled: oauthSettings!.cimd_enabled,
      canonical_host: oauthSettings!.canonical_host ?? null,
    };
  }
}
```

- [ ] **Step 5: Add form-primitive imports**

Update the import block at the top of `<script>` to add:

```ts
import { Checkbox, FormFieldRow, Input } from "$lib/components/forms";
```

The `Button` import is already present: `import Button from '$lib/components/Button.svelte';`

- [ ] **Step 6: Verify TypeScript checks**

```bash
cd frontend && npm run check 2>&1 | tail -20
```

Expected: no new type errors. (Markup still uses old raw inputs — that's fine until Task 2.)

---

## Task 2: Replace settings card markup with design primitives + Save/Discard

**Files:**

- Modify: `frontend/src/routes/settings/authentication/oauth-clients/+page.svelte`

- [ ] **Step 1: Replace the entire `{#if canManageGlobalSettings}` block**

Locate the existing settings card (around line 245–318):

```svelte
{#if canManageGlobalSettings}
    <SectionCard title="OAuth settings">
    ...
    </SectionCard>
{/if}
```

Replace it entirely with:

```svelte
{#if canManageGlobalSettings}
    <SectionCard title="OAuth settings">
        {#if settingsLoading}
            <p class="py-4 text-center text-sm text-[var(--text-secondary)]">Loading…</p>
        {:else if draft !== null && oauthSettings !== null}
            <div class="space-y-4">
                <label class="flex items-center gap-3" for="mcp_enabled">
                    <Checkbox
                        id="mcp_enabled"
                        bind:checked={draft.mcp_enabled}
                        disabled={savingSettings}
                    />
                    <span class="text-sm text-[var(--text-primary)]">Enable MCP OAuth (master switch)</span>
                </label>

                {#if draft.mcp_enabled && !draft.canonical_host}
                    <Callout
                        tone="warning"
                        message="oauth.canonical_host must be set before enabling MCP OAuth. Tokens minted without a canonical host will be invalid."
                    />
                {/if}

                <label class="flex items-center gap-3" for="dcr_enabled">
                    <Checkbox
                        id="dcr_enabled"
                        bind:checked={draft.dcr_enabled}
                        disabled={savingSettings || !draft.mcp_enabled}
                    />
                    <span class="text-sm text-[var(--text-primary)]">Enable Dynamic Client Registration (DCR)</span>
                </label>

                <label class="flex items-center gap-3" for="cimd_enabled">
                    <Checkbox
                        id="cimd_enabled"
                        bind:checked={draft.cimd_enabled}
                        disabled={savingSettings || !draft.mcp_enabled}
                    />
                    <span class="text-sm text-[var(--text-primary)]">Enable Client-Initiated Metadata Discovery (CIMD)</span>
                </label>

                <FormFieldRow
                    label="Canonical host"
                    hint="Required when MCP OAuth is enabled (e.g. auth.example.com)"
                    inputId="canonical_host"
                >
                    <Input
                        id="canonical_host"
                        type="text"
                        value={draft.canonical_host ?? ''}
                        placeholder="auth.example.com"
                        disabled={savingSettings}
                        oninput={(e) => {
                            const v = (e.currentTarget as HTMLInputElement).value.trim();
                            draft!.canonical_host = v === '' ? null : v;
                        }}
                    />
                </FormFieldRow>

                {#if oauthSettings.restart_required}
                    <Callout
                        tone="info"
                        message="Settings saved. Changes take effect after the controller is restarted."
                    />
                {/if}

                {#if settingsError}
                    <Callout tone="danger" message={settingsError} />
                {/if}

                <div class="flex gap-2">
                    <Button
                        variant="primary"
                        disabled={!isDirty || savingSettings}
                        loading={savingSettings}
                        onclick={() => {
                            if (draft !== null) void handleSettingsChange(draft);
                        }}
                    >
                        Save
                    </Button>
                    {#if isDirty}
                        <Button variant="ghost" disabled={savingSettings} onclick={handleDiscard}>
                            Discard
                        </Button>
                    {/if}
                </div>
            </div>
        {:else if settingsError}
            <Callout tone="danger" message={settingsError} />
        {/if}
    </SectionCard>
{/if}
```

- [ ] **Step 2: Remove the old `handleSettingsChange` call signature from the old markup**

The old markup called
`handleSettingsChange({ mcp_enabled: ..., dcr_enabled: ..., cimd_enabled: ..., canonical_host: ... })`
directly on each `onchange`. Verify there are no remaining `handleSettingsChange` calls in the
template (only one remains: the Save button's `onclick`).

```bash
grep -n "handleSettingsChange" frontend/src/routes/settings/authentication/oauth-clients/+page.svelte
```

Expected: only one match — inside the Save button `onclick`.

- [ ] **Step 3: TypeScript check**

```bash
cd frontend && npm run check 2>&1 | tail -20
```

Expected: no errors.

- [ ] **Step 4: Lint check**

```bash
cd frontend && npm run lint 2>&1 | tail -10
```

Expected: no errors.

- [ ] **Step 5: Run frontend tests**

```bash
cd frontend && npm run test 2>&1 | tail -20
```

Expected: all tests pass. (No new tests needed — this file has no co-located `.test.ts`.)

- [ ] **Step 6: Start dev server and manually verify the settings card**

```bash
cd frontend && npm run dev &
```

Open `http://localhost:5173/settings/authentication/oauth-clients` and verify:

1. Settings load into the card without a Save button initially disabled (since `isDirty = false`).
2. Changing any checkbox or the canonical host field makes the Save button active.
3. Clicking Discard resets the form to the persisted values.
4. Clicking Save calls the API once and reflects the updated `restart_required` from the response.
5. `dcr_enabled` and `cimd_enabled` checkboxes are disabled when `mcp_enabled` is unchecked.
6. Warning Callout appears when `mcp_enabled` is checked and `canonical_host` is empty.

Kill the dev server after testing.

- [ ] **Step 7: Build check**

```bash
cd frontend && npm run build 2>&1 | tail -10
```

Expected: build succeeds.

- [ ] **Step 8: Commit**

```bash
git add frontend/src/routes/settings/authentication/oauth-clients/+page.svelte
git commit -m "feat(frontend/oauth): add draft state, Save/Discard buttons, design primitives to settings card"
```

---

## Quality Gates

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

All must pass before marking this plan complete.
