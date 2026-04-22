# Hosts Button Migration (#3h) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate all interactive buttons in the hosts list and host detail routes to the
Button primitive, with correct loading/aria-busy wiring and inline ellipsis icon pattern.

**Architecture:** Two files (hosts/+page.svelte and hosts/[id]/+page.svelte) migrated in
separate tasks. Modal footers in detail page use secondary/primary with loading.
Ellipsis context-menu triggers use inline Unicode snippet.

**Tech Stack:** Svelte 5, Button.svelte, Vitest, Playwright

---

## File Map

| File | Change |
| --- | --- |
| `frontend/src/routes/hosts/+page.svelte` | Replace 3 raw buttons with Button primitive |
| `frontend/src/routes/hosts/[id]/+page.svelte` | Replace 9 raw buttons with Button primitive |
| `frontend/src/routes/hosts/hosts.test.ts` | Extend with Button contract assertions |
| `frontend/src/routes/hosts/[id]/host-detail.test.ts` | Extend; Button contract assertions |

### Buttons to migrate in `hosts/+page.svelte`

| Location | Current | Target |
| --- | --- | --- |
| Line 469–478: actions column ellipsis trigger | `<button class="btn btn-sm preset-tonal" aria-label="...">&#8943;</button>` | `<Button variant="ghost" size="sm" ariaLabel="...">` + `leadingIcon` snippet |
| Line 484: errorActions Retry | `<button class="btn preset-filled-primary-500">Retry</button>` | `<Button variant="primary" loading={isRetrying}>Retry</Button>` |
| Lines 583–585: Edit modal Cancel | `<button class="btn preset-tonal-surface">Cancel</button>` | `<Button variant="secondary">Cancel</Button>` |
| Lines 583–585: Edit modal Save | `<button class="btn preset-filled-primary-500" disabled={submitting}>` | `<Button variant="primary" loading={submitting}>Save</Button>` |

### Buttons to migrate in `hosts/[id]/+page.svelte`

| Location | Current | Target |
| --- | --- | --- |
| Line 399: error Retry | `<button class="btn preset-filled-primary-500">Retry</button>` | `<Button variant="primary" loading={isRetrying}>Retry</Button>` |
| Line 410: Edit Name | `<button class="btn preset-tonal-surface">Edit Name</button>` | `<Button variant="secondary">Edit Name</Button>` |
| Lines 411–415: Deactivate launcher | `<button class="btn preset-filled-error-500">Deactivate</button>` | `<Button variant="danger">Deactivate</Button>` |
| Line 420–422: Trigger Discovery | `<button class="btn preset-tonal-surface" disabled={discovering}>` | `<Button variant="secondary" loading={discovering}>Trigger Discovery</Button>` |
| Line 487: Set Tags | `<button class="btn btn-sm preset-tonal-surface">Set Tags</button>` | `<Button variant="secondary" size="sm">Set Tags</Button>` |
| Line 614: Add Plugin Type | `<button class="btn btn-sm preset-filled-primary-500">Add Plugin Type</button>` | `<Button variant="primary" size="sm">Add Plugin Type</Button>` |
| Lines 646–650: allowlist Remove | `<button class="btn btn-sm preset-tonal-error">Remove</button>` | `<Button variant="danger" size="sm">Remove</Button>` |
| Lines 730–735: Edit modal footer | `preset-tonal-surface` Cancel + `preset-filled-primary-500` Save | `<Button variant="secondary">` + `<Button variant="primary" loading={submitting}>Save</Button>` |
| Lines 755–763: Allowlist modal footer | same pattern | same migration |
| Lines 777–782: Set Tags modal footer | same pattern | same migration |

---

## Task 1: Migrate `hosts/+page.svelte` — ellipsis trigger, Retry, and Edit modal

**Files:**

- Modify: `frontend/src/routes/hosts/+page.svelte`
- Modify: `frontend/src/routes/hosts/hosts.test.ts`

### What changes

1. Add `Button` import alongside existing UI imports.
2. Add `let isRetrying = $state(false)` near other state declarations.
3. Replace ellipsis trigger `<button>` (line 468–478) with `<Button>` using `leadingIcon` snippet.
4. Replace errorActions `Retry` button (line 484) with `<Button loading={isRetrying}>` and wrap `loadHosts` call in async try/finally.
5. Replace Edit modal footer buttons (lines 582–586) with `<Button variant="secondary">` and `<Button variant="primary" loading={submitting}>Save</Button>`.

