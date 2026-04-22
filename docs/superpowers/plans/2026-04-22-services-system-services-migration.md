# Services + System-Services Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create the shared `EllipsisIcon` component and migrate all raw Skeleton
`preset-*`/`btn` button elements in `services/+page.svelte` and
`system-services/+page.svelte` to the `<Button>` primitive.

**Architecture:** EllipsisIcon creation first (shared dependency for Wave 5 #3j), then
one task per route file; filter chips, row ellipsis triggers, and modal footers are the
primary migration sites.

**Tech Stack:** Svelte 5, SvelteKit, TypeScript, Vitest, Playwright

---

## Dependency

**Blocks on:** sub-spec #2 merged (`Button` primitive); sub-spec #2c merged
(`ariaLabel` prop on Button, `--bg-hover` CSS token, `loading→disabled` internal
contract); sub-spec #3b merged (navbar-pill ghost + active-override pattern baseline).

**Blocks:** sub-spec #3j (imports `EllipsisIcon` from the `icons/` directory created
here); sub-spec #3i2 form-input pass (modal selects, inputs, checkboxes deferred until
sub-spec #2b primitives land).

---

## Button site inventory

### `frontend/src/routes/services/+page.svelte`

1. **Capability filter chip — All Services** (line 407): `btn btn-sm {capabilityFilter ===
   'all' ? 'preset-filled-primary-500' : 'preset-tonal'}` → ghost + active-override
2. **Capability filter chip — Agents** (line 411): same pattern → ghost + active-override
3. **Capability filter chip — SSH Agents** (line 417): same pattern → ghost + active-override
4. **Row ellipsis trigger** (line 515): `btn btn-sm preset-tonal` with `&#8943;` unicode →
   ghost + sm + ariaLabel + `{#snippet leadingIcon()}<EllipsisIcon />{/snippet}` + sr-only
5. **Error Retry** (line 532): `btn preset-filled-primary-500` → primary + new `isRetrying`
6. **Merge modal Cancel** (line 649): `btn preset-tonal-surface` → secondary
7. **Merge modal Submit** (line 650): `btn preset-filled-primary-500` with
   `{submitting ? 'Merging...' : 'Merge'}` text-swap → primary + `loading={submitting}` +
   static `Merge` children; `disabled={!mergeTargetId}` (drop `|| submitting`)
8. **Ping modal Cancel** (line 668): `btn preset-tonal-surface` → secondary
9. **Ping modal Submit** (line 669): `btn preset-filled-primary-500` with
   `{submitting ? 'Saving...' : 'Save'}` text-swap → primary + `loading={submitting}` +
   static `Save` children; drop `disabled={submitting}` (covered by `loading`)

### `frontend/src/routes/system-services/+page.svelte`

1. **Status filter chip — All** (line 383): `btn btn-sm {statusFilter === 'all' ?
   'preset-filled-primary-500' : 'preset-tonal'}` → ghost + active-override
2. **Status filter chip — Pending** (line 388): same pattern → ghost + active-override
3. **Status filter chip — Approved** (line 394): same pattern → ghost + active-override
4. **Status filter chip — Rejected** (line 400): same pattern → ghost + active-override
5. **Status filter chip — Deactivated** (line 406): same pattern → ghost + active-override
6. **Row ellipsis trigger** (line 499): `btn btn-sm preset-tonal` with `&#8943;` unicode →
   ghost + sm + ariaLabel + `{#snippet leadingIcon()}<EllipsisIcon />{/snippet}` + sr-only
7. **Error Retry** (line 516): `btn preset-filled-primary-500` → primary + new `isRetrying`
8. **Ping modal Cancel** (line 635): `btn preset-tonal-surface` → secondary
9. **Ping modal Submit** (line 636): `btn preset-filled-primary-500` with
   `{submitting ? 'Saving...' : 'Save'}` text-swap → primary + `loading={submitting}` +
   static `Save` children; drop `disabled={submitting}` (covered by `loading`)

---

## Migration pattern quick reference

| Legacy class | Button shape |
| --- | --- |
| `btn btn-sm preset-filled-primary-500` (filter active) | `<Button variant="ghost" size="sm" class="text-[var(--accent)] bg-[var(--bg-hover)]">` |
| `btn btn-sm preset-tonal` (filter inactive) | `<Button variant="ghost" size="sm">` |
| `btn btn-sm preset-tonal` on ellipsis trigger | `<Button variant="ghost" size="sm" ariaLabel="..." onclick={...}>` + leadingIcon snippet |
| `btn preset-filled-primary-500` on Retry | `<Button variant="primary" loading={isRetrying}>Retry</Button>` |
| `btn preset-tonal-surface` on modal Cancel | `<Button variant="secondary">Cancel</Button>` |
| `btn preset-filled-primary-500` on modal Submit | `<Button variant="primary" loading={submitting}>{label}</Button>` |

---

## Task 1: Create `EllipsisIcon.svelte` (and `icons/` directory)

