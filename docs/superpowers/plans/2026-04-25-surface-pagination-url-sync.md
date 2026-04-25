# Surface Pagination URL Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist `SurfaceTable` pagination state in the URL so page survives reload and can be shared/bookmarked.

**Architecture:** Route-owns-URL pattern. `[id]/+page.svelte` is the single writer of
`?page_<data_source_id>=N` params. It passes a `pageBySource` map and `onPageChange` callback
down through `SurfaceReadPanel` → `SurfaceRenderer` → `SurfaceTable`. `SurfaceTable` never
calls `goto()` — it only fires the callback and locally mirrors state for instant feedback.
`URLSearchParams` API handles encoding/decoding of `data_source_id` values transparently.

**Tech Stack:** SvelteKit 2 runes mode (`$state`, `$derived`, `$effect`, `$props()`),
`@testing-library/svelte`, vitest, `$app/state` (`page`), `$app/navigation` (`goto`).

---

## File Map

| File | Change |
| --- | --- |
| `src/lib/components/surfaces/SurfaceTable.svelte` | Add `initialPage` + `onPageChange` props; update `handlePageChange`; add prop-sync `$effect` |
| `src/lib/components/surfaces/SurfaceTable.test.ts` | Add 3 new tests covering prop init, callback, and back-nav sync |
| `src/lib/components/surfaces/SurfaceRenderer.svelte` | Thread `pageBySource` + `onPageChange` through all recursive calls; pass `initialPage` to `SurfaceTable` |
| `src/lib/components/surfaces/SurfaceReadPanel.svelte` | Add `pageBySource` + `onPageChange` optional props; pass to both `SurfaceRenderer` instances |
| `src/routes/surfaces/[id]/+page.svelte` | Add `readPageParams`, `$derived pageBySource`, `handlePageChange`; pass to `SurfaceReadPanel` |
| `src/routes/surfaces/surfaces-page.test.ts` | Mock `$app/navigation`; add 1 test for URL-initialised page rendering |

---

## Task 1: Extend `SurfaceTable` with external page control

**Files:**

- Modify: `src/lib/components/surfaces/SurfaceTable.svelte:18-46,194-196`
- Test: `src/lib/components/surfaces/SurfaceTable.test.ts`

- [ ] **Step 1: Write three failing tests**

Append to the bottom of the `describe('SurfaceTable', ...)` block in `SurfaceTable.test.ts`:

