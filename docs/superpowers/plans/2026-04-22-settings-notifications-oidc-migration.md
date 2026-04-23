# Settings Notifications + OIDC Button Migration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate every interactive button in three notification/OIDC settings sub-components to render through
the `<Button>` primitive.

**Architecture:** Template-level attribute migration only — no runtime behavior changes. New loading flags
introduced for OidcProvidersSettings (`saving`, `togglingProviderId`) and NotificationLogView (`isRetrying`).
Each file is a standalone commit.

**Tech Stack:** Svelte 5, TypeScript, `<Button>` from `$lib/components/Button.svelte`, Vitest + @testing-library/svelte

---

## File Map

| File | Change |
| --- | --- |
| `frontend/src/routes/settings/NotificationRulesSettings.svelte` | Add Button import; migrate Add Rule, per-row Edit/Delete, pagination Previous/Next, modal submit |
| `frontend/src/routes/settings/NotificationLogView.svelte` | Add Button import; introduce `isRetrying` state; migrate single Retry button inside `{#snippet errorActions()}` |
| `frontend/src/routes/settings/OidcProvidersSettings.svelte` | Add Button import; introduce `saving` + `togglingProviderId` state; wrap `saveOidcProvider`/`toggleOidcActive` in try/finally; migrate Add Provider, Edit, Activate/Deactivate, Delete, Cancel, modal submit |
| `frontend/src/routes/settings/NotificationRulesSettings.test.ts` | Create — unit tests for all migrated buttons |
| `frontend/src/routes/settings/NotificationLogView.test.ts` | Create — unit tests for Retry button + isRetrying |
| `frontend/src/routes/settings/OidcProvidersSettings.test.ts` | Create — unit tests for all migrated buttons including per-row toggle isolation |

---

## Task 1: Migrate `NotificationRulesSettings.svelte`

**Files:**

- Modify: `frontend/src/routes/settings/NotificationRulesSettings.svelte`
- Create: `frontend/src/routes/settings/NotificationRulesSettings.test.ts`

### Context

Six raw `<button>` sites in this file:

| Location | Current classes | Target |
| --- | --- | --- |
| Line 156 — Add Rule | `btn preset-filled-primary-500 btn-sm` | `variant="primary" size="sm"` |
| Line 191 — per-row Edit | `btn btn-sm preset-tonal` | `variant="secondary" size="sm"` |
| Lines 192–196 — per-row Delete | `btn btn-sm preset-filled-error-500` | `variant="danger" size="sm"` |
| Lines 207–214 — pagination Previous | `btn btn-sm preset-tonal` + `disabled` | `variant="secondary" size="sm"` + passthrough `disabled` |
| Lines 219–225 — pagination Next | `btn btn-sm preset-tonal` + `disabled` | `variant="secondary" size="sm"` + passthrough `disabled` |
| Line 290 — modal submit | `btn preset-filled-primary-500` + `disabled={saving}` + text swap | `variant="primary" loading={saving}` + static children `{editingRule ? 'Update' : 'Create'}` |

The `saving` flag already exists in the script (line 42). The modal submit is inside a `<form onsubmit>` — keep
`type="submit"` or use `onclick={() => void saveRule()}` consistent with the existing `<button type="submit">` pattern.
Since the current button has no explicit `type="submit"` and the form uses `onsubmit`, replace with
`<Button variant="primary" type="submit" loading={saving}>` to preserve form-submit behaviour.

- [ ] **Step 1.1: Write failing test**

Create `frontend/src/routes/settings/NotificationRulesSettings.test.ts`:

```ts
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
  listNotificationRules: vi.fn(),
  listNotificationChannels: vi.fn(),
  createNotificationRule: vi.fn(),
  updateNotificationRule: vi.fn(),
  deleteNotificationRule: vi.fn()
}));
vi.mock('$lib/auth.svelte', () => ({ getUser: vi.fn(() => null) }));
vi.mock('$lib/notifications.svelte', () => ({
  showSuccess: vi.fn(),
  showError: vi.fn()
}));

import * as api from '$lib/api';
import NotificationRulesSettings from './NotificationRulesSettings.svelte';

const defaultProps = {
  onSuccess: vi.fn(),
  onError: vi.fn()
};

function stubApis() {
  vi.mocked(api.listNotificationRules).mockResolvedValue({
    items: [],
    total: 0,
    page: 1,
    total_pages: 1
  });
  vi.mocked(api.listNotificationChannels).mockResolvedValue({
    items: [{ id: 'ch1', name: 'Channel A', channel_type: 'webhook' }],
    total: 1,
    page: 1,
    total_pages: 1
  });
}

afterEach(() => vi.clearAllMocks());

describe('NotificationRulesSettings — button variants', () => {
  it('Add Rule button has no raw preset-filled-primary-500 class', async () => {
    stubApis();
    const { container } = render(NotificationRulesSettings, defaultProps);
    await waitFor(() => expect(api.listNotificationRules).toHaveBeenCalled());
    expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
  });

  it('Add Rule button has primary variant class (accent gradient)', async () => {
    stubApis();
    const { container } = render(NotificationRulesSettings, defaultProps);
    await waitFor(() => expect(api.listNotificationRules).toHaveBeenCalled());
    const btn = screen.getByRole('button', { name: 'Add Rule' });
    expect(btn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
  });

  it('Add Rule button has sm size class (h-[19px])', async () => {
    stubApis();
    const { container } = render(NotificationRulesSettings, defaultProps);
    await waitFor(() => expect(api.listNotificationRules).toHaveBeenCalled());
    const btn = screen.getByRole('button', { name: 'Add Rule' });
    expect(btn.className).toContain('h-[19px]');
  });

  it('per-row Edit button has secondary variant class and sm size', async () => {
    vi.mocked(api.listNotificationRules).mockResolvedValue({
      items: [
        {
          id: 'r1',
          channel_id: 'ch1',
          event_type: 'update_available',
          host_id: null,
          software_item_id: null,
          plugin_type: null,
          enabled: true,
          created_at: '2026-01-01T00:00:00Z'
        }
      ],
      total: 1,
      page: 1,
      total_pages: 1
    });
    vi.mocked(api.listNotificationChannels).mockResolvedValue({
      items: [{ id: 'ch1', name: 'Channel A', channel_type: 'webhook' }],
      total: 1,
      page: 1,
      total_pages: 1
    });
    render(NotificationRulesSettings, defaultProps);
    const btn = await screen.findByRole('button', { name: 'Edit' });
    expect(btn.className).toContain('bg-[var(--bg-raised)]');
    expect(btn.className).toContain('h-[19px]');
  });

  it('per-row Delete button has danger variant class and sm size', async () => {
    vi.mocked(api.listNotificationRules).mockResolvedValue({
      items: [
        {
          id: 'r1',
          channel_id: 'ch1',
          event_type: 'update_available',
          host_id: null,
          software_item_id: null,
          plugin_type: null,
          enabled: true,
          created_at: '2026-01-01T00:00:00Z'
        }
      ],
      total: 1,
      page: 1,
      total_pages: 1
    });
    vi.mocked(api.listNotificationChannels).mockResolvedValue({
      items: [{ id: 'ch1', name: 'Channel A', channel_type: 'webhook' }],
      total: 1,
      page: 1,
      total_pages: 1
    });
    render(NotificationRulesSettings, defaultProps);
    const btn = await screen.findByRole('button', { name: 'Delete' });
    expect(btn.className).toContain('bg-[var(--color-error-bg)]');
    expect(btn.className).toContain('h-[19px]');
  });

  it('pagination Previous/Next buttons have secondary variant and sm size', async () => {
    vi.mocked(api.listNotificationRules).mockResolvedValue({
      items: [
        {
          id: 'r1',
          channel_id: 'ch1',
          event_type: 'update_available',
          host_id: null,
          software_item_id: null,
          plugin_type: null,
          enabled: true,
          created_at: '2026-01-01T00:00:00Z'
        }
      ],
      total: 1,
      page: 1,
      total_pages: 3
    });
    vi.mocked(api.listNotificationChannels).mockResolvedValue({
      items: [{ id: 'ch1', name: 'Channel A', channel_type: 'webhook' }],
      total: 1,
      page: 1,
      total_pages: 1
    });
    render(NotificationRulesSettings, defaultProps);
    const prev = await screen.findByRole('button', { name: 'Previous' });
    const next = screen.getByRole('button', { name: 'Next' });
    expect(prev.className).toContain('bg-[var(--bg-raised)]');
    expect(prev.className).toContain('h-[19px]');
    expect(next.className).toContain('bg-[var(--bg-raised)]');
    expect(next.className).toContain('h-[19px]');
  });

  it('modal submit carries aria-busy=true while save is in flight', async () => {
    stubApis();
    let resolve!: () => void;
    vi.mocked(api.createNotificationRule).mockReturnValue(
      new Promise((r) => {
        resolve = () => r({} as never);
      })
    );
    render(NotificationRulesSettings, defaultProps);
    await waitFor(() => expect(api.listNotificationRules).toHaveBeenCalled());

    await fireEvent.click(screen.getByRole('button', { name: 'Add Rule' }));
    const submitBtn = await screen.findByRole('button', { name: 'Create' });
    await fireEvent.click(submitBtn);

    await waitFor(() => expect(submitBtn).toHaveAttribute('aria-busy', 'true'));
    expect(submitBtn).toHaveTextContent('Create');

    resolve();
    await waitFor(() => expect(submitBtn).not.toHaveAttribute('aria-busy'));
  });

  it('modal submit text is Update when editing a rule', async () => {
    vi.mocked(api.listNotificationRules).mockResolvedValue({
      items: [
        {
          id: 'r1',
          channel_id: 'ch1',
          event_type: 'update_available',
          host_id: null,
          software_item_id: null,
          plugin_type: null,
          enabled: true,
          created_at: '2026-01-01T00:00:00Z'
        }
      ],
      total: 1,
      page: 1,
      total_pages: 1
    });
    vi.mocked(api.listNotificationChannels).mockResolvedValue({
      items: [{ id: 'ch1', name: 'Channel A', channel_type: 'webhook' }],
      total: 1,
      page: 1,
      total_pages: 1
    });
    render(NotificationRulesSettings, defaultProps);
    const editBtn = await screen.findByRole('button', { name: 'Edit' });
    await fireEvent.click(editBtn);
    await screen.findByRole('button', { name: 'Update' });
  });
});
```