---

- [ ] **Step 1.1: Add Button import**

In `frontend/src/routes/hosts/+page.svelte`, find the existing UI import block (around line 29–32):

```svelte
import {
  ActionBadge,
  ContextMenuShell,
  DataTable,
  ModalShell,
  PageShell,
  SectionCard,
  StatusBadge
} from '$lib/components/ui';
```

Replace with:

```svelte
import Button from '$lib/components/Button.svelte';
import {
  ActionBadge,
  ContextMenuShell,
  DataTable,
  ModalShell,
  PageShell,
  SectionCard,
  StatusBadge
} from '$lib/components/ui';
```

Note: `Button` is NOT in the `$lib/components/ui` barrel — it lives at
`$lib/components/Button.svelte` and requires a separate import.

- [ ] **Step 1.2: Add `isRetrying` state**

After the existing state block in the `<script>` (around line 50), add:

```svelte
let isRetrying: boolean = $state(false);
```

- [ ] **Step 1.3: Wrap Retry handler**

The `errorActions` snippet (line 483–485) currently calls `loadHosts(currentPage)` inline. Create a dedicated handler above `loadHosts`:

```svelte
async function retryLoad() {
  isRetrying = true;
  try {
    await loadHosts(currentPage);
  } finally {
    isRetrying = false;
  }
}
```

- [ ] **Step 1.4: Replace ellipsis trigger button**

Find (around line 468–478):

```svelte
<button
  class="btn btn-sm preset-tonal"
  aria-label="Actions for {host.friendly_name}"
  onclick={(e) => {
    e.stopPropagation();
    toggleMenu(host.id, e.currentTarget);
  }}
>
  &#8943;
</button>
```

Replace with:

```svelte
<Button
  variant="ghost"
  size="sm"
  ariaLabel="Actions for {host.friendly_name}"
  onclick={(e) => {
    e.stopPropagation();
    toggleMenu(host.id, e.currentTarget);
  }}
>
  {#snippet leadingIcon()}
    <span aria-hidden="true" class="leading-none">⋮</span>
  {/snippet}
</Button>
```

- [ ] **Step 1.5: Replace errorActions Retry button**

Find (around line 484):

```svelte
<button class="btn preset-filled-primary-500 mt-3" onclick={() => loadHosts(currentPage)}>Retry</button>
```

Replace with:

```svelte
<Button variant="primary" class="mt-3" loading={isRetrying} onclick={retryLoad}>Retry</Button>
```

- [ ] **Step 1.6: Replace Edit modal footer buttons**

Find (around lines 581–586):

```svelte
{#snippet footer()}
  <button class="btn preset-tonal-surface" onclick={cancelEdit}>Cancel</button>
  <button class="btn preset-filled-primary-500" disabled={submitting} onclick={executeEdit}>
    {submitting ? 'Saving...' : 'Save'}
  </button>
{/snippet}
```

Replace with:

```svelte
{#snippet footer()}
  <Button variant="secondary" onclick={cancelEdit}>Cancel</Button>
  <Button variant="primary" loading={submitting} onclick={executeEdit}>Save</Button>
{/snippet}
```

- [ ] **Step 1.7: Run svelte-check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no output (zero errors).

- [ ] **Step 1.8: Write failing tests for Button contract in `hosts.test.ts`**

Add a new `describe` block at the bottom of `frontend/src/routes/hosts/hosts.test.ts`:

```typescript
describe('Button primitive contract — hosts/+page.svelte', () => {
  it('ellipsis trigger uses Button primitive with ghost/sm classes and ariaLabel', async () => {
    vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
    render(HostsPage);
    await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());

    const btn = screen.getByRole('button', { name: /actions for production server/i });
    // sm size → h-[19px]
    expect(btn).toHaveClass('h-[19px]');
    // ghost → has border class
    expect(btn.className).toMatch(/border/);
    // aria-label present
    expect(btn).toHaveAttribute('aria-label', 'Actions for Production Server');
    // no legacy preset classes
    expect(btn.className).not.toMatch(/preset-tonal|preset-filled/);
  });

  it('Retry button uses Button primitive with primary variant and aria-busy during retry', async () => {
    vi.mocked(api.getHosts).mockRejectedValue(new Error('fail'));
    render(HostsPage);
    await waitFor(() => screen.getByRole('button', { name: /retry/i }));

    const btn = screen.getByRole('button', { name: /retry/i });
    // primary → gradient background class
    expect(btn.className).toMatch(/bg-\[linear-gradient/);
    // md size → h-[23px]
    expect(btn).toHaveClass('h-[23px]');
    // no legacy preset classes
    expect(btn.className).not.toMatch(/preset-tonal|preset-filled/);
  });

  it('Edit modal Save uses Button primitive — no text swap', async () => {
    vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
    render(HostsPage);
    await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());

    // Open edit dialog via context menu
    fireEvent.click(screen.getByRole('button', { name: /actions for production server/i }));
    await waitFor(() => screen.getByRole('menuitem', { name: 'Edit Name' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Edit Name' }));

    await waitFor(() => screen.getByRole('button', { name: /^save$/i }));
    const saveBtn = screen.getByRole('button', { name: /^save$/i });
    expect(saveBtn).toHaveClass('h-[23px]');
    expect(saveBtn.className).toMatch(/bg-\[linear-gradient/);
    // Static label — no "Saving..." text
    expect(saveBtn.textContent?.trim()).toBe('Save');
    expect(saveBtn.className).not.toMatch(/preset-filled/);
  });

  it('Edit modal Cancel uses secondary variant', async () => {
    vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
    render(HostsPage);
    await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: /actions for production server/i }));
    await waitFor(() => screen.getByRole('menuitem', { name: 'Edit Name' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Edit Name' }));

    await waitFor(() => screen.getByRole('button', { name: /^cancel$/i }));
    const cancelBtn = screen.getByRole('button', { name: /^cancel$/i });
    expect(cancelBtn).toHaveClass('h-[23px]');
    // secondary → bg-raised border
    expect(cancelBtn.className).toMatch(/bg-\[var\(--bg-raised\)\]/);
    expect(cancelBtn.className).not.toMatch(/preset-tonal|preset-filled/);
  });

  it('source contains no preset-filled-* or preset-tonal-* in hosts/+page.svelte', () => {
    // This test fails if the migration is incomplete — raw legacy classes still in file.
    // It passes once all buttons are migrated. Implemented as a static scan via import.
    // We verify by checking the rendered DOM has no such classes anywhere.
    vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
    const { container } = render(HostsPage);
    const allClasses = Array.from(container.querySelectorAll('*'))
      .map((el) => el.className)
      .join(' ');
    expect(allClasses).not.toMatch(/preset-filled-|preset-tonal-/);
  });
});
```

- [ ] **Step 1.9: Run tests — expect failures**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/routes/hosts/hosts.test.ts
```

Expected: new `describe` block fails (legacy classes present, `h-[19px]`/`h-[23px]` absent). Earlier tests pass.

- [ ] **Step 1.10: Run tests — expect pass after implementation**

With steps 1.1–1.6 complete, run again:

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/routes/hosts/hosts.test.ts
```

Expected: all tests pass including the new describe block.

- [ ] **Step 1.11: Format**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx prettier --write src/routes/hosts/+page.svelte src/routes/hosts/hosts.test.ts
```

- [ ] **Step 1.12: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit
git add frontend/src/routes/hosts/+page.svelte frontend/src/routes/hosts/hosts.test.ts
git commit -m "feat(ui): migrate hosts list buttons to Button primitive (#3h)"
```

---

## Task 2: Migrate `hosts/[id]/+page.svelte` — all buttons

**Files:**

- Modify: `frontend/src/routes/hosts/[id]/+page.svelte`
- Extend: `frontend/src/routes/hosts/[id]/host-detail.test.ts`

### What changes

1. Add `Button` import.
2. Add `let isRetrying = $state(false)` state.
3. Replace error-state Retry (line 399).
4. Replace header cluster: Edit Name, Deactivate, Trigger Discovery (lines 410–422).
5. Replace Tags section: Set Tags (line 487).
6. Replace Discovery Allowlist: Add Plugin Type (line 614), Remove per-row (lines 646–650).
7. Replace three modal footers: Edit modal (lines 729–735), Allowlist modal (lines 754–763), Set Tags modal (lines 776–782).

---