**Files:**

- Create: `frontend/src/lib/components/icons/EllipsisIcon.svelte`

Neither `frontend/src/lib/components/icons/` nor `EllipsisIcon.svelte` exists in the
repo. Both must be created. This file is a shared dependency for sub-spec #3j and must
land first.

- [ ] **Step 1: Create `frontend/src/lib/components/icons/EllipsisIcon.svelte`**

The file must be a static SVG — no script block, no props — following the same shape as
`PlayIcon` / `ChevronIcon` elsewhere: a `<svg>` with `viewBox`, `width`/`height`
defaulting to `"1em"`, `fill="currentColor"`, `aria-hidden="true"`, and three `<circle>`
elements representing horizontal dots.

Content:

```svelte
<svg
  xmlns="http://www.w3.org/2000/svg"
  viewBox="0 0 16 16"
  width="1em"
  height="1em"
  fill="currentColor"
  aria-hidden="true"
>
  <circle cx="2" cy="8" r="1.5" />
  <circle cx="8" cy="8" r="1.5" />
  <circle cx="14" cy="8" r="1.5" />
</svg>
```

- [ ] **Step 2: Verify the file compiles**

```bash
cd frontend && npm run check 2>&1 | grep -i 'EllipsisIcon'
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/components/icons/EllipsisIcon.svelte
git commit -m "feat(ui): add EllipsisIcon static SVG component for row action triggers (#3i)"
```

---

## Task 2: Write failing unit tests for `services/+page.svelte` migrations

**Files:**

- Modify: `frontend/src/routes/services/services.test.ts`

Read `frontend/src/routes/services/services.test.ts` in full before editing. The existing
mock setup, fixture helpers (`makePage`, `approvedAgent`, `adminUser`), and `vi.mock`
hoisting are essential context. Do NOT alter or remove any existing test. Types needed for
new tests are defined locally inside `services.test.ts`.

- [ ] **Step 1: Add a pending service fixture**

Below the existing `approvedAgent` fixture, add:

```ts
const pendingAgent: ServiceResponse = {
  ...approvedAgent,
  id: 'svc-002',
  friendly_name: 'pending-agent',
  status: 'pending'
};
```

- [ ] **Step 2: Add filter chip tests**

Add a `describe('capability filter chips', ...)` block inside
`describe('Services Page', ...)`:

```ts
describe('capability filter chips', () => {
  beforeEach(() => {
    vi.mocked(api.getServices).mockResolvedValue(makePage([]));
  });

  it('All Services chip is active by default — carries accent/bg-hover fragments',
    async () => {
      render(ServicesPage);
      await waitFor(() =>
        expect(screen.getByRole('button', { name: 'All Services' })).toBeInTheDocument()
      );
      const allChip = screen.getByRole('button', { name: 'All Services' });
      expect(allChip.className).toContain('text-[var(--accent)]');
      expect(allChip.className).toContain('bg-[var(--bg-hover)]');
  });

  it('inactive chips carry no accent/bg-hover fragments', async () => {
    render(ServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Agents' })).toBeInTheDocument()
    );
    for (const label of ['Agents', 'SSH Agents']) {
      const chip = screen.getByRole('button', { name: label });
      expect(chip.className).not.toContain('text-[var(--accent)]');
      expect(chip.className).not.toContain('bg-[var(--bg-hover)]');
    }
  });

  it('clicking Agents chip makes it active and deactivates All Services', async () => {
    render(ServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Agents' })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: 'Agents' }));
    await waitFor(() => {
      const agentsChip = screen.getByRole('button', { name: 'Agents' });
      expect(agentsChip.className).toContain('text-[var(--accent)]');
      expect(agentsChip.className).toContain('bg-[var(--bg-hover)]');
    });
    expect(screen.getByRole('button', { name: 'All Services' }).className)
      .not.toContain('text-[var(--accent)]');
  });

  it('clicking SSH Agents chip makes it active', async () => {
    render(ServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'SSH Agents' })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: 'SSH Agents' }));
    await waitFor(() => {
      const sshChip = screen.getByRole('button', { name: 'SSH Agents' });
      expect(sshChip.className).toContain('text-[var(--accent)]');
    });
  });
});
```

- [ ] **Step 3: Add row ellipsis trigger tests**

Add a `describe('row ellipsis trigger', ...)` block:

```ts
describe('row ellipsis trigger', () => {
  it('renders variant="ghost" size="sm" — bg-transparent and h-[19px]', async () => {
    vi.mocked(api.getServices).mockResolvedValue(makePage([approvedAgent]));
    render(ServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /actions for prod-agent/i }))
        .toBeInTheDocument()
    );
    const trigger = screen.getByRole('button', { name: /actions for prod-agent/i });
    expect(trigger.className).toContain('bg-transparent');
    expect(trigger.className).toContain('h-[19px]');
  });

  it('aria-label matches "Actions for {friendly_name}" including space in name',
    async () => {
      vi.mocked(api.getServices).mockResolvedValue(
        makePage([{ ...approvedAgent, friendly_name: 'my prod agent' }])
      );
      render(ServicesPage);
      await waitFor(() =>
        expect(screen.getByRole('button', { name: 'Actions for my prod agent' }))
          .toBeInTheDocument()
      );
  });

  it('clicking the trigger opens the context menu', async () => {
    vi.mocked(api.getServices).mockResolvedValue(makePage([approvedAgent]));
    render(ServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /actions for prod-agent/i }))
        .toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /actions for prod-agent/i }));
    await waitFor(() =>
      expect(document.querySelector('[data-ui="context-menu-item"]'))
        .toBeInTheDocument()
    );
  });
});
```

- [ ] **Step 4: Add Retry button tests**

Add a `describe('Retry button', ...)` block:

```ts
describe('Retry button', () => {
  it('renders variant="primary" (md size) in error state', async () => {
    vi.mocked(api.getServices).mockRejectedValue(new Error('fetch failed'));
    render(ServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument()
    );
    const retryBtn = screen.getByRole('button', { name: /retry/i });
    expect(retryBtn.className).toContain('h-[23px]');
    expect(retryBtn.className)
      .toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
  });

  it('sets aria-busy="true" during fetch and clears after rejection', async () => {
    vi.mocked(api.getServices).mockRejectedValue(new Error('fetch failed'));
    render(ServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument()
    );
    let resolveReject!: () => void;
    vi.mocked(api.getServices).mockReturnValue(
      new Promise<never>((_, reject) => {
        resolveReject = () => reject(new Error('still failing'));
      })
    );
    fireEvent.click(screen.getByRole('button', { name: /retry/i }));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /retry/i }))
        .toHaveAttribute('aria-busy', 'true')
    );
    resolveReject();
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /retry/i }))
        .not.toHaveAttribute('aria-busy')
    );
  });

  it('clears aria-busy after successful retry and hides the Retry button', async () => {
    vi.mocked(api.getServices).mockRejectedValue(new Error('fetch failed'));
    render(ServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument()
    );
    vi.mocked(api.getServices).mockResolvedValue(makePage([approvedAgent]));
    fireEvent.click(screen.getByRole('button', { name: /retry/i }));
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: /retry/i })).not.toBeInTheDocument()
    );
    expect(screen.getByText('prod-agent')).toBeInTheDocument();
  });
});
```

- [ ] **Step 5: Add Merge modal footer tests**

Add a `describe('Merge modal footer', ...)` block. The outer `beforeEach` sets
`adminUser` as the current user; this block sets a pending service row.

```ts
describe('Merge modal footer', () => {
  beforeEach(() => {
    vi.mocked(api.getServices).mockResolvedValue(makePage([pendingAgent]));
  });

  async function openMergeModal() {
    render(ServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /actions for pending-agent/i }))
        .toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /actions for pending-agent/i }));
    await waitFor(() => expect(screen.getByText('Merge Into...')).toBeInTheDocument());
    fireEvent.click(screen.getByText('Merge Into...'));
    await waitFor(() => expect(screen.getByText('Merge Service')).toBeInTheDocument());
  }

  it('Cancel renders variant="secondary"', async () => {
    await openMergeModal();
    const cancelBtn = screen.getByRole('button', { name: /cancel/i });
    expect(cancelBtn.className).toContain('bg-[var(--bg-raised)]');
    expect(cancelBtn.className).toContain('border');
  });

  it('Merge submit renders variant="primary" with static "Merge" label', async () => {
    await openMergeModal();
    const mergeBtn = screen.getByRole('button', { name: /^merge$/i });
    expect(mergeBtn.className)
      .toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
    expect(mergeBtn.textContent).not.toContain('Merging');
  });

  it('Merge submit is disabled when no target is selected', async () => {
    await openMergeModal();
    expect(screen.getByRole('button', { name: /^merge$/i })).toBeDisabled();
  });

  it('loading=true sets aria-busy and disables the Merge submit', async () => {
    vi.mocked(api.mergeService).mockReturnValue(new Promise(() => {}));
    vi.mocked(api.getServices).mockResolvedValue(
      makePage([
        pendingAgent,
        { ...approvedAgent, id: 'svc-target', capabilities: ['software_discovery'] }
      ])
    );
    await openMergeModal();
    const select = document.querySelector('select') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'svc-target' } });
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /^merge$/i })).not.toBeDisabled()
    );
    fireEvent.click(screen.getByRole('button', { name: /^merge$/i }));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /^merge$/i }))
        .toHaveAttribute('aria-busy', 'true')
    );
    expect(screen.getByRole('button', { name: /^merge$/i })).toBeDisabled();
  });
});
```

- [ ] **Step 6: Add Ping modal footer tests (services)**

Add a `describe('Ping modal footer (services)', ...)` block:

```ts
describe('Ping modal footer (services)', () => {
  beforeEach(() => {
    vi.mocked(api.getServices).mockResolvedValue(makePage([approvedAgent]));
  });

  async function openPingModal() {
    render(ServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /actions for prod-agent/i }))
        .toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /actions for prod-agent/i }));
    await waitFor(() =>
      expect(screen.getByText('Edit Ping Interval')).toBeInTheDocument()
    );
    fireEvent.click(screen.getByText('Edit Ping Interval'));
    await waitFor(() =>
      expect(screen.getByRole('heading', { name: 'Edit Ping Interval' }))
        .toBeInTheDocument()
    );
  }

  it('Cancel renders variant="secondary"', async () => {
    await openPingModal();
    const cancelBtn = screen.getByRole('button', { name: /cancel/i });
    expect(cancelBtn.className).toContain('bg-[var(--bg-raised)]');
  });

  it('Save renders variant="primary" with static "Save" label', async () => {
    await openPingModal();
    const saveBtn = screen.getByRole('button', { name: /^save$/i });
    expect(saveBtn.className)
      .toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
    expect(saveBtn.textContent).not.toContain('Saving');
  });

  it('Save shows aria-busy during submit and "Saving..." text never appears', async () => {
    vi.mocked(api.updateService).mockReturnValue(new Promise(() => {}));
    await openPingModal();
    fireEvent.click(screen.getByRole('button', { name: /^save$/i }));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /^save$/i }))
        .toHaveAttribute('aria-busy', 'true')
    );
    expect(document.body.textContent).not.toContain('Saving...');
  });
});
```

- [ ] **Step 7: Add ContextMenuItem out-of-scope regression guard**

```ts
it('ContextMenuItem entries are not wrapped in <Button> (scope guard for #3k)',
  async () => {
    vi.mocked(api.getServices).mockResolvedValue(makePage([pendingAgent]));
    render(ServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /actions for pending-agent/i }))
        .toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /actions for pending-agent/i }));
    await waitFor(() =>
      expect(document.querySelector('[data-ui="context-menu-item"]'))
        .toBeInTheDocument()
    );
    const menuItems = document.querySelectorAll('[data-ui="context-menu-item"]');
    expect(menuItems.length).toBeGreaterThan(0);
    for (const item of menuItems) {
      expect(item.closest('button[class*="h-[23px]"]\')).toBeNull();
      expect(item.closest('button[class*="h-[19px]"]\')).toBeNull();
    }
});
```

- [ ] **Step 8: Run tests (expect failures before implementation)**

```bash
cd frontend && npx vitest run src/routes/services/services.test.ts 2>&1 | tail -30
```

Expected: new tests fail; existing tests pass.

- [ ] **Step 9: Commit failing tests**

```bash
git add frontend/src/routes/services/services.test.ts
git commit -m "test(services): add failing Button primitive contract tests for #3i migration"
```

---

## Task 3: Migrate `services/+page.svelte`

**Files:**

- Modify: `frontend/src/routes/services/+page.svelte`

Read the file before editing to confirm exact line numbers.

- [ ] **Step 1: Add imports**

In the `<script lang="ts">` block, add after the existing imports:

```ts
import Button from '$lib/components/Button.svelte';
import EllipsisIcon from '$lib/components/icons/EllipsisIcon.svelte';
```

- [ ] **Step 2: Add `isRetrying` state variable**

Immediately after `let submitting: boolean = $state(false);`, add:

```ts
let isRetrying: boolean = $state(false);
```

- [ ] **Step 3: Migrate the 3 capability filter chips**

Replace all three `<button class="btn btn-sm ...">` elements in the "Service Filters"
`<SectionCard>` with `<Button>` primitive calls. Pattern for All Services:

```svelte
<Button
  variant="ghost"
  size="sm"
  class={capabilityFilter === 'all' ? 'text-[var(--accent)] bg-[var(--bg-hover)]' : ''}
  onclick={() => setFilter('all')}
>
  All Services
</Button>
```

Apply the same pattern for Agents (`capabilityFilter === 'software_discovery'`) and SSH
Agents (`capabilityFilter === 'ssh_remote'`). The handler name is `setFilter` (not
`setStatusFilter`).

- [ ] **Step 4: Migrate the row ellipsis trigger**

Replace the `<button class="btn btn-sm preset-tonal" ...>&#8943;</button>` inside the
`{#if hasActions(service)}` block with:

```svelte
<Button
  variant="ghost"
  size="sm"
  ariaLabel="Actions for {service.friendly_name}"
  onclick={(e) => {
    e.stopPropagation();
    toggleMenu(service.id, e.currentTarget);
  }}
>
  {#snippet leadingIcon()}<EllipsisIcon />{/snippet}
  <span class="sr-only">Actions for {service.friendly_name}</span>
</Button>
```

`children: Snippet` is required (non-optional) in Button — the `<span class="sr-only">`
satisfies it without visible output. `ariaLabel` overrides the sr-only text for screen
readers. Use `{#snippet leadingIcon()}` block syntax — NEVER `leadingIcon={EllipsisIcon}`.