- [ ] **Step 1.2: Run test — expect FAIL**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/NotificationRulesSettings.test.ts 2>&1 | tail -20
```

Expected: tests fail because raw preset classes still exist and aria-busy is never set.

- [ ] **Step 1.3: Add Button import to `NotificationRulesSettings.svelte`**

After the existing imports (after `import { SectionCard, StatusBadge } from '$lib/components/ui';`), add:

```svelte
import Button from '$lib/components/Button.svelte';
```

- [ ] **Step 1.4: Replace Add Rule button (line 156)**

Old:

```svelte
<button class="btn preset-filled-primary-500 btn-sm" onclick={openCreate}>Add Rule</button>
```

New:

```svelte
<Button variant="primary" size="sm" onclick={openCreate}>Add Rule</Button>
```

- [ ] **Step 1.5: Replace per-row Edit button (line 191)**

Old:

```svelte
<button class="btn btn-sm preset-tonal" onclick={() => openEdit(rule)}>Edit</button>
```

New:

```svelte
<Button variant="secondary" size="sm" onclick={() => openEdit(rule)}>Edit</Button>
```

- [ ] **Step 1.6: Replace per-row Delete button (lines 192–196)**

Old:

```svelte
<button
    class="btn btn-sm preset-filled-error-500"
    onclick={() => (deleteConfirm = { id: rule.id, eventType: rule.event_type })}
>
    Delete
</button>
```

New:

```svelte
<Button
    variant="danger"
    size="sm"
    onclick={() => (deleteConfirm = { id: rule.id, eventType: rule.event_type })}
>Delete</Button>
```

- [ ] **Step 1.7: Replace pagination Previous button (lines 207–214)**

Old:

```svelte
<button
    class="btn btn-sm preset-tonal"
    disabled={currentPage <= 1}
    onclick={() => {
        currentPage--;
        void loadData();
    }}
>
    Previous
</button>
```

New:

```svelte
<Button
    variant="secondary"
    size="sm"
    disabled={currentPage <= 1}
    onclick={() => {
        currentPage--;
        void loadData();
    }}
>Previous</Button>
```

- [ ] **Step 1.8: Replace pagination Next button (lines 219–225)**

Old:

```svelte
<button
    class="btn btn-sm preset-tonal"
    disabled={currentPage >= totalPages}
    onclick={() => {
        currentPage++;
        void loadData();
    }}
>
    Next
</button>
```

New:

```svelte
<Button
    variant="secondary"
    size="sm"
    disabled={currentPage >= totalPages}
    onclick={() => {
        currentPage++;
        void loadData();
    }}
>Next</Button>
```

- [ ] **Step 1.9: Replace modal submit button (line 290)**

Old:

```svelte
<button type="submit" class="btn preset-filled-primary-500" disabled={saving}>
    {saving ? 'Saving...' : editingRule ? 'Update' : 'Create'}
</button>
```

New:

```svelte
<Button type="submit" variant="primary" loading={saving}>
    {editingRule ? 'Update' : 'Create'}
</Button>
```

Note: `type="submit"` is preserved so the form `onsubmit` handler fires correctly. The `disabled={saving}`
is removed — `loading={saving}` makes the button inert automatically (`Button` sets `disabled={inert}` where
`inert = disabled || loading`). The text-swap expression is replaced with a static conditional that does not
include "Saving...".

- [ ] **Step 1.10: Run type check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 1.11: Run tests — expect PASS**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/NotificationRulesSettings.test.ts 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 1.12: Run full suite to confirm no regressions**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run 2>&1 | tail -5
```

Expected: all tests pass, 0 failures.