```typescript
it('loads from initialPage prop when provided', async () => {
  vi.mocked(invokeSurfaceInteraction)
    .mockReset()
    .mockResolvedValueOnce({
      items: [{ id: 'chan-1', name: 'Alpha' }],
      total: 40,
      page: 2,
      per_page: 20,
      total_pages: 2
    });

  const node: Extract<SurfaceNode, { kind: 'table' }> = {
    kind: 'table',
    data_source_id: 'data.primary',
    columns: [{ key: 'name', label: 'Name' }],
    row_actions: []
  };
  const dataSource: DataSourceDescriptor = {
    data_source_id: 'data.primary',
    kind: { kind: 'provider_query', operation_id: 'list' },
    result_schema: 'array',
    pagination: { default_page_size: 20, max_page_size: 200 },
    refresh_policy: { type: 'manual' }
  };
  const interactions: InteractionDescriptor[] = [
    { interaction_id: 'list', kind: 'data_load', label: 'List', transport: { mode: 'controller_local' } }
  ];

  render(SurfaceTable, {
    surfaceId: 'notifications.email',
    node,
    dataSource,
    dataLoadInteraction: interactions[0],
    interactions,
    initialPage: 2
  });

  expect(await screen.findByText('Alpha')).toBeInTheDocument();
  expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledOnce();
  expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledWith('notifications.email', 'list', {
    params: { page: 2, per_page: 20 },
    target_provider_id: undefined,
    timeout_seconds: undefined
  });
});

it('fires onPageChange callback with data_source_id and new page when page changes', async () => {
  vi.mocked(invokeSurfaceInteraction)
    .mockReset()
    .mockResolvedValueOnce({
      items: [{ id: 'chan-1', name: 'Alpha' }],
      total: 40,
      page: 1,
      per_page: 20,
      total_pages: 2
    })
    .mockResolvedValueOnce({
      items: [{ id: 'chan-2', name: 'Beta' }],
      total: 40,
      page: 2,
      per_page: 20,
      total_pages: 2
    });

  const onPageChange = vi.fn();
  const node: Extract<SurfaceNode, { kind: 'table' }> = {
    kind: 'table',
    data_source_id: 'data.primary',
    columns: [{ key: 'name', label: 'Name' }],
    row_actions: []
  };
  const dataSource: DataSourceDescriptor = {
    data_source_id: 'data.primary',
    kind: { kind: 'provider_query', operation_id: 'list' },
    result_schema: 'array',
    pagination: { default_page_size: 20, max_page_size: 200 },
    refresh_policy: { type: 'manual' }
  };
  const interactions: InteractionDescriptor[] = [
    { interaction_id: 'list', kind: 'data_load', label: 'List', transport: { mode: 'controller_local' } }
  ];

  render(SurfaceTable, {
    surfaceId: 'notifications.email',
    node,
    dataSource,
    dataLoadInteraction: interactions[0],
    interactions,
    onPageChange
  });

  expect(await screen.findByText('Alpha')).toBeInTheDocument();
  await fireEvent.click(screen.getByRole('button', { name: 'Next' }));

  await waitFor(() => {
    expect(onPageChange).toHaveBeenCalledOnce();
    expect(onPageChange).toHaveBeenCalledWith('data.primary', 2);
  });
});

it('syncs currentPage from initialPage prop when it changes (browser back simulation)', async () => {
  vi.mocked(invokeSurfaceInteraction)
    .mockReset()
    .mockResolvedValue({
      items: [{ id: 'chan-1', name: 'Alpha' }],
      total: 40,
      page: 1,
      per_page: 20,
      total_pages: 2
    });

  const node: Extract<SurfaceNode, { kind: 'table' }> = {
    kind: 'table',
    data_source_id: 'data.primary',
    columns: [{ key: 'name', label: 'Name' }],
    row_actions: []
  };
  const dataSource: DataSourceDescriptor = {
    data_source_id: 'data.primary',
    kind: { kind: 'provider_query', operation_id: 'list' },
    result_schema: 'array',
    pagination: { default_page_size: 20, max_page_size: 200 },
    refresh_policy: { type: 'manual' }
  };
  const interactions: InteractionDescriptor[] = [
    { interaction_id: 'list', kind: 'data_load', label: 'List', transport: { mode: 'controller_local' } }
  ];

  const view = render(SurfaceTable, {
    surfaceId: 'notifications.email',
    node,
    dataSource,
    dataLoadInteraction: interactions[0],
    interactions,
    initialPage: 2
  });

  await waitFor(() => {
    expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledWith(
      'notifications.email', 'list',
      expect.objectContaining({ params: expect.objectContaining({ page: 2 }) })
    );
  });

  vi.mocked(invokeSurfaceInteraction).mockClear();

  await view.rerender({
    surfaceId: 'notifications.email',
    node,
    dataSource,
    dataLoadInteraction: interactions[0],
    interactions,
    initialPage: 1
  });

  await waitFor(() => {
    expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledWith(
      'notifications.email', 'list',
      expect.objectContaining({ params: expect.objectContaining({ page: 1 }) })
    );
  });
});
```