- [ ] **Step 5: Migrate the error Retry button**

Replace the `{#snippet errorActions()}` button:

Before:

```svelte
<button class="btn preset-filled-primary-500 mt-3"
  onclick={() => loadServices(currentPage)}>Retry</button>
```

After:

```svelte
<Button
  variant="primary"
  class="mt-3"
  loading={isRetrying}
  onclick={async () => {
    isRetrying = true;
    try {
      await loadServices(currentPage);
    } finally {
      isRetrying = false;
    }
  }}
>
  Retry
</Button>
```

`loadServices` itself is unchanged. `isRetrying` is separate from `submitting` to prevent
spurious spinners on unrelated flows.

- [ ] **Step 6: Migrate the Merge modal footer**

Replace the two `<button>` elements inside the Merge modal's `{#snippet footer()}`.

Before:

```svelte
<button class="btn preset-tonal-surface" onclick={cancelMerge}>Cancel</button>
<button class="btn preset-filled-primary-500"
  disabled={!mergeTargetId || submitting} onclick={executeMerge}>
  {submitting ? 'Merging...' : 'Merge'}
</button>
```

After:

```svelte
<Button variant="secondary" onclick={cancelMerge}>Cancel</Button>
<Button
  variant="primary"
  loading={submitting}
  disabled={!mergeTargetId}
  onclick={executeMerge}
>
  Merge
</Button>
```

The `|| submitting` disjunct is dropped from `disabled` because `loading=true` internally
sets `disabled=true` via the Button primitive's #2c contract. The text-swap expression is
removed — spinner + static label is the locked loading UI contract.

- [ ] **Step 7: Migrate the Ping modal footer (services)**

Replace the two `<button>` elements inside the Ping modal's `{#snippet footer()}`.

Before:

```svelte
<button class="btn preset-tonal-surface" onclick={cancelPingEdit}>Cancel</button>
<button class="btn preset-filled-primary-500" disabled={submitting}
  onclick={executePingEdit}>
  {submitting ? 'Saving...' : 'Save'}
</button>
```

After:

```svelte
<Button variant="secondary" onclick={cancelPingEdit}>Cancel</Button>
<Button variant="primary" loading={submitting} onclick={executePingEdit}>Save</Button>
```

Drop `disabled={submitting}` (primitive handles via `loading`). Remove text-swap.

- [ ] **Step 8: Verify compilation**

```bash
cd frontend && npm run check 2>&1 | grep -i 'services'
```

Expected: no type errors on `services/+page.svelte`.

- [ ] **Step 9: Run unit tests**

