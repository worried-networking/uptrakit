# History Button Migration (#3g) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate all interactive buttons in the history route to the Button primitive, with
correct loading/aria-busy wiring, active-state filter chip pattern, and expand/collapse
toggle using inline SVG snippets.

**Architecture:** Single route file (+page.svelte) plus test extension and e2e baseline.
Filter chips use active-state class override. Expand/collapse uses inline leadingIcon SVG
snippet (no icon component import).

**Tech Stack:** Svelte 5, Button.svelte, Vitest, Playwright

---

## Files

| File | Action | Responsibility |
| --- | --- | --- |
| `frontend/src/routes/history/+page.svelte` | Modify | Replace all raw `<button>` elements with `<Button>` primitive |
| `frontend/src/routes/history/history.test.ts` | Modify | Add Button migration assertions (filter chips, expand toggle, row text matrix) |
| `frontend/src/routes/history/history-trigger-status.test.ts` | Modify | Add modal Cancel/Submit Button assertions |
| `frontend/tests/e2e/history.spec.ts` | Create | Visual parity baseline for history route post-migration |

---

## Button sites inventory (current raw `<button>` elements in +page.svelte)

1. **Line 546** — "Trigger Update" header launcher (`preset-filled-primary-500`)
2. **Lines 551–561** — Status filter chips (`btn-sm`, active: `preset-filled-primary-500`, inactive: `preset-tonal`)
3. **Line 573** — "Retry" inline error fallback (`preset-filled-primary-500`)
4. **Lines 615–624** — Per-row expand toggle (raw `<button>`, `▶ view log` / `▼ hide log`)
5. **Line 711** — Modal Cancel (`preset-tonal-surface`)
6. **Lines 712–718** — Modal Submit Trigger Update (`preset-filled-primary-500`, text-swap on `triggering`)

---

## Task 1: Filter chip migration (inactive + active)

**Files:**

- Modify: `frontend/src/routes/history/+page.svelte` (lines 549–562)
- Modify: `frontend/src/routes/history/history.test.ts`

- [ ] **Step 1: Write failing tests for filter chip rendering**

Add this `describe` block inside the existing `describe('History Route', ...)` in `frontend/src/routes/history/history.test.ts`:

```typescript
describe('filter chips', () => {
  it('renders inactive filter chip as ghost sm with no active class', async () => {
    render(HistoryPage);
    await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

    // 'Completed' chip should be inactive — statusFilter defaults to 'all'
    const completedChip = screen.getByRole('button', { name: 'Completed' });
    expect(completedChip).toBeInTheDocument();
    // ghost variant: has border border-[var(--border-default)]
    expect(completedChip.className).toContain('border-[var(--border-default)]');
    // no active override
    expect(completedChip.className).not.toContain('text-[var(--accent)]');
    expect(completedChip.className).not.toContain('bg-[var(--bg-hover)]');
  });

  it('renders active filter chip with accent + bg-hover class override', async () => {
    // Pre-set URL to status=completed so the chip renders active on mount
    page.url.search = '?status=completed';
    render(HistoryPage);
    await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

    const completedChip = screen.getByRole('button', { name: 'Completed' });
    expect(completedChip.className).toContain('text-[var(--accent)]');
    expect(completedChip.className).toContain('bg-[var(--bg-hover)]');
  });

  it('renders All chip as active by default', async () => {
    render(HistoryPage);
    await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

    const allChip = screen.getByRole('button', { name: 'All' });
    expect(allChip.className).toContain('text-[var(--accent)]');
    expect(allChip.className).toContain('bg-[var(--bg-hover)]');
  });
});
```