- [ ] **Step 2: Run new tests to confirm they all fail**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run test -- --reporter=verbose SurfaceTable.test
```

Expected: 3 new tests fail. Existing 7 tests pass.

- [ ] **Step 3: Extend the props interface in `SurfaceTable.svelte`**

In `src/lib/components/surfaces/SurfaceTable.svelte`, change the `$props()` destructure and add the two new props to the interface:

```svelte
let {
  surfaceId,
  node,
  dataSource,
  dataLoadInteraction,
  interactions = [],
  targetProviderId,
  encryptionContext,
  baseParams = {},
  rows = [],
  initialPage = 1,
  onPageChange
}: {
  surfaceId: string;
  node: Extract<SurfaceNode, { kind: 'table' }>;
  dataSource?: DataSourceDescriptor;
  dataLoadInteraction?: InteractionDescriptor;
  interactions?: InteractionDescriptor[];
  targetProviderId?: string;
  encryptionContext?: SurfaceEncryptionContext;
  baseParams?: Record<string, unknown>;
  rows?: Record<string, unknown>[];
  initialPage?: number;
  onPageChange?: (dataSourceId: string, page: number) => void;
} = $props();
```

- [ ] **Step 4: Initialize `currentPage` from `initialPage` and add prop-sync effect**

Change the `let currentPage = $state(1);` line (line 43) to:

```svelte
let currentPage = $state(initialPage);
```

Then add a new `$effect` directly after the existing reload-event `$effect` block (after line 122, before the `function isRowActionVisible` line):

```svelte
$effect(() => {
  const propPage = initialPage;
  if (propPage !== currentPage) {
    currentPage = propPage;
  }
});
```

- [ ] **Step 5: Update `handlePageChange` to fire the callback**

Change the existing `handlePageChange` function (line 194–196):

```svelte
function handlePageChange(page: number): void {
  currentPage = page;
  onPageChange?.(node.data_source_id, page);
}
```

- [ ] **Step 6: Run all SurfaceTable tests**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run test -- --reporter=verbose SurfaceTable.test
```

Expected: All 10 tests pass.