```bash
cd frontend && npx vitest run src/routes/services/services.test.ts 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 10: Commit**

```bash
git add frontend/src/routes/services/+page.svelte
git commit -m "refactor(services): migrate filter chips, ellipsis trigger, Retry, and modal footers to Button primitive (#3i)"
```

---

## Task 4: Write failing unit tests for `system-services/+page.svelte` migrations

**Files:**

- Modify: `frontend/src/routes/system-services/system-services.test.ts`

Read `frontend/src/routes/system-services/system-services.test.ts` in full before
editing. Types needed for new tests are defined locally in the test file.

- [ ] **Step 1: Add filter chip tests**

Add a `describe('status filter chips', ...)` block inside
`describe('System Services Route', ...)`:

```ts
describe('status filter chips', () => {
  it('All chip is active by default — carries accent/bg-hover fragments', async () => {
    render(SystemServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'All' })).toBeInTheDocument()
    );
    const allChip = screen.getByRole('button', { name: 'All' });
    expect(allChip.className).toContain('text-[var(--accent)]');
    expect(allChip.className).toContain('bg-[var(--bg-hover)]');
  });

  it('inactive chips carry no accent/bg-hover fragments', async () => {
    render(SystemServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Pending' })).toBeInTheDocument()
    );
    for (const label of ['Pending', 'Approved', 'Rejected', 'Deactivated']) {
      const chip = screen.getByRole('button', { name: label });
      expect(chip.className).not.toContain('text-[var(--accent)]');
      expect(chip.className).not.toContain('bg-[var(--bg-hover)]');
    }
  });

  it('clicking Pending chip makes it active and deactivates All', async () => {
    render(SystemServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Pending' })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: 'Pending' }));
    await waitFor(() => {
      const pendingChip = screen.getByRole('button', { name: 'Pending' });
      expect(pendingChip.className).toContain('text-[var(--accent)]');
      expect(pendingChip.className).toContain('bg-[var(--bg-hover)]');
    });
    expect(screen.getByRole('button', { name: 'All' }).className)
      .not.toContain('text-[var(--accent)]');
  });

  it('clicking Approved chip makes it active', async () => {
    render(SystemServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Approved' })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: 'Approved' }));
    await waitFor(() => {
      const chip = screen.getByRole('button', { name: 'Approved' });
      expect(chip.className).toContain('text-[var(--accent)]');
    });
  });

  it('clicking Rejected chip makes it active', async () => {
    render(SystemServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Rejected' })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: 'Rejected' }));
    await waitFor(() => {
      const chip = screen.getByRole('button', { name: 'Rejected' });
      expect(chip.className).toContain('text-[var(--accent)]');
    });
  });

  it('clicking Deactivated chip makes it active', async () => {
    render(SystemServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Deactivated' })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: 'Deactivated' }));
    await waitFor(() => {
      const chip = screen.getByRole('button', { name: 'Deactivated' });
      expect(chip.className).toContain('text-[var(--accent)]');
    });
  });
});
```

- [ ] **Step 2: Add row ellipsis trigger tests**

The outer `beforeEach` already provides a `pending` row (`scheduler-service`).

```ts
describe('row ellipsis trigger', () => {
  it('renders variant="ghost" size="sm" — bg-transparent and h-[19px]', async () => {
    render(SystemServicesPage);
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: /actions for scheduler-service/i })
      ).toBeInTheDocument()
    );
    const trigger = screen.getByRole(
      'button', { name: /actions for scheduler-service/i }
    );
    expect(trigger.className).toContain('bg-transparent');
    expect(trigger.className).toContain('h-[19px]');
  });

  it('aria-label matches "Actions for {friendly_name}" including space in name',
    async () => {
      vi.mocked(api.getSystemServices).mockResolvedValue(
        makePage([
          {
            id: 'sys-space',
            friendly_name: 'my system svc',
            hostname: 'host-a',
            ip_address: null,
            status: 'pending',
            is_embedded: false,
            yielded_to: [],
            last_seen_at: '2026-02-01T10:00:00Z',
            capabilities: []
          } as unknown as SystemServiceResponse
        ])
      );
      render(SystemServicesPage);
      await waitFor(() =>
        expect(
          screen.getByRole('button', { name: 'Actions for my system svc' })
        ).toBeInTheDocument()
      );
  });

  it('clicking the trigger opens the context menu', async () => {
    render(SystemServicesPage);
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: /actions for scheduler-service/i })
      ).toBeInTheDocument()
    );
    fireEvent.click(
      screen.getByRole('button', { name: /actions for scheduler-service/i })
    );
    await waitFor(() =>
      expect(document.querySelector('[data-ui="context-menu-item"]'))
        .toBeInTheDocument()
    );
  });
});
```

- [ ] **Step 3: Add Retry button tests**

```ts
describe('Retry button', () => {
  it('renders variant="primary" (md default size) in error state', async () => {
    vi.mocked(api.getSystemServices).mockRejectedValue(new Error('network error'));
    render(SystemServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument()
    );
    const retryBtn = screen.getByRole('button', { name: /retry/i });
    expect(retryBtn.className).toContain('h-[23px]');
    expect(retryBtn.className)
      .toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
  });

  it('sets aria-busy="true" during fetch and clears after rejection', async () => {
    vi.mocked(api.getSystemServices).mockRejectedValue(new Error('network error'));
    render(SystemServicesPage);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument()
    );
    let resolveReject!: () => void;
    vi.mocked(api.getSystemServices).mockReturnValue(
      new Promise<never>((_, reject) => {
        resolveReject = () => reject(new Error('still failing'));
      })
    );
    fireEvent.click(screen.getByRole('button', { name: /retry/i }));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /retry/i }))
        .toHaveAttribute('aria-busy', 'true')
    );
    resolveReject();
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /retry/i }))
        .not.toHaveAttribute('aria-busy')
    );
  });
});
```

- [ ] **Step 4: Add Ping modal footer tests**

```ts
describe('Ping modal footer (system-services)', () => {
  const approvedSystemSvc = {
    id: 'sys-approved',
    friendly_name: 'approved-scheduler',
    hostname: 'controller-b',
    ip_address: '10.10.1.6',
    status: 'approved',
    is_embedded: false,
    yielded_to: [],
    last_seen_at: '2026-02-01T10:00:00Z',
    capabilities: []
  } as unknown as SystemServiceResponse;

  beforeEach(() => {
    vi.mocked(api.getSystemServices).mockResolvedValue(makePage([approvedSystemSvc]));
  });

  async function openPingModal() {
    render(SystemServicesPage);
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: /actions for approved-scheduler/i })
      ).toBeInTheDocument()
    );
    fireEvent.click(
      screen.getByRole('button', { name: /actions for approved-scheduler/i })
    );
    await waitFor(() =>
      expect(screen.getByText('Edit Ping Interval')).toBeInTheDocument()
    );
    fireEvent.click(screen.getByText('Edit Ping Interval'));
    await waitFor(() =>
      expect(screen.getByRole('heading', { name: 'Edit Ping Interval' }))
        .toBeInTheDocument()
    );
  }

  it('Cancel renders variant="secondary"', async () => {
    await openPingModal();
    const cancelBtn = screen.getByRole('button', { name: /cancel/i });
    expect(cancelBtn.className).toContain('bg-[var(--bg-raised)]');
  });

  it('Save renders variant="primary" with static "Save" label', async () => {
    await openPingModal();
    const saveBtn = screen.getByRole('button', { name: /^save$/i });
    expect(saveBtn.className)
      .toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
    expect(saveBtn.textContent).not.toContain('Saving');
  });

  it('Save shows aria-busy during submit and "Saving..." text never appears',
    async () => {
      vi.mocked(api.updateSystemService).mockReturnValue(new Promise(() => {}));
      await openPingModal();
      fireEvent.click(screen.getByRole('button', { name: /^save$/i }));
      await waitFor(() =>
        expect(screen.getByRole('button', { name: /^save$/i }))
          .toHaveAttribute('aria-busy', 'true')
      );
      expect(document.body.textContent).not.toContain('Saving...');
  });
});
```

- [ ] **Step 5: Add ContextMenuItem regression guard**

```ts
it('ContextMenuItem entries are not wrapped in <Button> (scope guard for #3k)',
  async () => {
    render(SystemServicesPage);
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: /actions for scheduler-service/i })
      ).toBeInTheDocument()
    );
    fireEvent.click(
      screen.getByRole('button', { name: /actions for scheduler-service/i })
    );
    await waitFor(() =>
      expect(document.querySelector('[data-ui="context-menu-item"]'))
        .toBeInTheDocument()
    );
    const menuItems = document.querySelectorAll('[data-ui="context-menu-item"]');
    expect(menuItems.length).toBeGreaterThan(0);
    for (const item of menuItems) {
      expect(item.closest('button[class*="h-[23px]"]\')).toBeNull();
      expect(item.closest('button[class*="h-[19px]"]\')).toBeNull();
    }
});
```

- [ ] **Step 6: Run tests (expect failures)**

```bash
cd frontend && npx vitest run src/routes/system-services/system-services.test.ts 2>&1 | tail -30
```

Expected: new tests fail; existing tests pass.

- [ ] **Step 7: Commit failing tests**

```bash
git add frontend/src/routes/system-services/system-services.test.ts
git commit -m "test(system-services): add failing Button primitive contract tests for #3i migration"
```

---

## Task 5: Migrate `system-services/+page.svelte`

**Files:**

- Modify: `frontend/src/routes/system-services/+page.svelte`

Read the file before editing to confirm exact line numbers.

- [ ] **Step 1: Add imports**

In the `<script lang="ts">` block, add:

```ts
import Button from '$lib/components/Button.svelte';
import EllipsisIcon from '$lib/components/icons/EllipsisIcon.svelte';
```

- [ ] **Step 2: Add `isRetrying` state variable**

After `let submitting: boolean = $state(false);`, add:

```ts
let isRetrying: boolean = $state(false);
```

- [ ] **Step 3: Migrate the 5 status filter chips**

Replace all five `<button class="btn btn-sm ...">` elements in the "Status Filters"
`<SectionCard>`. Pattern (All chip):

```svelte
<Button
  variant="ghost"
  size="sm"
  class={statusFilter === 'all' ? 'text-[var(--accent)] bg-[var(--bg-hover)]' : ''}
  onclick={() => setFilter('all')}
>
  All
</Button>
```

Apply for all five values: `'all'` → "All", `'pending'` → "Pending", `'approved'` →
"Approved", `'rejected'` → "Rejected", `'deactivated'` → "Deactivated". The handler is
`setFilter` (not `setStatusFilter`).

- [ ] **Step 4: Migrate the row ellipsis trigger**

Replace `<button class="btn btn-sm preset-tonal" ...>&#8943;</button>` inside
`{#if hasActions(service)}` with:

```svelte
<Button
  variant="ghost"
  size="sm"
  ariaLabel="Actions for {service.friendly_name}"
  onclick={(e) => {
    e.stopPropagation();
    toggleMenu(service.id, e.currentTarget);
  }}
>
  {#snippet leadingIcon()}<EllipsisIcon />{/snippet}
  <span class="sr-only">Actions for {service.friendly_name}</span>
</Button>
```

- [ ] **Step 5: Migrate the error Retry button**

Replace in `{#snippet errorActions()}`:

Before:

```svelte
<button class="btn preset-filled-primary-500 mt-3"
  onclick={() => loadServices(currentPage)}>Retry</button>
```

After:

```svelte
<Button
  variant="primary"
  class="mt-3"
  loading={isRetrying}
  onclick={async () => {
    isRetrying = true;
    try {
      await loadServices(currentPage);
    } finally {
      isRetrying = false;
    }
  }}
>
  Retry
</Button>
```

- [ ] **Step 6: Migrate the Ping modal footer**

Replace the two `<button>` elements inside the Ping modal's `{#snippet footer()}`.

Before:

```svelte
<button class="btn preset-tonal-surface" onclick={cancelPingEdit}>Cancel</button>
<button class="btn preset-filled-primary-500" disabled={submitting}
  onclick={executePingEdit}>
  {submitting ? 'Saving...' : 'Save'}
</button>
```

After:

```svelte
<Button variant="secondary" onclick={cancelPingEdit}>Cancel</Button>
<Button variant="primary" loading={submitting} onclick={executePingEdit}>Save</Button>
```

Drop `disabled={submitting}` (primitive handles via `loading`). Remove text-swap.

- [ ] **Step 7: Verify compilation**

```bash
cd frontend && npm run check 2>&1 | grep -i 'system-services'
```

Expected: no type errors on `system-services/+page.svelte`.

- [ ] **Step 8: Run unit tests**

```bash
cd frontend && npx vitest run src/routes/system-services/system-services.test.ts 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 9: Commit**

```bash
git add frontend/src/routes/system-services/+page.svelte
git commit -m "refactor(system-services): migrate filter chips, ellipsis trigger, Retry, and modal footer to Button primitive (#3i)"
```

---

## Task 6: Re-baseline Playwright snapshots

**Files:**

- Modify or create: e2e spec for services and system-services routes

Check whether an existing Playwright spec covers these routes:

```bash
ls frontend/tests/e2e/
```

Read any relevant spec files to understand the `mockAuthApi` pattern before editing.

- [ ] **Step 1: Identify or create the e2e spec**

If a spec exists for `/services` or `/system-services`, read it before editing. If none
exists, create `frontend/tests/e2e/services.spec.ts` following the shape of
`frontend/tests/e2e/button-primitive.spec.ts`.

The spec should cover:

- `/services` — default state (capability = all, pending row present)
- `/services` — capability filter switched to Agents (active chip style asserted)
- `/services` — capability filter switched to SSH Agents
- `/system-services` — default state (status = all)
- `/system-services` — status filter switched to Pending
- `/system-services` — induced error state (mock 500 on `getSystemServices`)

Each scenario × 2 themes (dark + light) using `toHaveScreenshot` with `threshold: 0.005`.

- [ ] **Step 2: Delta verification per parent §9**

Verify these pixel-level deltas in the screenshots:

- Filter chips (`size="sm"`): `h-[19px]` height; active chip shows `--bg-hover` background
  and `--accent` text. Inactive chip shows no colored background.
- Ellipsis trigger (`size="sm"`): `h-[19px]`; three-dot SVG visible instead of `&#8943;`.
- Retry + modal buttons (`size="md"`): `h-[23px]`; primary shows gradient; secondary shows
  raised background.

- [ ] **Step 3: Snapshot masking per parent §3 (total masked area under 15%)**

Apply masks for:

- `last_seen_at` timestamp cells (dynamic content)
- `<Button loading>` spinner rotation area on Retry during fetch window
- Batch-selection count text in `BatchActionBar`
- Toast banners raised by save / batch flows

- [ ] **Step 4: Generate baselines**

```bash
cd frontend && npx playwright test tests/e2e/services.spec.ts --update-snapshots
```

- [ ] **Step 5: Re-run to confirm stability**

```bash
cd frontend && npx playwright test tests/e2e/services.spec.ts
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add "frontend/tests/e2e/services.spec.ts"
git add "frontend/tests/e2e/services.spec.ts-snapshots"
git commit -m "test(e2e): add/re-baseline services + system-services snapshots after Button primitive migration (#3i)"
```

---

## Task 7: Full frontend gate

- [ ] **Step 1: Run full gate**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: every command exits 0.

- [ ] **Step 2: Run full e2e suite**

```bash
cd frontend && npx playwright test 2>&1 | tail -20
```

Expected: 0 failures. All other snapshot suites unaffected.

---

## Commit summary

| # | Commit | Files |
| --- | --- | --- |
| 1 | Create EllipsisIcon SVG | `icons/EllipsisIcon.svelte` |
| 2 | Failing tests — services | `services/services.test.ts` |
| 3 | Migrate services route | `services/+page.svelte` |
| 4 | Failing tests — system-services | `system-services/system-services.test.ts` |
| 5 | Migrate system-services route | `system-services/+page.svelte` |
| 6 | E2e baselines | `tests/e2e/services.spec.ts` + PNGs |

---

## Out-of-scope (do not touch)

- `ContextMenuShell.svelte`, `ContextMenuItem.svelte`, `BatchActionBar.svelte`,
  `ConfirmDialog.svelte`, `BatchResultDialog.svelte`, `ModalShell.svelte` — owned by #3k
- `confirmClass: 'preset-filled-success-500' | 'preset-filled-error-500'` inputs in
  `confirmLabels` and inline batch-confirm expressions — #3k concern
- Form inputs in modal bodies (select, input, checkbox) — deferred to #3i2 after #2b lands
- `<input type="checkbox">` table-header and per-row select checkboxes — deferred to #3i2