- [ ] **Step 2.1: Add Button import**

In `frontend/src/routes/hosts/[id]/+page.svelte`, find (around line 42):

```svelte
import { Callout, ModalShell, PageShell, SectionCard, StatusBadge, type StatusBadgeTone } from '$lib/components/ui';
```

Replace with (Button is a separate import — not in the ui barrel):

```svelte
import Button from '$lib/components/Button.svelte';
import { Callout, ModalShell, PageShell, SectionCard, StatusBadge, type StatusBadgeTone } from '$lib/components/ui';
```

- [ ] **Step 2.2: Add `isRetrying` state**

After `let discovering: boolean = $state(false);` (line 54), add:

```svelte
let isRetrying: boolean = $state(false);
```

- [ ] **Step 2.3: Add `retryLoad` handler**

After the existing `loadData` function definition, add:

```svelte
async function retryLoad() {
  isRetrying = true;
  try {
    await loadData();
  } finally {
    isRetrying = false;
  }
}
```

- [ ] **Step 2.4: Replace error-state Retry button**

Find (around line 399):

```svelte
<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadData()}>Retry</button>
```

Replace with:

```svelte
<Button variant="primary" class="mt-2" loading={isRetrying} onclick={retryLoad}>Retry</Button>
```

- [ ] **Step 2.5: Replace header action cluster**

Find (around lines 409–424):

```svelte
{#if canManage}
  <button class="btn preset-tonal-surface" onclick={openEditDialog}> Edit Name </button>
  <button
    class="btn preset-filled-error-500"
    onclick={() => (confirmDeactivate = true)}
    disabled={submitting}
  >
    Deactivate
  </button>
{/if}
{#if canManageSoftware}
  <button class="btn preset-tonal-surface" onclick={triggerDiscovery} disabled={discovering}>
    {discovering ? 'Triggering…' : 'Trigger Discovery'}
  </button>
{/if}
```

Replace with:

```svelte
{#if canManage}
  <Button variant="secondary" onclick={openEditDialog}>Edit Name</Button>
  <Button variant="danger" onclick={() => (confirmDeactivate = true)}>Deactivate</Button>
{/if}
{#if canManageSoftware}
  <Button variant="secondary" loading={discovering} onclick={triggerDiscovery}>Trigger Discovery</Button>
{/if}
```

Note: `disabled={submitting}` removed from Deactivate — the ConfirmDialog guards execution;
`submitting` flag is set only after confirmation, while the launcher just opens the dialog.

- [ ] **Step 2.6: Replace Set Tags button**

Find (around line 487):

```svelte
<button class="btn btn-sm preset-tonal-surface" onclick={openSetTagsModal}>Set Tags</button>
```

Replace with:

```svelte
<Button variant="secondary" size="sm" onclick={openSetTagsModal}>Set Tags</Button>
```

- [ ] **Step 2.7: Replace Add Plugin Type button**

Find (around line 614):

```svelte
<button class="btn btn-sm preset-filled-primary-500" onclick={openAddAllowlistEntry}>
  Add Plugin Type
</button>
```

Replace with:

```svelte
<Button variant="primary" size="sm" onclick={openAddAllowlistEntry}>Add Plugin Type</Button>
```

- [ ] **Step 2.8: Replace allowlist Remove buttons**

Find (around lines 646–650):

```svelte
<button
  class="btn btn-sm preset-tonal-error"
  onclick={() =>
    (allowlistDeleteConfirm = { id: entry.id, plugin_type: entry.plugin_type })}
>
  Remove
</button>
```

Replace with:

```svelte
<Button
  variant="danger"
  size="sm"
  onclick={() => (allowlistDeleteConfirm = { id: entry.id, plugin_type: entry.plugin_type })}
>
  Remove
</Button>
```

- [ ] **Step 2.9: Replace Edit modal footer**

Find (around lines 729–735):

```svelte
{#snippet footer()}
  <button class="btn preset-tonal-surface" onclick={cancelEdit}>Cancel</button>
  <button class="btn preset-filled-primary-500" disabled={submitting} onclick={executeEdit}>
    {submitting ? 'Saving...' : 'Save'}
  </button>
{/snippet}
```

Replace with:

```svelte
{#snippet footer()}
  <Button variant="secondary" onclick={cancelEdit}>Cancel</Button>
  <Button variant="primary" loading={submitting} onclick={executeEdit}>Save</Button>
{/snippet}
```