- [ ] **Step 1.13: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx prettier --write src/routes/settings/NotificationRulesSettings.svelte src/routes/settings/NotificationRulesSettings.test.ts
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/src/routes/settings/NotificationRulesSettings.svelte frontend/src/routes/settings/NotificationRulesSettings.test.ts
git commit -m "feat(ui): migrate NotificationRulesSettings to Button primitive (#3e step 1)"
```

---

## Task 2: Migrate `NotificationLogView.svelte`

**Files:**

- Modify: `frontend/src/routes/settings/NotificationLogView.svelte`
- Create: `frontend/src/routes/settings/NotificationLogView.test.ts`

### Context

One raw `<button>` site in this file at line 180, inside `{#snippet errorActions()}` passed to `DataTable`:

```svelte
{#snippet errorActions()}
    <button class="btn preset-filled-primary-500 mt-3" onclick={() => void loadData()}>Retry</button>
{/snippet}
```

The `DataTable` renders this snippet when `error` is non-null. The component currently has no loading guard on
`loadData()`. Introduce `let isRetrying = $state(false)` and wrap the retry call in try/finally. The API function
used for loading is `listNotificationLog` (imported at line 2, called at line 43).

- [ ] **Step 2.1: Write failing test**

Create `frontend/src/routes/settings/NotificationLogView.test.ts`:

```ts
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
  listNotificationLog: vi.fn(),
  listNotificationChannels: vi.fn()
}));
vi.mock('$lib/auth.svelte', () => ({ getUser: vi.fn(() => null) }));

import * as api from '$lib/api';
import NotificationLogView from './NotificationLogView.svelte';

afterEach(() => vi.clearAllMocks());

describe('NotificationLogView Retry button', () => {
  it('Retry button has no raw preset-filled-primary-500 class', async () => {
    vi.mocked(api.listNotificationLog).mockRejectedValue(new Error('network error'));
    vi.mocked(api.listNotificationChannels).mockResolvedValue({
      items: [],
      total: 0,
      page: 1,
      total_pages: 1
    });
    const { container } = render(NotificationLogView);
    await screen.findByRole('button', { name: 'Retry' });
    expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
  });

  it('Retry button has primary variant class (accent gradient)', async () => {
    vi.mocked(api.listNotificationLog).mockRejectedValue(new Error('network error'));
    vi.mocked(api.listNotificationChannels).mockResolvedValue({
      items: [],
      total: 0,
      page: 1,
      total_pages: 1
    });
    render(NotificationLogView);
    const btn = await screen.findByRole('button', { name: 'Retry' });
    expect(btn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
  });

  it('Retry button carries aria-busy=true while retry is in flight', async () => {
    vi.mocked(api.listNotificationLog)
      .mockRejectedValueOnce(new Error('network error'))
      .mockReturnValue(new Promise(() => {}));
    vi.mocked(api.listNotificationChannels).mockResolvedValue({
      items: [],
      total: 0,
      page: 1,
      total_pages: 1
    });
    render(NotificationLogView);
    const btn = await screen.findByRole('button', { name: 'Retry' });
    await fireEvent.click(btn);
    await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));
  });

  it('aria-busy clears after retry resolves', async () => {
    let resolve!: () => void;
    vi.mocked(api.listNotificationLog)
      .mockRejectedValueOnce(new Error('network error'))
      .mockReturnValue(
        new Promise((r) => {
          resolve = () =>
            r({ items: [], total: 0, page: 1, total_pages: 1 });
        })
      );
    vi.mocked(api.listNotificationChannels).mockResolvedValue({
      items: [],
      total: 0,
      page: 1,
      total_pages: 1
    });
    render(NotificationLogView);
    const btn = await screen.findByRole('button', { name: 'Retry' });
    await fireEvent.click(btn);
    await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));
    resolve();
    await waitFor(() => expect(btn).not.toHaveAttribute('aria-busy'));
  });
});
```

- [ ] **Step 2.2: Run test — expect FAIL**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/NotificationLogView.test.ts 2>&1 | tail -20
```

Expected: tests fail — raw class found, aria-busy never set (no isRetrying state).

- [ ] **Step 2.3: Add Button import and isRetrying state to `NotificationLogView.svelte`**

Add after the existing imports (after the `$lib/components/ui` import):

```svelte
import Button from '$lib/components/Button.svelte';
```

Add to the state declarations block (after `let totalItems` or near end of state declarations):

```svelte
let isRetrying: boolean = $state(false);
```

- [ ] **Step 2.4: Replace the Retry button inside `{#snippet errorActions()}`**

Old:

```svelte
{#snippet errorActions()}
    <button class="btn preset-filled-primary-500 mt-3" onclick={() => void loadData()}>Retry</button>
{/snippet}
```

New:

```svelte
{#snippet errorActions()}
    <Button
        variant="primary"
        loading={isRetrying}
        onclick={async () => {
            isRetrying = true;
            try {
                await loadData();
            } finally {
                isRetrying = false;
            }
        }}
    >Retry</Button>
{/snippet}
```

Note: `loadData()` already sets `loading = true` and clears `error` internally. `isRetrying` is a separate flag
that only controls the button's loading spinner and inert state during the retry; it does not replace the
component-level `loading` state. The `mt-3` class from the original button is dropped — layout spacing should
be handled by the surrounding snippet context (DataTable).

- [ ] **Step 2.5: Run type check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 2.6: Run tests — expect PASS**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/NotificationLogView.test.ts 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 2.7: Run full suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run 2>&1 | tail -5
```

Expected: all tests pass, 0 failures.

- [ ] **Step 2.8: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx prettier --write src/routes/settings/NotificationLogView.svelte src/routes/settings/NotificationLogView.test.ts
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/src/routes/settings/NotificationLogView.svelte frontend/src/routes/settings/NotificationLogView.test.ts
git commit -m "feat(ui): migrate NotificationLogView to Button primitive (#3e step 2)"
```

---

## Task 3: Migrate `OidcProvidersSettings.svelte`

**Files:**

- Modify: `frontend/src/routes/settings/OidcProvidersSettings.svelte`
- Create: `frontend/src/routes/settings/OidcProvidersSettings.test.ts`

### Context

Six raw `<button>` sites in this file:

| Location | Current classes | Target |
| --- | --- | --- |
| Line 199 — Add Provider | `btn preset-filled-primary-500` | `variant="primary"` |
| Line 229 — per-row Edit | `btn btn-sm preset-tonal` | `variant="secondary" size="sm"` |
| Line 231 — per-row Deactivate | `btn btn-sm preset-tonal-warning` | `variant="secondary" size="sm" loading={togglingProviderId === provider.id}` |
| Line 235 — per-row Activate | `btn btn-sm preset-tonal-success` | `variant="secondary" size="sm" loading={togglingProviderId === provider.id}` |
| Line 239 — per-row Delete | `btn btn-sm preset-tonal-error` | `variant="danger" size="sm"` |
| Line 362 — modal Cancel | `btn preset-tonal-surface` | `variant="secondary"` |
| Line 363 — modal submit | `btn preset-filled-primary-500` + `disabled={!getIsOnline()}` + conditional text | `variant="primary" loading={saving} disabled={!getIsOnline()}` + static children `{editingProvider ? 'Update' : 'Create'}` |

The current `saveOidcProvider()` and `toggleOidcActive()` functions have no loading guards. Two new reactive
state variables must be introduced before the migrations are applied.

New state variables to add to the script block (after the existing `deleteConfirm` declaration at line 46):

```svelte
let saving: boolean = $state(false);
let togglingProviderId: string | null = $state(null);
```

Updated `saveOidcProvider()` — wrap the outer try block in saving guard:

```svelte
async function saveOidcProvider() {
    let roleMapping: Record<string, string>;
    try {
        roleMapping = JSON.parse(oidcForm.role_mapping_json);
    } catch {
        onError('Role mapping must be valid JSON (e.g. {"oidc_value": "local_role"})');
        return;
    }

    saving = true;
    try {
        // existing create/update logic unchanged
        if (editingProvider) {
            // ... existing updateOidcProvider call ...
        } else {
            // ... existing createOidcProvider call ...
        }
        closeOidcModal();
    } catch (e) {
        onError(e instanceof Error ? e.message : 'Failed to save OIDC provider');
    } finally {
        saving = false;
    }
}
```

Updated `toggleOidcActive()` — wrap body in togglingProviderId guard:

```svelte
async function toggleOidcActive(provider: OidcProviderResponse) {
    togglingProviderId = provider.id;
    try {
        let updated: OidcProviderResponse;
        if (provider.is_active) {
            updated = await deactivateOidcProvider(provider.id);
        } else {
            updated = await activateOidcProvider(provider.id);
        }
        oidcProviders = await getOidcProviders();
        onSuccess(updated.is_active ? `${updated.name} activated.` : `${updated.name} deactivated.`);
    } catch (e) {
        onError(e instanceof Error ? e.message : 'Failed to update provider status');
    } finally {
        togglingProviderId = null;
    }
}
```

- [ ] **Step 3.1: Write failing test**

Create `frontend/src/routes/settings/OidcProvidersSettings.test.ts`:

```ts
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
  getOidcProviders: vi.fn(),
  createOidcProvider: vi.fn(),
  updateOidcProvider: vi.fn(),
  deleteOidcProvider: vi.fn(),
  activateOidcProvider: vi.fn(),
  deactivateOidcProvider: vi.fn()
}));
vi.mock('$lib/auth.svelte', () => ({ getUser: vi.fn(() => null) }));
vi.mock('$lib/notifications.svelte', () => ({
  showSuccess: vi.fn(),
  showError: vi.fn()
}));
vi.mock('$lib/stores/network.svelte', () => ({ getIsOnline: vi.fn(() => true) }));

import * as api from '$lib/api';
import OidcProvidersSettings from './OidcProvidersSettings.svelte';

const defaultProps = {
  providers: [],
  multiTenancyEnabled: false,
  onSuccess: vi.fn(),
  onError: vi.fn()
};

function makeProvider(id: string, name: string, isActive: boolean) {
  return {
    id,
    name,
    slug: id,
    logo_url: null,
    issuer_url: 'https://issuer.example.com',
    client_id: 'client_id',
    scopes: 'openid',
    auto_create_users: true,
    allow_private_network_issuers: false,
    role_mapping: {},
    role_claim_path: null,
    is_active: isActive
  };
}

afterEach(() => vi.clearAllMocks());

describe('OidcProvidersSettings — button variants', () => {
  it('Add Provider button has no raw preset-filled-primary-500 class', () => {
    const { container } = render(OidcProvidersSettings, defaultProps);
    expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
  });

  it('Add Provider button has primary variant class (accent gradient)', () => {
    render(OidcProvidersSettings, defaultProps);
    const btn = screen.getByRole('button', { name: 'Add Provider' });
    expect(btn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
  });

  it('per-row Edit button has secondary variant and sm size', () => {
    render(OidcProvidersSettings, {
      ...defaultProps,
      providers: [makeProvider('p1', 'Provider One', false)]
    });
    const btn = screen.getByRole('button', { name: 'Edit' });
    expect(btn.className).toContain('bg-[var(--bg-raised)]');
    expect(btn.className).toContain('h-[19px]');
  });

  it('per-row Activate button has secondary variant and sm size', () => {
    render(OidcProvidersSettings, {
      ...defaultProps,
      providers: [makeProvider('p1', 'Provider One', false)]
    });
    const btn = screen.getByRole('button', { name: 'Activate' });
    expect(btn.className).toContain('bg-[var(--bg-raised)]');
    expect(btn.className).toContain('h-[19px]');
  });

  it('per-row Deactivate button has secondary variant and sm size', () => {
    render(OidcProvidersSettings, {
      ...defaultProps,
      providers: [makeProvider('p1', 'Provider One', true)]
    });
    const btn = screen.getByRole('button', { name: 'Deactivate' });
    expect(btn.className).toContain('bg-[var(--bg-raised)]');
    expect(btn.className).toContain('h-[19px]');
  });

  it('per-row Delete button has danger variant and sm size', () => {
    render(OidcProvidersSettings, {
      ...defaultProps,
      providers: [makeProvider('p1', 'Provider One', false)]
    });
    const btn = screen.getByRole('button', { name: 'Delete' });
    expect(btn.className).toContain('bg-[var(--color-error-bg)]');
    expect(btn.className).toContain('h-[19px]');
  });

  it('only the toggled row has aria-busy=true — other rows remain unaffected', async () => {
    let resolveToggle!: () => void;
    vi.mocked(api.deactivateOidcProvider).mockReturnValue(
      new Promise((r) => {
        resolveToggle = () => r(makeProvider('p1', 'Provider One', false));
      })
    );
    vi.mocked(api.getOidcProviders).mockResolvedValue([
      makeProvider('p1', 'Provider One', true),
      makeProvider('p2', 'Provider Two', true)
    ]);

    render(OidcProvidersSettings, {
      ...defaultProps,
      providers: [makeProvider('p1', 'Provider One', true), makeProvider('p2', 'Provider Two', true)]
    });

    const deactivateBtns = screen.getAllByRole('button', { name: 'Deactivate' });
    expect(deactivateBtns).toHaveLength(2);

    await fireEvent.click(deactivateBtns[0]);
    await waitFor(() => expect(deactivateBtns[0]).toHaveAttribute('aria-busy', 'true'));
    expect(deactivateBtns[1]).not.toHaveAttribute('aria-busy');

    resolveToggle();
    await waitFor(() => expect(deactivateBtns[0]).not.toHaveAttribute('aria-busy'));
  });

  it('modal submit shows Create text and carries aria-busy during save', async () => {
    let resolve!: () => void;
    vi.mocked(api.createOidcProvider).mockReturnValue(
      new Promise((r) => {
        resolve = () => r(makeProvider('p-new', 'New Provider', false));
      })
    );

    render(OidcProvidersSettings, defaultProps);
    await fireEvent.click(screen.getByRole('button', { name: 'Add Provider' }));

    const submitBtn = await screen.findByRole('button', { name: 'Create' });
    expect(submitBtn).toBeDefined();

    await fireEvent.click(submitBtn);
    await waitFor(() => expect(submitBtn).toHaveAttribute('aria-busy', 'true'));

    resolve();
    await waitFor(() => expect(submitBtn).not.toHaveAttribute('aria-busy'));
  });

  it('modal submit shows Update text when editing', async () => {
    render(OidcProvidersSettings, {
      ...defaultProps,
      providers: [makeProvider('p1', 'Provider One', false)]
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Edit' }));
    await screen.findByRole('button', { name: 'Update' });
  });

  it('modal Cancel button has secondary variant', async () => {
    render(OidcProvidersSettings, defaultProps);
    await fireEvent.click(screen.getByRole('button', { name: 'Add Provider' }));
    const cancelBtn = await screen.findByRole('button', { name: 'Cancel' });
    expect(cancelBtn.className).toContain('bg-[var(--bg-raised)]');
  });
});
```

- [ ] **Step 3.2: Run test — expect FAIL**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/OidcProvidersSettings.test.ts 2>&1 | tail -20
```

Expected: tests fail — raw preset classes found, aria-busy never set.

- [ ] **Step 3.3: Add Button import to `OidcProvidersSettings.svelte`**

After `import { FormFieldRow, SectionCard, StatusBadge } from '$lib/components/ui';`, add:

```svelte
import Button from '$lib/components/Button.svelte';
```

- [ ] **Step 3.4: Add `saving` and `togglingProviderId` state declarations**

After the existing `let deleteConfirm` declaration (line 46), add:

```svelte
let saving: boolean = $state(false);
let togglingProviderId: string | null = $state(null);
```

- [ ] **Step 3.5: Wrap `saveOidcProvider` in saving guard**

The current function has an outer JSON parse try/catch followed by an inner try/catch for API calls. Add
`saving = true` after the JSON parse succeeds (before the inner try block) and add a `finally { saving = false; }`
to the inner try/catch:

Old (inner try/catch in `saveOidcProvider`, lines 119–160):

```svelte
		try {
			if (editingProvider) {
				// ...
			} else {
				// ...
			}
			closeOidcModal();
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save OIDC provider');
		}
```

New:

```svelte
		saving = true;
		try {
			if (editingProvider) {
				// ...
			} else {
				// ...
			}
			closeOidcModal();
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save OIDC provider');
		} finally {
			saving = false;
		}
```

- [ ] **Step 3.6: Wrap `toggleOidcActive` in togglingProviderId guard**

Old (lines 180–194):

```svelte
	async function toggleOidcActive(provider: OidcProviderResponse) {
		try {
			let updated: OidcProviderResponse;
			if (provider.is_active) {
				updated = await deactivateOidcProvider(provider.id);
			} else {
				updated = await activateOidcProvider(provider.id);
			}
			// Activation may deactivate others, so reload all
			oidcProviders = await getOidcProviders();
			onSuccess(updated.is_active ? `${updated.name} activated.` : `${updated.name} deactivated.`);
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to update provider status');
		}
	}
```

New:

```svelte
	async function toggleOidcActive(provider: OidcProviderResponse) {
		togglingProviderId = provider.id;
		try {
			let updated: OidcProviderResponse;
			if (provider.is_active) {
				updated = await deactivateOidcProvider(provider.id);
			} else {
				updated = await activateOidcProvider(provider.id);
			}
			// Activation may deactivate others, so reload all
			oidcProviders = await getOidcProviders();
			onSuccess(updated.is_active ? `${updated.name} activated.` : `${updated.name} deactivated.`);
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to update provider status');
		} finally {
			togglingProviderId = null;
		}
	}
```

- [ ] **Step 3.7: Replace Add Provider button (line 199)**

Old:

```svelte
<button class="btn preset-filled-primary-500" onclick={openCreateOidc}> Add Provider </button>
```

New:

```svelte
<Button variant="primary" onclick={openCreateOidc}>Add Provider</Button>
```

- [ ] **Step 3.8: Replace per-row Edit button (line 229)**

Old:

```svelte
<button class="btn btn-sm preset-tonal" onclick={() => openEditOidc(provider)}> Edit </button>
```

New:

```svelte
<Button variant="secondary" size="sm" onclick={() => openEditOidc(provider)}>Edit</Button>
```

- [ ] **Step 3.9: Replace per-row Deactivate button (line 231)**

Old:

```svelte
<button class="btn btn-sm preset-tonal-warning" onclick={() => toggleOidcActive(provider)}>
    Deactivate
</button>
```

New:

```svelte
<Button
    variant="secondary"
    size="sm"
    loading={togglingProviderId === provider.id}
    onclick={() => void toggleOidcActive(provider)}
>Deactivate</Button>
```

- [ ] **Step 3.10: Replace per-row Activate button (line 235)**

Old:

```svelte
<button class="btn btn-sm preset-tonal-success" onclick={() => toggleOidcActive(provider)}>
    Activate
</button>
```

New:

```svelte
<Button
    variant="secondary"
    size="sm"
    loading={togglingProviderId === provider.id}
    onclick={() => void toggleOidcActive(provider)}
>Activate</Button>
```

- [ ] **Step 3.11: Replace per-row Delete button (line 239)**

Old:

```svelte
<button class="btn btn-sm preset-tonal-error" onclick={() => requestDeleteOidc(provider)}>
    Delete
</button>
```

New:

```svelte
<Button variant="danger" size="sm" onclick={() => requestDeleteOidc(provider)}>Delete</Button>
```

- [ ] **Step 3.12: Replace modal Cancel button (line 362)**

Old:

```svelte
<button class="btn preset-tonal-surface" onclick={closeOidcModal}>Cancel</button>
```

New:

```svelte
<Button variant="secondary" onclick={closeOidcModal}>Cancel</Button>
```

- [ ] **Step 3.13: Replace modal submit button (line 363)**

Old:

```svelte
<button class="btn preset-filled-primary-500" onclick={saveOidcProvider} disabled={!getIsOnline()}>
    {editingProvider ? 'Update' : 'Create'}
</button>
```

New:

```svelte
<Button
    variant="primary"
    loading={saving}
    disabled={!getIsOnline()}
    onclick={() => void saveOidcProvider()}
>{editingProvider ? 'Update' : 'Create'}</Button>
```

- [ ] **Step 3.14: Run type check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 3.15: Run tests — expect PASS**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/OidcProvidersSettings.test.ts 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 3.16: Run full suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run 2>&1 | tail -5
```

Expected: all tests pass, 0 failures.

- [ ] **Step 3.17: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx prettier --write src/routes/settings/OidcProvidersSettings.svelte src/routes/settings/OidcProvidersSettings.test.ts
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/src/routes/settings/OidcProvidersSettings.svelte frontend/src/routes/settings/OidcProvidersSettings.test.ts
git commit -m "feat(ui): migrate OidcProvidersSettings to Button primitive (#3e step 3)"
```

---

## Task 4: Extend unit tests

**Files:**

- Modify: any test files from Tasks 1–3 where gaps are identified

- [ ] **Step 4.1: Run full settings test suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/ 2>&1 | tail -20
```

Expected: all tests in the settings directory pass. If any failures exist in the three new test files or
pre-existing test files that were affected by the migrations, fix them before continuing.

- [ ] **Step 4.2: Fix any gaps**

Common gaps to check:

- NotificationRulesSettings: verify pagination buttons are only rendered when `totalPages > 1` — the test
  already passes `total_pages: 3` for that case.
- OidcProvidersSettings: if the `mt-3` class removal in NotificationLogView caused layout assertions in any
  other test, update those assertions.
- Any mock type mismatches surfaced by `svelte-check` in test files — add missing required fields to mock
  response objects.

- [ ] **Step 4.3: Commit if fixes were needed**

```bash
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/src/routes/settings/
git commit -m "test(ui): extend Button migration unit tests for settings #3e"
```

If no fixes were needed, skip the commit.

---

## Task 5: Frontend gate

**Files:**

- Modify: any files with lint/format/type issues discovered during the gate

- [ ] **Step 5.1: Run lint**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npm run lint 2>&1 | tail -10
```

Expected: no errors.

- [ ] **Step 5.2: Run format check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npm run format:check 2>&1 | tail -10
```

Expected: no files need formatting. If files are flagged, run:

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npm run format
```

- [ ] **Step 5.3: Run svelte-check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npm run check 2>&1 | grep -E "error|Error" | head -20
```

Expected: no errors.

- [ ] **Step 5.4: Run full test suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npm run test 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 5.5: Run build**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npm run build 2>&1 | tail -10
```

Expected: build succeeds with no errors.

- [ ] **Step 5.6: Commit if fixes were needed**

```bash
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/
git commit -m "chore(ui): lint+type fixes for #3e Button migration"
```

If the gate passed without changes, skip the commit.

---

## Task 6: Re-baseline Playwright snapshots

**Files:**

- Modify: Playwright snapshot baselines under `frontend/tests/e2e/` (if settings-related snapshots exist)

- [ ] **Step 6.1: Check for existing e2e coverage of the affected routes**

```bash
ls /Users/andreyyantsen/Development/uptrakit/frontend/tests/e2e/
```

Identify any test files covering `/settings` notifications tab or OIDC tab.

- [ ] **Step 6.2: Run Playwright tests for settings with snapshot update**

If coverage exists:

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx playwright test --update-snapshots tests/e2e/settings 2>&1 | tail -20
```

If the settings e2e file has a different name (e.g., covers the full settings page), update accordingly.
Accept delta: button heights (`h-[19px]` vs old `btn-sm` pixel value), uppercase text from Button's
`uppercase tracking-wide` class, variant-specific color tokens replacing Skeleton preset tokens.

- [ ] **Step 6.3: Verify updated snapshots are correct**

Review the updated snapshot images to confirm changes are limited to button appearance and do not include
unexpected layout shifts or missing elements.

- [ ] **Step 6.4: Commit snapshot updates**

```bash
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/tests/e2e/
git commit -m "test(e2e): re-baseline Playwright snapshots for #3e Button migration"
```

If no snapshots exist for the affected routes, skip this task entirely.

---

## Self-Review Checklist

**Spec coverage:**

| Spec requirement | Task |
| --- | --- |
| NotificationRulesSettings: Add Rule → `primary sm` | Task 1, Step 1.4 |
| NotificationRulesSettings: Edit → `secondary sm` | Task 1, Step 1.5 |
| NotificationRulesSettings: Delete → `danger sm` | Task 1, Step 1.6 |
| NotificationRulesSettings: Previous → `secondary sm` + disabled passthrough | Task 1, Step 1.7 |
| NotificationRulesSettings: Next → `secondary sm` + disabled passthrough | Task 1, Step 1.8 |
| NotificationRulesSettings: modal submit → `primary loading={saving}` + static text | Task 1, Step 1.9 |
| NotificationLogView: Retry → `primary loading={isRetrying}` | Task 2, Step 2.4 |
| NotificationLogView: isRetrying state introduced | Task 2, Step 2.3 |
| OidcProvidersSettings: Add Provider → `primary` | Task 3, Step 3.7 |
| OidcProvidersSettings: Edit → `secondary sm` | Task 3, Step 3.8 |
| OidcProvidersSettings: Deactivate → `secondary sm loading={togglingProviderId === provider.id}` | Task 3, Step 3.9 |
| OidcProvidersSettings: Activate → `secondary sm loading={togglingProviderId === provider.id}` | Task 3, Step 3.10 |
| OidcProvidersSettings: Delete → `danger sm` | Task 3, Step 3.11 |
| OidcProvidersSettings: Cancel → `secondary` | Task 3, Step 3.12 |
| OidcProvidersSettings: modal submit → `primary loading={saving}` + static conditional text | Task 3, Step 3.13 |
| OidcProvidersSettings: saving state introduced | Task 3, Step 3.4 |
| OidcProvidersSettings: togglingProviderId state introduced | Task 3, Step 3.4 |
| No text-swap expressions ("Saving...", "Creating...") anywhere | All tasks |
| Form inputs (`<input>`, `<textarea>`, `<select>`) untouched | All tasks — deferred to #3e2 |
| Full frontend gate | Task 5 |
| Playwright re-baseline | Task 6 |