- [ ] **Step 7: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
git add src/lib/components/surfaces/SurfaceTable.svelte src/lib/components/surfaces/SurfaceTable.test.ts
git commit -m "feat(surfaces): add initialPage prop and onPageChange callback to SurfaceTable"
```

---

## Task 2: Thread props through `SurfaceRenderer`

**Files:**

- Modify: `src/lib/components/surfaces/SurfaceRenderer.svelte:17-38,90-238`

No new test file needed — existing `SurfaceRenderer.test.ts` coverage confirms the component still renders; the integration flows from Task 1 tests.

- [ ] **Step 1: Add props to the interface**

In `src/lib/components/surfaces/SurfaceRenderer.svelte`, extend the `$props()` destructure and
its TypeScript interface to include two new optional props. Change:

```svelte
let {
  surfaceId,
  node,
  interactions = [],
  dataSources = [],
  targetProviderId,
  encryptionContext,
  dataBySource = {},
  baseParams = {},
  requiredContextParam,
  requiredForInteractionIds = []
}: {
  surfaceId: string;
  node: SurfaceNode;
  interactions?: InteractionDescriptor[];
  dataSources?: DataSourceDescriptor[];
  targetProviderId?: string;
  encryptionContext?: SurfaceEncryptionContext;
  dataBySource?: Record<string, unknown>;
  baseParams?: Record<string, unknown>;
  requiredContextParam?: string;
  requiredForInteractionIds?: string[];
} = $props();
```

To:

```svelte
let {
  surfaceId,
  node,
  interactions = [],
  dataSources = [],
  targetProviderId,
  encryptionContext,
  dataBySource = {},
  baseParams = {},
  requiredContextParam,
  requiredForInteractionIds = [],
  pageBySource = {},
  onPageChange
}: {
  surfaceId: string;
  node: SurfaceNode;
  interactions?: InteractionDescriptor[];
  dataSources?: DataSourceDescriptor[];
  targetProviderId?: string;
  encryptionContext?: SurfaceEncryptionContext;
  dataBySource?: Record<string, unknown>;
  baseParams?: Record<string, unknown>;
  requiredContextParam?: string;
  requiredForInteractionIds?: string[];
  pageBySource?: Record<string, number>;
  onPageChange?: (dataSourceId: string, page: number) => void;
} = $props();
```

- [ ] **Step 2: Pass props through the `section` recursive renders**

In the `{#if node.kind === 'section'}` block, each child renders a `<SurfaceRenderer>`. Add
`{pageBySource}` and `{onPageChange}` to every `<SurfaceRenderer>` call inside this block:

```svelte
{#if node.kind === 'section'}
  <div class="space-y-4">
    {#if node.title}
      <h3 class="text-subsection-title font-bold text-[var(--text-primary)]">{node.title}</h3>
    {/if}
    {#each node.children ?? [] as child, idx (idx)}
      <SurfaceRenderer
        {surfaceId}
        node={child}
        {interactions}
        {dataSources}
        {targetProviderId}
        {encryptionContext}
        {dataBySource}
        {baseParams}
        {requiredContextParam}
        {requiredForInteractionIds}
        {pageBySource}
        {onPageChange}
      />
    {/each}
  </div>
```

- [ ] **Step 3: Pass `initialPage` and `onPageChange` to `SurfaceTable`**

In the `{:else if node.kind === 'table'}` block, extend the `<SurfaceTable>` call:

```svelte
{:else if node.kind === 'table'}
  <SurfaceTable
    {surfaceId}
    {node}
    dataSource={findDataSource(node.data_source_id)}
    dataLoadInteraction={findTableDataLoadInteraction(node.data_source_id)}
    {interactions}
    {targetProviderId}
    {encryptionContext}
    {baseParams}
    rows={(dataBySource[node.data_source_id] as Record<string, unknown>[]) ?? []}
    initialPage={pageBySource[node.data_source_id] ?? 1}
    {onPageChange}
  />
```

- [ ] **Step 4: Pass props through the `tabs` recursive render**

In the `{:else if node.kind === 'tabs'}` block, extend the inner `<SurfaceRenderer>` call:

```svelte
        <SurfaceRenderer
          {surfaceId}
          node={tabs[selectedTabIndex].root}
          {interactions}
          {dataSources}
          {targetProviderId}
          {encryptionContext}
          {dataBySource}
          {baseParams}
          {requiredContextParam}
          {requiredForInteractionIds}
          {pageBySource}
          {onPageChange}
        />
```

- [ ] **Step 5: Pass props through the `modal_trigger` recursive renders**

In the `{:else if node.kind === 'modal_trigger'}` block, extend the inner `<SurfaceRenderer>` call for each modal node:

```svelte
            <div class="space-y-4">
              {#each node.modal_nodes ?? [] as child, idx (idx)}
                <SurfaceRenderer
                  {surfaceId}
                  node={child}
                  {interactions}
                  {dataSources}
                  {targetProviderId}
                  {encryptionContext}
                  {dataBySource}
                  {baseParams}
                  {pageBySource}
                  {onPageChange}
                />
              {/each}
            </div>
```

- [ ] **Step 6: Run SurfaceRenderer and SurfaceTable tests**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run test -- --reporter=verbose SurfaceRenderer.test SurfaceTable.test
```

Expected: All tests pass.

- [ ] **Step 7: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
git add src/lib/components/surfaces/SurfaceRenderer.svelte
git commit -m "feat(surfaces): thread pageBySource and onPageChange through SurfaceRenderer"
```

---

## Task 3: Thread props through `SurfaceReadPanel`

**Files:**

- Modify: `src/lib/components/surfaces/SurfaceReadPanel.svelte:13-23,374-416`

- [ ] **Step 1: Add props to the interface**

In `src/lib/components/surfaces/SurfaceReadPanel.svelte`, change the `$props()` destructure. The current props are:

```svelte
let {
  surface,
  read,
  baseParams = {},
  reloadToken = 0
}: {
  surface: SurfaceResponse;
  read?: SurfaceReadResponse;
  baseParams?: Record<string, unknown>;
  reloadToken?: string | number;
} = $props();
```

Change to:

```svelte
let {
  surface,
  read,
  baseParams = {},
  reloadToken = 0,
  pageBySource = {},
  onPageChange
}: {
  surface: SurfaceResponse;
  read?: SurfaceReadResponse;
  baseParams?: Record<string, unknown>;
  reloadToken?: string | number;
  pageBySource?: Record<string, number>;
  onPageChange?: (dataSourceId: string, page: number) => void;
} = $props();
```

- [ ] **Step 2: Pass new props to the targeted `SurfaceRenderer` (first renderer instance)**

In the targeted provider branch (`{:else if descriptor.targeting === 'targeted'}` → `{:else}`
after the provider selector), the `<SurfaceRenderer>` call ends after `{encryptionContext}`.
Add `{pageBySource}` and `{onPageChange}`:

```svelte
        <SurfaceRenderer
          surfaceId={descriptor.surface_id}
          node={descriptor.root_node}
          interactions={read.interactions}
          dataSources={read.data_sources}
          targetProviderId={selectedProvider?.provider_id}
          {encryptionContext}
          {dataBySource}
          baseParams={effectiveBaseParams}
          {pageBySource}
          {onPageChange}
        />
```

- [ ] **Step 3: Pass new props to the universal `SurfaceRenderer` (second renderer instance)**

In the universal branch (`{:else}` at top level, after `{:else if descriptor.targeting === 'targeted'}`),
the `<SurfaceRenderer>` call is:

```svelte
      <SurfaceRenderer
        surfaceId={descriptor.surface_id}
        node={descriptor.root_node}
        interactions={read.interactions}
        dataSources={read.data_sources}
        {dataBySource}
        baseParams={effectiveBaseParams}
        {requiredContextParam}
        {requiredForInteractionIds}
        {pageBySource}
        {onPageChange}
      />
```

- [ ] **Step 4: Run SurfaceReadPanel tests**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run test -- --reporter=verbose SurfaceReadPanel.test
```

Expected: All existing tests pass.

- [ ] **Step 5: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
git add src/lib/components/surfaces/SurfaceReadPanel.svelte
git commit -m "feat(surfaces): thread pageBySource and onPageChange through SurfaceReadPanel"
```

---

## Task 4: Add URL management to the surfaces route

**Files:**

- Modify: `src/routes/surfaces/[id]/+page.svelte`
- Modify: `src/routes/surfaces/surfaces-page.test.ts`

- [ ] **Step 1: Add mock and a new test to `surfaces-page.test.ts`**

Add a `vi.mock` for `$app/navigation` alongside the other `vi.mock(...)` calls near the top of the file (before `import SurfacesPage from './[id]/+page.svelte'`):

```typescript
vi.mock('$app/navigation', () => ({
  goto: vi.fn(async () => {})
}));
```

Add a static import for `goto` immediately after the `vi.mock` blocks, before `import SurfacesPage from './[id]/+page.svelte'`:

```typescript
import { goto } from '$app/navigation';
```

Then add one new test at the bottom of the `describe` block:

```typescript
it('renders normally when the page component is mounted (goto not called on mount)', () => {
  // The $app/state mock at the top of this file has a URL without page params.
  // This verifies the route component does not spontaneously call goto on initial render.
  const surface = buildSurface({
    root_node: { kind: 'text_block', text: 'surface content' }
  });
  const read = buildRead(surface);
  vi.mocked(getSurfaceById).mockReturnValue(surface);
  vi.mocked(getSurfaceReadModel).mockReturnValue(read);

  render(SurfacesPage);

  expect(screen.getByText('surface content')).toBeInTheDocument();
  expect(vi.mocked(goto)).not.toHaveBeenCalled();
});
```

- [ ] **Step 2: Run surfaces-page tests to confirm all existing tests pass with mock in place**

The new test uses `expect(vi.mocked(goto)).not.toHaveBeenCalled()` — a negative assertion
already true before the feature is implemented, so it cannot serve as a TDD red step.
Run here to confirm the mock addition doesn't break any existing tests and the new test passes:

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run test -- --reporter=verbose surfaces-page.test
```

Expected: All tests pass (including the new one).

- [ ] **Step 3: Update `[id]/+page.svelte` with URL management**

The full updated script section of `src/routes/surfaces/[id]/+page.svelte`:

```svelte
<script lang="ts">
  import { page } from '$app/state';
  import { goto } from '$app/navigation';
  import { getUser } from '$lib/auth.svelte';
  import SurfaceReadPanel from '$lib/components/surfaces/SurfaceReadPanel.svelte';
  import {
    getSurfaceById,
    getSurfaceReadLoading,
    getSurfaceReadModel,
    getSurfaceReadRequested,
    getSurfaceRegistryLoaded,
    loadSurfaceReadModels
  } from '$lib/surfaces/registry.svelte';
  import { isSurfaceTabPending } from '$lib/surfaces/read-model';
  import { hasPermissionValue } from '$lib/types';
  import { Callout, PageShell } from '$lib/components/ui';

  let surfaceId = $derived(page.params.id as string);
  let surface = $derived(getSurfaceById(surfaceId));
  let surfaceRead = $derived(surface ? getSurfaceReadModel(surface.surface_id) : undefined);
  let isReadRequested = $derived(surface ? getSurfaceReadRequested(surface.surface_id) : false);
  let isReadLoading = $derived(surface ? getSurfaceReadLoading(surface.surface_id) : false);
  let canViewSurface = $derived(surface ? hasPermissionValue(getUser(), surface.required_permission) : false);
  let isPendingSurfaceRead = $derived(
    surface
      ? isSurfaceTabPending({
          activeTab: surface.surface_id,
          slotSurfaces: [surface],
          readBySurface: surfaceRead ? { [surface.surface_id]: surfaceRead } : {},
          isReadRequested,
          isReadLoading
        })
      : false
  );
  let pageTitle = $derived(surface?.label ?? 'Surface');

  const pageBySource = $derived(readPageParams(page.url));

  function readPageParams(url: URL): Record<string, number> {
    const result: Record<string, number> = {};
    for (const [key, value] of url.searchParams) {
      if (key.startsWith('page_')) {
        const dataSourceId = key.slice(5);
        const num = parseInt(value, 10);
        if (dataSourceId && num >= 1) {
          result[dataSourceId] = num;
        }
      }
    }
    return result;
  }

  function handlePageChange(dataSourceId: string, pageNum: number): void {
    const params = new URLSearchParams(page.url.searchParams);
    const key = `page_${dataSourceId}`;
    if (pageNum <= 1) {
      params.delete(key);
    } else {
      params.set(key, String(pageNum));
    }
    const search = params.toString();
    void goto(search ? `?${search}` : page.url.pathname, {
      replaceState: true,
      keepFocus: true,
      noScroll: true
    });
  }

  $effect(() => {
    if (!surface || !canViewSurface) {
      return;
    }
    if (isReadRequested || isReadLoading) {
      return;
    }
    void loadSurfaceReadModels([surface.surface_id]);
  });
</script>
```

- [ ] **Step 4: Pass new props to `SurfaceReadPanel` in the template**

In the template section of `[id]/+page.svelte`, the `<SurfaceReadPanel>` call currently is:

```svelte
<SurfaceReadPanel {surface} read={surfaceRead} />
```

Change to:

```svelte
<SurfaceReadPanel {surface} read={surfaceRead} {pageBySource} onPageChange={handlePageChange} />
```

- [ ] **Step 5: Run all surfaces-related tests**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run test -- --reporter=verbose surfaces-page.test SurfaceReadPanel.test SurfaceRenderer.test SurfaceTable.test
```

Expected: All tests pass.

- [ ] **Step 6: Run the full frontend test suite and quality checks**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: Clean pass on all checks. Address any TypeScript errors before proceeding.

- [ ] **Step 7: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
git add src/routes/surfaces/[id]/+page.svelte src/routes/surfaces/surfaces-page.test.ts
git commit -m "feat(surfaces): sync SurfaceTable pagination page to URL search params"
```

---

## Self-Review

### Spec coverage

| Requirement | Task |
| --- | --- |
| Page survives reload | Task 4 — URL is source of truth via `$derived pageBySource` |
| Multiple tables on same surface don't conflict | Task 2 — each table keyed by `data_source_id` |
| Encoding safety for `data_source_id` values | `URLSearchParams` API used throughout — handles encoding transparently |
| Browser Back restores page | Task 1 — prop-sync `$effect` reacts when `initialPage` prop changes |
| No spontaneous `goto` on mount | Task 4 — `goto` only called inside `handlePageChange`, never in an `$effect` |
| Existing callers unaffected (e.g. `SurfaceSlot`) | All new props are optional with safe defaults |
| Merges with existing URL params | `handlePageChange` reads current `page.url.searchParams` and merges |

### Placeholder scan

No TBDs, no "similar to" references. All code blocks are complete.

### Type consistency

- `onPageChange: (dataSourceId: string, page: number) => void` — consistent across `SurfaceTable`, `SurfaceRenderer`, `SurfaceReadPanel`, `+page.svelte`
- `pageBySource: Record<string, number>` — consistent across all files
- `initialPage: number` — only in `SurfaceTable`, derived as `pageBySource[node.data_source_id] ?? 1` in `SurfaceRenderer`