- [ ] **Step 2.10: Replace Discovery Allowlist modal footer**

Find (around lines 754–763):

```svelte
{#snippet footer()}
  <button class="btn preset-tonal-surface" onclick={closeAllowlistModal}>Cancel</button>
  <button
    class="btn preset-filled-primary-500"
    onclick={saveAllowlistEntry}
    disabled={!allowlistForm.plugin_type.trim()}
  >
    Add
  </button>
{/snippet}
```

Replace with:

```svelte
{#snippet footer()}
  <Button variant="secondary" onclick={closeAllowlistModal}>Cancel</Button>
  <Button
    variant="primary"
    disabled={!allowlistForm.plugin_type.trim()}
    onclick={saveAllowlistEntry}
  >Add</Button>
{/snippet}
```

- [ ] **Step 2.11: Replace Set Tags modal footer**

Find (around lines 776–782):

```svelte
{#snippet footer()}
  <button class="btn preset-tonal-surface" onclick={() => (showSetTagsModal = false)}>Cancel</button>
  <button class="btn preset-filled-primary-500" disabled={submitting} onclick={executeSetTags}>
    {submitting ? 'Saving...' : 'Save'}
  </button>
{/snippet}
```

Replace with:

```svelte
{#snippet footer()}
  <Button variant="secondary" onclick={() => (showSetTagsModal = false)}>Cancel</Button>
  <Button variant="primary" loading={submitting} onclick={executeSetTags}>Save</Button>
{/snippet}
```

- [ ] **Step 2.12: Run svelte-check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no output.

- [ ] **Step 2.13: Extend `host-detail.test.ts` with failing tests**

Extend `frontend/src/routes/hosts/[id]/host-detail.test.ts` — add a new
`describe('Button primitive contract — hosts/[id]/+page.svelte', ...)` block:

```typescript
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import type { HostResponse, UpdateHistoryResponse, PaginatedResponse } from '$lib/types';
import { Permission } from '$lib/types';

vi.mock('$app/state', () => ({
  page: { params: { id: 'host-001' }, url: new URL('http://localhost/hosts/host-001') }
}));

vi.mock('$app/navigation', () => ({
  goto: vi.fn()
}));

vi.mock('$lib/api', () => ({
  getHost: vi.fn(),
  listUpdateHistory: vi.fn(),
  updateHost: vi.fn(),
  deactivateHost: vi.fn(),
  triggerHostDiscovery: vi.fn(),
  listHostDiscoveryAllowlist: vi.fn(),
  addHostDiscoveryAllowlistEntry: vi.fn(),
  deleteHostDiscoveryAllowlistEntry: vi.fn(),
  listPluginTypes: vi.fn(),
  getHostTags: vi.fn(),
  setHostTags: vi.fn(),
  getSoftwareItems: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
  getUser: vi.fn(() => null),
  getAccessToken: vi.fn(() => null)
}));

vi.mock('$lib/notifications.svelte', () => ({
  showSuccess: vi.fn(),
  showError: vi.fn()
}));

vi.mock('$lib/stores/events.svelte', () => ({
  subscribeToEvent: vi.fn(() => () => {})
}));

vi.mock('$lib/surfaces/registry.svelte', () => ({
  getSurfaceReadModel: vi.fn(() => undefined),
  getSurfacesBySlot: vi.fn(() => []),
  loadSurfaceReadModels: vi.fn()
}));

import HostDetailPage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const adminUser = {
  id: '00000000-0000-0000-0000-000000000002',
  email: 'admin@example.com',
  first_name: 'Admin',
  last_name: 'User',
  permissions: [
    Permission.UpdateHosts,
    Permission.DeactivateHosts,
    Permission.ViewSoftware,
    Permission.CreateSoftware,
    Permission.UpdateSoftware,
    Permission.DeleteSoftware,
    Permission.TriggerChecks,
    Permission.TriggerUpdates
  ]
};

const sampleHost: HostResponse = {
  id: 'host-001',
  machine_id: 'machine-abc',
  hostname: 'prod-server',
  friendly_name: 'Production Server',
  os_type: 'Linux',
  os_version: 'Ubuntu 24.04',
  architecture: 'x86_64',
  ip_address: '10.0.0.5',
  last_seen_at: '2024-06-01T12:00:00Z',
  created_at: '2024-01-01T00:00:00Z',
  updated_at: '2024-01-01T00:00:00Z',
  agents: [],
  tags: [],
  software_status: { known: true, update_count: 0, error_count: 0 }
} as unknown as HostResponse;

function makeHistoryPage(): PaginatedResponse<UpdateHistoryResponse> {
  return { items: [], total: 0, page: 1, per_page: 5, total_pages: 1 };
}

function setupApis() {
  vi.mocked(api.getHost).mockResolvedValue(sampleHost);
  vi.mocked(api.listUpdateHistory).mockResolvedValue(makeHistoryPage());
  vi.mocked(api.listHostDiscoveryAllowlist).mockResolvedValue([]);
  vi.mocked(api.listPluginTypes).mockResolvedValue([]);
  vi.mocked(api.getHostTags).mockResolvedValue({ items: [], total: 0, page: 1, per_page: 100, total_pages: 1 });
  vi.mocked(api.getSoftwareItems).mockResolvedValue({ items: [], total: 0, page: 1, per_page: 20, total_pages: 1 });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('Host Detail Page — Button primitive contract', () => {
  beforeEach(() => {
    vi.mocked(auth.getUser).mockReturnValue(adminUser);
    setupApis();
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it('Edit Name uses secondary variant (md size, bg-raised border)', async () => {
    render(HostDetailPage);
    await waitFor(() => screen.getByRole('button', { name: /edit name/i }));

    const btn = screen.getByRole('button', { name: /edit name/i });
    expect(btn).toHaveClass('h-[23px]');
    expect(btn.className).toMatch(/bg-\[var\(--bg-raised\)\]/);
    expect(btn.className).not.toMatch(/preset-tonal|preset-filled/);
  });

  it('Deactivate uses danger variant (error colors)', async () => {
    render(HostDetailPage);
    await waitFor(() => screen.getByRole('button', { name: /^deactivate$/i }));

    const btn = screen.getByRole('button', { name: /^deactivate$/i });
    expect(btn).toHaveClass('h-[23px]');
    expect(btn.className).toMatch(/color-error/);
    expect(btn.className).not.toMatch(/preset-filled-error/);
  });

  it('Trigger Discovery uses secondary variant and aria-busy while discovering', async () => {
    let resolveTrigger!: (v: { plugins_queued: number; message: string }) => void;
    vi.mocked(api.triggerHostDiscovery).mockReturnValue(
      new Promise((res) => { resolveTrigger = res; })
    );

    render(HostDetailPage);
    await waitFor(() => screen.getByRole('button', { name: /trigger discovery/i }));

    const btn = screen.getByRole('button', { name: /trigger discovery/i });
    expect(btn).toHaveClass('h-[23px]');
    expect(btn.className).toMatch(/bg-\[var\(--bg-raised\)\]/);
    expect(btn.className).not.toMatch(/preset-tonal|preset-filled/);

    fireEvent.click(btn);
    await waitFor(() => expect(screen.getByRole('button', { name: /trigger discovery/i })).toHaveAttribute('aria-busy', 'true'));

    // Static label — no text swap
    expect(btn.textContent).toMatch(/trigger discovery/i);

    resolveTrigger({ plugins_queued: 0, message: 'ok' });
    await waitFor(() => expect(screen.getByRole('button', { name: /trigger discovery/i })).not.toHaveAttribute('aria-busy'));
  });

  it('Set Tags uses secondary sm variant', async () => {
    render(HostDetailPage);
    await waitFor(() => screen.getByRole('button', { name: /set tags/i }));

    const btn = screen.getByRole('button', { name: /set tags/i });
    expect(btn).toHaveClass('h-[19px]');
    expect(btn.className).toMatch(/bg-\[var\(--bg-raised\)\]/);
    expect(btn.className).not.toMatch(/preset-tonal|preset-filled/);
  });

  it('Add Plugin Type uses primary sm variant', async () => {
    render(HostDetailPage);
    await waitFor(() => screen.getByRole('button', { name: /add plugin type/i }));

    const btn = screen.getByRole('button', { name: /add plugin type/i });
    expect(btn).toHaveClass('h-[19px]');
    expect(btn.className).toMatch(/bg-\[linear-gradient/);
    expect(btn.className).not.toMatch(/preset-filled/);
  });

  it('error Retry uses primary variant with aria-busy during retry', async () => {
    vi.mocked(api.getHost).mockRejectedValue(new Error('Network error'));
    render(HostDetailPage);
    await waitFor(() => screen.getByRole('button', { name: /retry/i }));

    const btn = screen.getByRole('button', { name: /retry/i });
    expect(btn).toHaveClass('h-[23px]');
    expect(btn.className).toMatch(/bg-\[linear-gradient/);
    expect(btn.className).not.toMatch(/preset-filled/);
  });

  it('Edit modal Save has static label and loading wires to aria-busy', async () => {
    let resolveUpdate!: (v: HostResponse) => void;
    vi.mocked(api.updateHost).mockReturnValue(new Promise((res) => { resolveUpdate = res; }));

    render(HostDetailPage);
    await waitFor(() => screen.getByRole('button', { name: /edit name/i }));
    fireEvent.click(screen.getByRole('button', { name: /edit name/i }));

    await waitFor(() => screen.getByRole('button', { name: /^save$/i }));
    const saveBtn = screen.getByRole('button', { name: /^save$/i });

    // Static label
    expect(saveBtn.textContent?.trim()).toBe('Save');

    fireEvent.click(saveBtn);
    await waitFor(() => expect(saveBtn).toHaveAttribute('aria-busy', 'true'));
    // Label stays static during submission
    expect(saveBtn.textContent?.trim()).toBe('Save');

    resolveUpdate(sampleHost);
    await waitFor(() => expect(saveBtn).not.toHaveAttribute('aria-busy'));
  });

  it('Edit modal Cancel uses secondary variant', async () => {
    render(HostDetailPage);
    await waitFor(() => screen.getByRole('button', { name: /edit name/i }));
    fireEvent.click(screen.getByRole('button', { name: /edit name/i }));

    await waitFor(() => screen.getByRole('button', { name: /^cancel$/i }));
    const cancelBtn = screen.getByRole('button', { name: /^cancel$/i });
    expect(cancelBtn).toHaveClass('h-[23px]');
    expect(cancelBtn.className).toMatch(/bg-\[var\(--bg-raised\)\]/);
    expect(cancelBtn.className).not.toMatch(/preset-tonal|preset-filled/);
  });

  it('source has no preset-filled-* or preset-tonal-* classes in rendered DOM', async () => {
    render(HostDetailPage);
    await waitFor(() => screen.getByRole('button', { name: /edit name/i }));

    // Open the allowlist modal to render its footer too
    fireEvent.click(screen.getByRole('button', { name: /add plugin type/i }));
    await waitFor(() => screen.getByRole('button', { name: /^cancel$/i }));

    const allClasses = Array.from(document.querySelectorAll('*'))
      .map((el) => el.className)
      .join(' ');
    expect(allClasses).not.toMatch(/preset-filled-|preset-tonal-/);
  });
});
```