- [ ] **Step 2: Run to confirm failure**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/routes/history/history.test.ts 2>&1 | tail -30
```

Expected: FAIL — "ghost sm" / active class assertions fail on raw `<button>` elements.

- [ ] **Step 3: Add Button import to +page.svelte**

In `frontend/src/routes/history/+page.svelte`, add `Button` to the existing ui import line:

```svelte
import { Button } from '$lib/components/ui';
```

The current line (line 24) imports `{ Callout, PageShell, SectionCard, StatusBadge }` from `'$lib/components/ui'` — extend it:

```svelte
import { Button, Callout, PageShell, SectionCard, StatusBadge } from '$lib/components/ui';
```

- [ ] **Step 4: Replace filter chip buttons**

Replace lines 549–563 (the `<div class="flex gap-1 flex-wrap">` block containing the `{#each}` chips) with:

```svelte
<div class="flex gap-1 flex-wrap">
  {#each ['all', 'pending', 'in_progress', 'completed', 'failed'] as s (s)}
    <Button
      variant="ghost"
      size="sm"
      class={statusFilter === s ? 'text-[var(--accent)] bg-[var(--bg-hover)]' : ''}
      onclick={() => {
        currentPage = 1;
        statusFilter = s;
        loadHistory(1);
      }}
    >
      {s === 'in_progress' ? 'In Progress' : s.charAt(0).toUpperCase() + s.slice(1)}
    </Button>
  {/each}
</div>
```

- [ ] **Step 5: Run tests to confirm pass**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/routes/history/history.test.ts 2>&1 | tail -30
```

Expected: filter chip tests PASS. Existing tests still PASS.

- [ ] **Step 6: Check types**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 7: Format**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx prettier --write src/routes/history/+page.svelte src/routes/history/history.test.ts
```

- [ ] **Step 8: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit
git add frontend/src/routes/history/+page.svelte frontend/src/routes/history/history.test.ts
git commit -m "feat(frontend): migrate history filter chips to Button primitive (#3g)"
```

---

## Task 2: Per-row expand toggle migration

**Files:**

- Modify: `frontend/src/routes/history/+page.svelte` (lines 615–624)
- Modify: `frontend/src/routes/history/history.test.ts`

- [ ] **Step 1: Write failing tests for expand toggle**

Add this `describe` block inside `describe('History Route', ...)` in `frontend/src/routes/history/history.test.ts`:

```typescript
describe('per-row expand toggle', () => {
  it('renders View logs for non-interactive idle row', async () => {
    render(HistoryPage);
    await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

    // completedItem: interactive=false, status=completed, not expanded
    const viewButtons = screen.getAllByRole('button', { name: /view logs/i });
    expect(viewButtons.length).toBeGreaterThan(0);
  });

  it('renders Attach terminal for interactive in_progress row when collapsed', async () => {
    render(HistoryPage);
    await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

    // inProgressItem: interactive=true, status=in_progress
    const attachButton = screen.getByRole('button', { name: /attach terminal/i });
    expect(attachButton).toBeInTheDocument();
  });

  it('renders Hide logs after expanding a non-interactive row', async () => {
    render(HistoryPage);
    await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

    // Click expand on completedItem
    const grafanaEntry = screen.getByText('grafana on prod-05').closest('article')!;
    const viewBtn = grafanaEntry.querySelector('button[aria-expanded="false"]') as HTMLElement;
    expect(viewBtn).not.toBeNull();
    await fireEvent.click(viewBtn);

    await waitFor(() => {
      const hideBtn = grafanaEntry.querySelector('button[aria-expanded="true"]') as HTMLElement;
      expect(hideBtn).not.toBeNull();
      expect(hideBtn.textContent).toContain('Hide logs');
    });
  });

  it('renders Close terminal after expanding interactive in_progress row', async () => {
    render(HistoryPage);
    await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

    const pgEntry = screen.getByText('postgresql on prod-03').closest('article')!;
    const attachBtn = pgEntry.querySelector('button[aria-expanded="false"]') as HTMLElement;
    expect(attachBtn).not.toBeNull();
    await fireEvent.click(attachBtn);
    vi.runOnlyPendingTimers();

    await waitFor(() => {
      const closeBtn = pgEntry.querySelector('button[aria-expanded="true"]') as HTMLElement;
      expect(closeBtn).not.toBeNull();
      expect(closeBtn.textContent).toContain('Close terminal');
    });
  });

  it('shows aria-busy=true while wsState=connecting on interactive row', async () => {
    const { connectInteractiveSession } = await import('$lib/interactive');
    // Mock: do not call onStateChange — leave wsState at 'connecting'
    vi.mocked(connectInteractiveSession).mockImplementation(() => ({
      disconnect: vi.fn(),
      sendSignal: vi.fn(),
      sendInput: vi.fn()
    }));

    render(HistoryPage);
    await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

    const pgEntry = screen.getByText('postgresql on prod-03').closest('article')!;
    const attachBtn = pgEntry.querySelector('button[aria-expanded="false"]') as HTMLElement;
    await fireEvent.click(attachBtn);
    vi.runOnlyPendingTimers();

    await waitFor(() => {
      const expandedBtn = pgEntry.querySelector('button[aria-expanded="true"]') as HTMLElement;
      expect(expandedBtn).not.toBeNull();
      expect(expandedBtn).toHaveAttribute('aria-busy', 'true');
    });
  });

  it('renders chevron-down SVG path when row is expanded', async () => {
    render(HistoryPage);
    await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

    const grafanaEntry = screen.getByText('grafana on prod-05').closest('article')!;
    const viewBtn = grafanaEntry.querySelector('button[aria-expanded="false"]') as HTMLElement;
    await fireEvent.click(viewBtn);

    await waitFor(() => {
      const expandedBtn = grafanaEntry.querySelector('button[aria-expanded="true"]') as HTMLElement;
      const path = expandedBtn.querySelector('path');
      expect(path).not.toBeNull();
      // chevron-down path (16×16 filled)
      expect(path!.getAttribute('d')).toBe('M4 6l4 4 4-4');
    });
  });
});
```

- [ ] **Step 2: Run to confirm failure**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/routes/history/history.test.ts 2>&1 | tail -40
```

Expected: FAIL — no `button[aria-expanded]`, `textContent` matching "View logs" / "Attach terminal" etc.

- [ ] **Step 3: Replace the per-row expand toggle in +page.svelte**

Replace lines 614–624 (the raw `<button>` expand toggle inside the article's `items-end` column) with:

```svelte
<Button
  variant="ghost"
  size="sm"
  aria-expanded={expandedId === item.id}
  loading={expandedId === item.id && wsState === 'connecting'}
  onclick={() => toggleExpand(item.id)}
>
  {#snippet leadingIcon()}
    <svg viewBox="0 0 16 16" class="h-4 w-4" fill="currentColor">
      {#if expandedId === item.id}
        <path d="M4 6l4 4 4-4" />
      {:else}
        <path d="M6 8l4-4 4 4" />
      {/if}
    </svg>
  {/snippet}
  {#if item.interactive && item.status === 'in_progress'}
    {expandedId === item.id ? 'Close terminal' : 'Attach terminal'}
  {:else}
    {expandedId === item.id ? 'Hide logs' : 'View logs'}
  {/if}
</Button>
```

Also remove the now-unused `aria-label` attribute that was on the old `<button>` (the accessible name comes from children text now).

- [ ] **Step 4: Run tests to confirm pass**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/routes/history/history.test.ts 2>&1 | tail -40
```

Expected: all new expand toggle tests PASS. All previous tests PASS.

Note: existing test on line 199 uses `name: 'Expand output for nginx on prod-01'` — that old
`aria-label` no longer exists. The new button has no `ariaLabel`; its accessible name is the
children text "View logs". Update that test's `findByRole` query too:

In `history.test.ts`, replace (line 199):

```typescript
const viewLogButton = await screen.findByRole('button', {
  name: 'Expand output for nginx on prod-01'
});
```

with:

```typescript
const nginxEntry = screen.getByText('nginx on prod-01').closest('article')!;
const viewLogButton = nginxEntry.querySelector('button[aria-expanded="false"]') as HTMLElement;
expect(viewLogButton).not.toBeNull();
```

And replace (line 215):

```typescript
const viewLogButton = await screen.findByRole('button', {
  name: 'Expand output for postgresql on prod-03'
});
```

with:

```typescript
const pgEntry = screen.getByText('postgresql on prod-03').closest('article')!;
const viewLogButton = pgEntry.querySelector('button[aria-expanded="false"]') as HTMLElement;
expect(viewLogButton).not.toBeNull();
```

Similarly in `history-trigger-status.test.ts`, replace (line 226):

```typescript
const viewLogButton = screen.getByRole('button', {
  name: 'Expand output for Demo App on Host One'
});
```

with:

```typescript
const demoEntry = screen.getByText('Demo App on Host One').closest('article')!;
const viewLogButton = demoEntry.querySelector('button[aria-expanded="false"]') as HTMLElement;
expect(viewLogButton).not.toBeNull();
```

- [ ] **Step 5: Run all history tests**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/routes/history/ 2>&1 | tail -40
```

Expected: all tests in both history test files PASS.

- [ ] **Step 6: Check types**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 7: Format**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx prettier --write src/routes/history/+page.svelte src/routes/history/history.test.ts src/routes/history/history-trigger-status.test.ts
```

- [ ] **Step 8: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit
git add frontend/src/routes/history/+page.svelte frontend/src/routes/history/history.test.ts frontend/src/routes/history/history-trigger-status.test.ts
git commit -m "feat(frontend): migrate history row expand toggle to Button primitive (#3g)"
```

---

## Task 3: Header launcher + Retry + Modal buttons migration

**Files:**

- Modify: `frontend/src/routes/history/+page.svelte` (lines 546, 573, 711–718)
- Modify: `frontend/src/routes/history/history-trigger-status.test.ts`

- [ ] **Step 1: Write failing tests for modal buttons**

Add this `describe` block to `frontend/src/routes/history/history-trigger-status.test.ts`,
inside the existing `describe('History Trigger Update Modal', ...)`:

```typescript
describe('modal button variants', () => {
  it('Trigger Update header launcher renders primary sm, no loading', async () => {
    render(HistoryPage);
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Update History' })).toBeInTheDocument());

    // There is exactly one "Trigger Update" button visible before modal opens
    const launcherBtn = screen.getByRole('button', { name: 'Trigger Update' });
    // primary variant: has gradient background class
    expect(launcherBtn.className).toContain('bg-[linear-gradient');
    // sm size
    expect(launcherBtn.className).toContain('h-[19px]');
    // no aria-busy
    expect(launcherBtn).not.toHaveAttribute('aria-busy');
  });

  it('modal Cancel renders secondary variant', async () => {
    render(HistoryPage);
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Update History' })).toBeInTheDocument());
    await fireEvent.click(screen.getByRole('button', { name: 'Trigger Update' }));
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Trigger Software Update' })).toBeInTheDocument());

    const cancelBtn = screen.getByRole('button', { name: 'Cancel' });
    // secondary variant: bg-[var(--bg-raised)]
    expect(cancelBtn.className).toContain('bg-[var(--bg-raised)]');
    // md size (default)
    expect(cancelBtn.className).toContain('h-[23px]');
  });

  it('modal Submit renders primary md, static children "Trigger Update"', async () => {
    render(HistoryPage);
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Update History' })).toBeInTheDocument());
    await fireEvent.click(screen.getByRole('button', { name: 'Trigger Update' }));
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Trigger Software Update' })).toBeInTheDocument());

    // Two "Trigger Update" buttons now: launcher (hidden behind modal) + submit
    const allTriggerBtns = screen.getAllByRole('button', { name: 'Trigger Update' });
    const submitBtn = allTriggerBtns[allTriggerBtns.length - 1];
    // primary variant
    expect(submitBtn.className).toContain('bg-[linear-gradient');
    // md size
    expect(submitBtn.className).toContain('h-[23px]');
    // static children — no "Triggering..." text present
    expect(submitBtn.textContent).not.toContain('Triggering');
  });

  it('modal Submit shows spinner via aria-busy when triggering, text stays static', async () => {
    // Stall the trigger call so we can inspect mid-flight state
    let resolveTrigger!: (v: { update_history_id: string; status: string }) => void;
    vi.mocked(api.triggerSoftwareUpdate).mockReturnValue(
      new Promise((res) => { resolveTrigger = res; })
    );

    render(HistoryPage);
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Update History' })).toBeInTheDocument());
    await fireEvent.click(screen.getByRole('button', { name: 'Trigger Update' }));
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Trigger Software Update' })).toBeInTheDocument());

    const selects = screen.getAllByRole('combobox');
    await fireEvent.change(selects[0], { target: { value: 'software-1' } });
    await waitFor(() => expect(screen.getAllByRole('combobox')).toHaveLength(2));
    await fireEvent.change(screen.getAllByRole('combobox')[1], { target: { value: 'host-1' } });
    await fireEvent.input(screen.getByPlaceholderText('e.g. 1.2.3'), { target: { value: '1.1.0' } });

    const allTriggerBtns = screen.getAllByRole('button', { name: 'Trigger Update' });
    const submitBtn = allTriggerBtns[allTriggerBtns.length - 1];
    await fireEvent.click(submitBtn);

    // Mid-flight: aria-busy=true, children text still "Trigger Update" (no text swap)
    await waitFor(() => {
      expect(submitBtn).toHaveAttribute('aria-busy', 'true');
      expect(submitBtn.textContent).not.toContain('Triggering');
    });

    // Resolve so test cleanup works
    resolveTrigger({ update_history_id: 'h-1', status: 'pending' });
  });
});
```

- [ ] **Step 2: Run to confirm failure**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/routes/history/history-trigger-status.test.ts 2>&1 | tail -30
```

Expected: FAIL — raw `<button>` has no gradient class, no `bg-[var(--bg-raised)]`, and "Triggering..." text-swap is still present.

- [ ] **Step 3: Replace header launcher button (line 546)**

Replace:

```svelte
<button class="btn preset-filled-primary-500" onclick={openTriggerModal}>Trigger Update</button>
```

with:

```svelte
<Button variant="primary" size="sm" onclick={openTriggerModal}>Trigger Update</Button>
```

- [ ] **Step 4: Replace Retry button (line 573)**

Replace:

```svelte
<button class="btn preset-filled-primary-500" onclick={() => loadHistory(currentPage)}>Retry</button>
```

with:

```svelte
<Button variant="primary" size="sm" onclick={() => loadHistory(currentPage)}>Retry</Button>
```

- [ ] **Step 5: Replace modal Cancel and Submit buttons (lines 711–718)**

Replace the modal footer `<div class="flex justify-end gap-2">` contents:

```svelte
<button class="btn preset-tonal-surface" onclick={closeTriggerModal}>Cancel</button>
<button
  class="btn preset-filled-primary-500"
  onclick={handleTrigger}
  disabled={!selectedItemId || !selectedHostId || !targetVersion.trim() || triggering}
>
  {triggering ? 'Triggering...' : 'Trigger Update'}
</button>
```

with:

```svelte
<Button variant="secondary" onclick={closeTriggerModal}>Cancel</Button>
<Button
  variant="primary"
  loading={triggering}
  disabled={!selectedItemId || !selectedHostId || !targetVersion.trim()}
  onclick={handleTrigger}
>
  Trigger Update
</Button>
```

Note: `disabled` no longer needs `|| triggering` because `loading={triggering}` sets `inert` on the primitive, which blocks `onclick` automatically.

- [ ] **Step 6: Run all history tests**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/routes/history/ 2>&1 | tail -40
```

Expected: all tests in both files PASS.

- [ ] **Step 7: Check types**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 8: Format**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx prettier --write src/routes/history/+page.svelte src/routes/history/history-trigger-status.test.ts
```

- [ ] **Step 9: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit
git add frontend/src/routes/history/+page.svelte frontend/src/routes/history/history-trigger-status.test.ts
git commit -m "feat(frontend): migrate history header launcher and modal buttons to Button primitive (#3g)"
```

---

## Task 4: Source scan — no preset classes remain

**Files:**

- No code changes — verification only

- [ ] **Step 1: Confirm no preset-filled or preset-tonal remain in history route**

```bash
grep -n "preset-filled\|preset-tonal" /Users/andreyyantsen/Development/uptrakit/frontend/src/routes/history/+page.svelte
```

Expected: no output (zero matches).

- [ ] **Step 2: Confirm no raw btn class remains for interactive elements**

```bash
grep -n 'class="btn' /Users/andreyyantsen/Development/uptrakit/frontend/src/routes/history/+page.svelte
```

Expected: no output.

- [ ] **Step 3: Run full history test suite one final time**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/routes/history/ 2>&1 | tail -20
```

Expected: all PASS.

---

## Task 5: Playwright e2e baseline

**Files:**

- Create: `frontend/tests/e2e/history.spec.ts`

- [ ] **Step 1: Write the e2e spec**

Create `frontend/tests/e2e/history.spec.ts`:

```typescript
import { expect, test } from '@playwright/test';
import {
  PARITY_DYNAMIC_MASK_SELECTOR,
  PARITY_VIEWPORT_PRESETS,
  expectParityScreenshot
} from './parity-config';

const isCanonicalUiParityHost = process.platform === 'darwin';
const canonicalUiParityReason =
  'ui parity screenshot baselines are canonicalized on macOS Chromium to avoid cross-OS rasterization drift';

const mockUser = {
  id: '00000000-0000-0000-0000-000000000201',
  email: 'admin@example.com',
  first_name: 'Admin',
  last_name: 'User',
  permissions: [
    'view_software',
    'trigger_updates',
    'view_services',
    'approve_services',
    'reject_services',
    'remove_services',
    'update_services',
    'view_hosts',
    'manage_hosts',
    'update_hosts',
    'deactivate_hosts',
    'create_software',
    'update_software',
    'delete_software',
    'trigger_checks',
    'manage_scheduler',
    'view_settings',
    'manage_auth_settings',
    'manage_enrollment_tokens',
    'manage_agent_certs',
    'manage_global_settings',
    'view_notifications',
    'update_system_services',
    'view_system_services',
    'view_audit_logs'
  ]
};

const baseHistoryItem = {
  host_id: 'host-001',
  software_item_id: 'sw-001',
  actor_type: 'user',
  actor_id: 'actor-1',
  output: '',
  output_truncated: false,
  interactive: false,
  pre_update_protection_status: null,
  pre_update_protection_summary: null,
  recovery_hint: null,
  created_at: '2026-01-15T08:00:00Z'
};

const historyItems = [
  {
    ...baseHistoryItem,
    id: 'hist-001',
    host_name: 'prod-01',
    software_item_name: 'nginx',
    from_version: '1.24.0',
    to_version: '1.25.0',
    status: 'completed',
    started_at: '2026-01-15T08:00:00Z',
    completed_at: '2026-01-15T08:05:00Z',
    output: 'Update completed successfully.'
  },
  {
    ...baseHistoryItem,
    id: 'hist-002',
    host_name: 'prod-02',
    software_item_name: 'redis',
    from_version: '7.0.0',
    to_version: '7.2.0',
    status: 'failed',
    started_at: '2026-01-15T07:00:00Z',
    completed_at: '2026-01-15T07:10:00Z'
  },
  {
    ...baseHistoryItem,
    id: 'hist-003',
    host_name: 'prod-03',
    software_item_name: 'postgresql',
    from_version: '16.1',
    to_version: '16.2',
    status: 'in_progress',
    interactive: true,
    started_at: '2026-01-15T09:00:00Z',
    completed_at: null
  }
];

async function mockHistoryApi(page: import('@playwright/test').Page, scenario: 'default' | 'filter-completed' | 'in-progress-interactive' = 'default') {
  await page.route('**/api/v1/**', async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname;
    const method = route.request().method();
    const json = (body: unknown, status = 200) =>
      route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

    if (method === 'POST' && path === '/api/v1/auth/refresh') {
      return json({
        access_token: 'test-token',
        refresh_token: 'test-refresh',
        expires_in: 3600,
        token_type: 'Bearer',
        user: mockUser
      });
    }
    if (method === 'GET' && path === '/api/v1/auth/me') return json(mockUser);
    if (method === 'GET' && path === '/api/v1/system/alerts') return json({ alerts: [] });
    if (method === 'GET' && path === '/api/v1/surfaces') return json([]);

    if (method === 'GET' && path === '/api/v1/update-history') {
      const status = url.searchParams.get('status');
      const items = scenario === 'filter-completed'
        ? historyItems.filter(i => i.status === 'completed')
        : scenario === 'in-progress-interactive'
        ? historyItems.filter(i => i.status === 'in_progress')
        : historyItems;
      return json({ items, total: items.length, page: 1, per_page: 25, total_pages: 1 });
    }

    // Block SSE connection
    if (method === 'GET' && path === '/api/v1/admin/events') {
      return route.abort();
    }

    return route.abort();
  });
}

async function setTheme(page: import('@playwright/test').Page, theme: 'dark' | 'light') {
  await page.addInitScript((t) => {
    if (t === 'dark') document.documentElement.classList.add('dark');
    else document.documentElement.classList.remove('dark');
    try { localStorage.setItem('theme', t); } catch { /* ignore */ }
  }, theme);
}

test.use({
  viewport: PARITY_VIEWPORT_PRESETS.desktop,
  locale: 'en-US',
  timezoneId: 'UTC'
});

test.describe('history route visual parity', () => {
  test.skip(!isCanonicalUiParityHost, canonicalUiParityReason);

  test('default feed — light', async ({ page }) => {
    await setTheme(page, 'light');
    await mockHistoryApi(page, 'default');
    await page.goto('/history');
    await page.waitForSelector('[data-ui="history-feed-list"]');
    await page.waitForLoadState('networkidle');

    await expectParityScreenshot({
      page,
      target: page,
      name: 'history-default-light.png',
      viewport: 'desktop',
      maskSelectors: [PARITY_DYNAMIC_MASK_SELECTOR]
    });
  });

  test('default feed — dark', async ({ page }) => {
    await setTheme(page, 'dark');
    await mockHistoryApi(page, 'default');
    await page.goto('/history');
    await page.waitForSelector('[data-ui="history-feed-list"]');
    await page.waitForLoadState('networkidle');

    await expectParityScreenshot({
      page,
      target: page,
      name: 'history-default-dark.png',
      viewport: 'desktop',
      maskSelectors: [PARITY_DYNAMIC_MASK_SELECTOR]
    });
  });

  test('filter active — completed chip selected, light', async ({ page }) => {
    await setTheme(page, 'light');
    await mockHistoryApi(page, 'filter-completed');
    await page.goto('/history?status=completed');
    await page.waitForSelector('[data-ui="history-feed-list"]');
    await page.waitForLoadState('networkidle');

    await expectParityScreenshot({
      page,
      target: page,
      name: 'history-filter-completed-light.png',
      viewport: 'desktop',
      maskSelectors: [PARITY_DYNAMIC_MASK_SELECTOR]
    });
  });

  test('in-progress interactive row — light', async ({ page }) => {
    await setTheme(page, 'light');
    await mockHistoryApi(page, 'in-progress-interactive');
    await page.goto('/history?status=in_progress');
    await page.waitForSelector('[data-ui="history-feed-list"]');
    await page.waitForLoadState('networkidle');

    await expectParityScreenshot({
      page,
      target: page,
      name: 'history-in-progress-interactive-light.png',
      viewport: 'desktop',
      maskSelectors: [PARITY_DYNAMIC_MASK_SELECTOR]
    });
  });
});

test.describe('history route button contract smoke', () => {
  test('filter chips have correct active/inactive classes', async ({ page }) => {
    await mockHistoryApi(page, 'filter-completed');
    await page.goto('/history?status=completed');
    await page.waitForSelector('[data-ui="history-feed-list"]');

    // Active chip: Completed
    const completedChip = page.getByRole('button', { name: 'Completed' });
    const completedClass = await completedChip.getAttribute('class');
    expect(completedClass).toContain('text-[var(--accent)]');
    expect(completedClass).toContain('bg-[var(--bg-hover)]');

    // Inactive chip: Failed
    const failedChip = page.getByRole('button', { name: 'Failed' });
    const failedClass = await failedChip.getAttribute('class');
    expect(failedClass).not.toContain('text-[var(--accent)]');
  });

  test('expand toggle shows Attach terminal for interactive in-progress row', async ({ page }) => {
    await mockHistoryApi(page, 'in-progress-interactive');
    await page.goto('/history?status=in_progress');
    await page.waitForSelector('[data-ui="history-feed-list"]');

    const attachBtn = page.getByRole('button', { name: /attach terminal/i });
    await expect(attachBtn).toBeVisible();
    await expect(attachBtn).toHaveAttribute('aria-expanded', 'false');
  });

  test('no preset-filled-* or preset-tonal-* classes in history DOM', async ({ page }) => {
    await mockHistoryApi(page, 'default');
    await page.goto('/history');
    await page.waitForSelector('[data-ui="history-feed-list"]');

    const presetElements = page.locator('[class*="preset-filled-"],[class*="preset-tonal-"]');
    await expect(presetElements).toHaveCount(0);
  });
});
```

- [ ] **Step 2: Add `data-visual-dynamic` attributes to history page dynamic content**

In `frontend/src/routes/history/+page.svelte`, add `data-visual-dynamic=""` to the dynamic
time spans and in-progress duration labels so they get masked by the parity harness:

Find the `formatRelativeTime` render site (the `<span>` with relative time on line ~612):

```svelte
<span class="text-[10px] text-[var(--text-secondary)]"
  >{formatRelativeTime(item.started_at)}</span
>
```

Change to:

```svelte
<span class="text-[10px] text-[var(--text-secondary)]" data-visual-dynamic=""
  >{formatRelativeTime(item.started_at)}</span
>
```

- [ ] **Step 3: Run the smoke (non-screenshot) Playwright tests only**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx playwright test tests/e2e/history.spec.ts --grep "smoke" 2>&1 | tail -30
```

Expected: "history route button contract smoke" tests PASS (3 tests).

- [ ] **Step 4: Generate visual baselines on macOS Chromium**

Run this only on macOS (the canonical platform per parity-config.ts):

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx playwright test tests/e2e/history.spec.ts --grep "visual parity" --update-snapshots --project=chromium 2>&1 | tail -20
```

Expected: 4 baseline screenshots created in `tests/e2e/history.spec.ts-snapshots/`.

- [ ] **Step 5: Run full Playwright history spec to confirm baselines pass**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx playwright test tests/e2e/history.spec.ts --project=chromium 2>&1 | tail -20
```

Expected: all tests PASS.

- [ ] **Step 6: Format**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx prettier --write tests/e2e/history.spec.ts src/routes/history/+page.svelte
```

- [ ] **Step 7: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit
git add frontend/tests/e2e/history.spec.ts frontend/tests/e2e/history.spec.ts-snapshots/ frontend/src/routes/history/+page.svelte
git commit -m "feat(frontend): add Playwright e2e baseline for history Button migration (#3g)"
```

---

## Task 6: Full frontend gate

**Files:**

- No code changes — gate verification only

- [ ] **Step 1: Lint**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run lint 2>&1 | tail -20
```

Expected: no errors.

- [ ] **Step 2: Format check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run format:check 2>&1 | tail -10
```

Expected: no unformatted files.

- [ ] **Step 3: Type check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run check 2>&1 | tail -20
```

Expected: 0 errors.

- [ ] **Step 4: Full unit test suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run 2>&1 | tail -20
```

Expected: all tests PASS.

- [ ] **Step 5: Build**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run build 2>&1 | tail -20
```

Expected: build succeeds, no errors.

- [ ] **Step 6: Commit gate result (if any formatting fixes were needed)**

Only commit if `format:check` found issues and you ran `npm run format` to fix them:

```bash
cd /Users/andreyyantsen/Development/uptrakit
git add frontend/
git commit -m "chore(frontend): format fixes after history Button migration gate (#3g)"
```