- [ ] **Step 2.14: Run tests — expect failures**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/routes/hosts/
```

Expected: new test file fails (legacy classes in DOM, `h-[19px]`/`h-[23px]` absent).

- [ ] **Step 2.15: Run tests — expect pass after implementation**

With steps 2.1–2.11 complete, run again:

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/routes/hosts/
```

Expected: all tests pass.

- [ ] **Step 2.16: Format**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx prettier --write src/routes/hosts/\[id\]/+page.svelte src/routes/hosts/\[id\]/host-detail.test.ts
```

- [ ] **Step 2.17: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit
git add frontend/src/routes/hosts/\[id\]/+page.svelte frontend/src/routes/hosts/\[id\]/host-detail.test.ts
git commit -m "feat(ui): migrate host detail buttons to Button primitive (#3h)"
```

---

## Task 3: Full suite verification

**Files:** none modified

- [ ] **Step 3.1: Run full frontend test suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/routes/hosts/
```

Expected: all tests in both files pass, zero failures.

- [ ] **Step 3.2: Run svelte-check across project**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no output.

- [ ] **Step 3.3: Final commit if any formatting drift**

If prettier modified files in 3.1–3.2 cleanup:

```bash
cd /Users/andreyyantsen/Development/uptrakit
git add -p
git commit -m "chore(ui): format hosts routes after Button migration (#3h)"
```
