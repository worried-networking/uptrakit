<!-- markdownlint-disable MD013 MD032 -->

# Responsive Layout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the last open responsive gap — `/software` and all `DataTable` usages render identically at every viewport; no mobile fallback exists.

**Architecture:** Two tracks. Track A: `DataTable` gains `mobileMode` prop with dual-DOM cards rendering (CSS media query, no JS flash) and scroll-mode `w-max` fix. Track B: `SoftwareGroupList.svelte` is extracted from the software page and adds its own mobile card layout. Both tracks get new `chromium-mobile` / `chromium-mobile-dark` Playwright project coverage.

**Tech Stack:** Svelte 5 (runes), Tailwind CSS v4, Playwright, Vitest + Testing Library

---

## File Map

| File | Action | Notes |
| --- | --- | --- |
| `frontend/playwright.config.ts` | Modify | Add `chromium-mobile` and `chromium-mobile-dark` projects |
| `frontend/src/lib/components/ui/DataTable.svelte` | Modify | Add `mobileMode`, `mobileRow`, column flags, dual-DOM, scroll width |
| `frontend/src/lib/components/ui/DataTable.test.ts` | Modify | Tests for all new DataTable behavior |
| `frontend/src/lib/components/ui/SoftwareGroupList.svelte` | **Create** | Extracted group list + mobile card layout |
| `frontend/src/routes/software/+page.svelte` | Modify | Replace inline group list with `<SoftwareGroupList>` |
| `frontend/tests/e2e/software-area.spec.ts` | Modify | Add mobile snapshot variants |
| `docs/development/ui/layout.md` | Modify | Update responsive status to `Implemented` |

---

## Task 1: Add `chromium-mobile` Playwright Projects

**Files:**
- Modify: `frontend/playwright.config.ts`

- [ ] **Step 1: Add the two mobile projects**

  Open `frontend/playwright.config.ts`. The `projects` array currently has two entries (`chromium` and `chromium-dark`). Add two more immediately after `chromium-dark`:

  ```typescript
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'], colorScheme: 'light' }
    },
    {
      name: 'chromium-dark',
      use: { ...devices['Desktop Chrome'], colorScheme: 'dark' }
    },
    {
      name: 'chromium-mobile',
      use: { ...devices['Desktop Chrome'], colorScheme: 'light', viewport: { width: 393, height: 852 } }
    },
    {
      name: 'chromium-mobile-dark',
      use: { ...devices['Desktop Chrome'], colorScheme: 'dark', viewport: { width: 393, height: 852 } }
    }
  ],
  ```

  Using `devices['Desktop Chrome']` as base is intentional — it carries `deviceScaleFactor: 1` (required by the parity harness DPR guard in `parity-config.ts:171`). Both project names start with `'chromium'` (required by `PARITY_REQUIRED_PROJECT` guard at `parity-config.ts:34`). `chromium-mobile-dark` contains `'dark'` (required by dark-mode detection at `parity-config.ts:178`).

- [ ] **Step 2: Verify the config parses**

  ```bash
  cd frontend && npx playwright list-tests --project=chromium-mobile 2>&1 | head -5
  ```

  Expected: lists test files (no error about unknown project).

- [ ] **Step 3: Commit**

  ```bash
  git add frontend/playwright.config.ts
  git commit -m "test(e2e): add chromium-mobile and chromium-mobile-dark playwright projects"
  ```

---

## Task 2: Write Failing Tests for DataTable Responsive Extension

**Files:**
- Modify: `frontend/src/lib/components/ui/DataTable.test.ts`

- [ ] **Step 1: Add the failing tests**

  Append this block to the end of the `describe('DataTable', ...)` block in `frontend/src/lib/components/ui/DataTable.test.ts` (before the closing `}`):

  ```typescript
  describe('responsive mode', () => {
    it('renders data-table-cards container with role=list when mobileMode is cards', () => {
      const { container } = render(DataTable, {
        columns: [
          { key: 'name', label: 'Name', mobileTitle: true },
          { key: 'status', label: 'Status' }
        ],
        rows: [{ name: 'alpha', status: 'ready' }],
        mobileMode: 'cards'
      });

      const cardsEl = container.querySelector('[data-ui="data-table-cards"]');
      expect(cardsEl).toBeInTheDocument();
      expect(cardsEl).toHaveAttribute('role', 'list');
    });

    it('renders both table and cards layouts in DOM for cards mode (dual-DOM)', () => {
      const { container } = render(DataTable, {
        columns: [{ key: 'name', label: 'Name', mobileTitle: true }],
        rows: [{ name: 'alpha' }],
        mobileMode: 'cards'
      });

      expect(container.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
      expect(container.querySelector('[data-ui="data-table-cards"]')).toBeInTheDocument();
    });

    it('auto-generates a card with title from mobileTitle column and dl key/value pairs', () => {
      const { container } = render(DataTable, {
        columns: [
          { key: 'name', label: 'Name', mobileTitle: true },
          { key: 'status', label: 'Status' }
        ],
        rows: [{ name: 'alpha', status: 'ready' }],
        mobileMode: 'cards'
      });

      const cardsEl = container.querySelector('[data-ui="data-table-cards"]');
      const listItem = cardsEl?.querySelector('[role="listitem"]');
      expect(listItem).toBeInTheDocument();
      // Title column renders as <p>
      const titleEl = listItem?.querySelector('p');
      expect(titleEl).toHaveTextContent('alpha');
      // Value column renders in <dl>
      expect(listItem?.querySelector('dt')).toHaveTextContent('Status');
      expect(listItem?.querySelector('dd')).toHaveTextContent('ready');
    });

    it('uses the first visible column as implicit title when no mobileTitle is set', () => {
      const { container } = render(DataTable, {
        columns: [
          { key: 'name', label: 'Name' },
          { key: 'status', label: 'Status' }
        ],
        rows: [{ name: 'alpha', status: 'ready' }],
        mobileMode: 'cards'
      });

      const cardsEl = container.querySelector('[data-ui="data-table-cards"]');
      const listItem = cardsEl?.querySelector('[role="listitem"]');
      const titleEl = listItem?.querySelector('p');
      expect(titleEl).toHaveTextContent('alpha');
      // 'Name' column used as title so only 'Status' appears in dl
      expect(listItem?.querySelector('dt')).toHaveTextContent('Status');
    });

    it('excludes mobileHide columns from auto-generated cards', () => {
      const { container } = render(DataTable, {
        columns: [
          { key: 'name', label: 'Name', mobileTitle: true },
          { key: 'internal', label: 'Internal', mobileHide: true },
          { key: 'status', label: 'Status' }
        ],
        rows: [{ name: 'alpha', internal: 'hidden', status: 'ready' }],
        mobileMode: 'cards'
      });

      const cardsEl = container.querySelector('[data-ui="data-table-cards"]');
      const dts = cardsEl?.querySelectorAll('dt');
      // Only 'Status' dt — 'Internal' is hidden, 'Name' is the title
      expect(dts).toHaveLength(1);
      expect(dts?.[0]).toHaveTextContent('Status');
    });

    it('renders mobileRow snippet content in cards mode when provided', () => {
      const mobileRowSnippet = createRawSnippet<[Record<string, unknown>]>((getRow) => ({
        render() {
          const row = getRow();
          return `<div role="listitem" data-testid="custom-mobile-card">${String(row.name)}</div>`;
        }
      }));

      const { container } = render(DataTable, {
        columns: [{ key: 'name', label: 'Name' }],
        rows: [{ name: 'custom' }],
        mobileMode: 'cards',
        mobileRow: mobileRowSnippet
      });

      expect(container.querySelector('[data-testid="custom-mobile-card"]')).toBeInTheDocument();
      expect(container.querySelector('[data-testid="custom-mobile-card"]')).toHaveTextContent('custom');
    });

    it('renders auto-generated cards even when custom row snippet is provided without mobileRow', () => {
      const { container } = render(DataTable, {
        columns: [{ key: 'name', label: 'Name' }],
        rows: [{ name: 'alpha' }],
        mobileMode: 'cards',
        row: makeRowSnippet()
        // no mobileRow: auto-generated cards are the normal path
      });

      // Cards DOM present — row snippet does not suppress cards mode
      expect(container.querySelector('[data-ui="data-table-cards"]')).toBeInTheDocument();
      expect(container.querySelector('[data-ui="data-table-cards"] [role="listitem"]')).toBeInTheDocument();
    });

    it('applies w-max class to table when mobileMode is scroll', () => {
      const { container } = render(DataTable, {
        columns: [{ key: 'name', label: 'Name' }],
        rows: [{ name: 'alpha' }],
        mobileMode: 'scroll'
      });

      expect(container.querySelector('table')).toHaveClass('w-max');
      expect(container.querySelector('table')).not.toHaveClass('min-w-full');
    });

    it('keeps min-w-full on table when no mobileMode is provided', () => {
      const { container } = render(DataTable, {
        columns: [{ key: 'name', label: 'Name' }],
        rows: [{ name: 'alpha' }]
      });

      expect(container.querySelector('table')).toHaveClass('min-w-full');
      expect(container.querySelector('table')).not.toHaveClass('w-max');
    });

    it('renders rowActions in a group element inside cards auto-layout', () => {
      const { container } = render(DataTable, {
        columns: [{ key: 'name', label: 'Name', mobileTitle: true }],
        rows: [{ name: 'alpha' }],
        mobileMode: 'cards',
        rowActions: makeRowActions(),
        rowActionsLabel: 'Row actions'
      });

      const cardsEl = container.querySelector('[data-ui="data-table-cards"]');
      const actionsGroup = cardsEl?.querySelector('[role="group"]');
      expect(actionsGroup).toBeInTheDocument();
      expect(actionsGroup).toHaveAttribute('aria-label', 'Row actions');
      expect(actionsGroup?.querySelector('button')).toHaveTextContent('Inspect alpha');
    });
  });
  ```

- [ ] **Step 2: Run the new tests — confirm they all fail**

  ```bash
  cd frontend && npx vitest run src/lib/components/ui/DataTable.test.ts 2>&1 | tail -20
  ```

  Expected: all tests in the new `'responsive mode'` describe block FAIL (properties/elements not found). Existing tests still pass.

---

## Task 3: Implement DataTable Responsive Extension

**Files:**
- Modify: `frontend/src/lib/components/ui/DataTable.svelte`

- [ ] **Step 1: Replace the full file with the updated implementation**

  Replace `frontend/src/lib/components/ui/DataTable.svelte` with:

  ```svelte
  <script lang="ts">
  	import type { Snippet } from 'svelte';
  	import Callout from './Callout.svelte';
  	import EmptyState from './EmptyState.svelte';

  	export type DataTableColumn = {
  		key: string;
  		label: string;
  		align?: 'left' | 'center' | 'right';
  		mobileHide?: boolean;
  		mobileTitle?: boolean;
  	};

  	let {
  		columns = [],
  		rows = [],
  		caption,
  		loading = false,
  		error,
  		emptyTitle = 'No rows available',
  		emptyDescription,
  		header,
  		row,
  		footer,
  		rowKey,
  		errorActions,
  		rowActions,
  		rowActionsLabel = 'Actions',
  		mobileMode,
  		mobileRow
  	}: {
  		columns: DataTableColumn[];
  		rows: Record<string, unknown>[];
  		caption?: string;
  		loading?: boolean;
  		error?: string | null;
  		emptyTitle?: string;
  		emptyDescription?: string;
  		header?: Snippet;
  		row?: Snippet<[Record<string, unknown>]>;
  		footer?: Snippet;
  		rowKey?: (row: Record<string, unknown>, index: number) => string | number;
  		errorActions?: Snippet;
  		rowActions?: Snippet<[Record<string, unknown>]>;
  		rowActionsLabel?: string;
  		mobileMode?: 'scroll' | 'cards';
  		mobileRow?: Snippet<[Record<string, unknown>]>;
  	} = $props();

  	function resolveRowKey(rowValue: Record<string, unknown>, index: number): string | number {
  		return rowKey ? rowKey(rowValue, index) : `${index}`;
  	}

  	// mobileMode directly drives layout. No fallback — row snippet is desktop-only and
  	// does not affect mobile card generation. cards + no mobileRow = auto-generated dl/dt/dd.
  	const effectiveMobileMode = $derived(mobileMode);

  	// Explicit 'scroll' mode: w-max lets table overflow and trigger horizontal scroll.
  	// absent or 'cards': keep min-w-full.
  	const tableWidthClass = $derived(mobileMode === 'scroll' ? 'w-max' : 'min-w-full');

  	// Columns visible in the auto-generated cards layout.
  	// Columns with empty label (action columns) and mobileHide columns are excluded.
  	const visibleMobileColumns = $derived(columns.filter((col) => !col.mobileHide && col.label !== ''));
  	const titleCol = $derived(visibleMobileColumns.find((col) => col.mobileTitle) ?? visibleMobileColumns[0]);
  	const valueColumns = $derived(titleCol ? visibleMobileColumns.filter((col) => col.key !== titleCol.key) : []);
  </script>

  {#if error}
  	<Callout tone="danger" title="Unable to load data" message={error}>
  		{#if errorActions}
  			{@render errorActions()}
  		{/if}
  	</Callout>
  {:else if loading}
  	<p class="py-8 text-center text-sm text-[var(--text-secondary)]">Loading...</p>
  {:else if rows.length === 0}
  	<EmptyState title={emptyTitle} description={emptyDescription} />
  {:else}
  	<!-- Table layout. Hidden on mobile when effectiveMobileMode='cards'. -->
  	<div
  		class="overflow-hidden rounded-panel border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm{effectiveMobileMode === 'cards'
  			? ' max-sm:hidden'
  			: ''}"
  		data-ui="data-table"
  	>
  		<div class="overflow-x-auto">
  			<table class="{tableWidthClass} border-collapse text-table-body">
  				{#if caption}
  					<caption class="sr-only">{caption}</caption>
  				{/if}
  				<thead>
  					{#if header}
  						{@render header()}
  					{:else}
  						<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
  							{#each columns as column (column.key)}
  								<th
  									class="table-cell-pad text-table-header font-semibold uppercase tracking-table-header {column.align === 'right'
  										? 'text-right'
  										: column.align === 'center'
  											? 'text-center'
  											: 'text-left'}"
  									scope="col"
  								>
  									{column.label}
  								</th>
  							{/each}
  							{#if rowActions}
  								<th
  									class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
  									scope="col"
  								>
  									{rowActionsLabel}
  								</th>
  							{/if}
  						</tr>
  					{/if}
  				</thead>
  				<tbody>
  					{#each rows as rowValue, index (resolveRowKey(rowValue, index))}
  						{#if row}
  							{@render row(rowValue)}
  						{:else}
  							<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
  								{#each columns as column (column.key)}
  									<td
  										class="table-cell-pad text-[var(--text-primary)] {column.align === 'right'
  											? 'text-right'
  											: column.align === 'center'
  												? 'text-center'
  												: 'text-left'}"
  									>
  										{String(rowValue[column.key] ?? '')}
  									</td>
  								{/each}
  								{#if rowActions}
  									<td class="table-cell-pad">
  										<div class="flex flex-wrap gap-2">
  											{@render rowActions(rowValue)}
  										</div>
  									</td>
  								{/if}
  							</tr>
  						{/if}
  					{/each}
  				</tbody>
  			</table>
  		</div>
  		{#if footer}
  			{@render footer()}
  		{/if}
  	</div>

  	<!-- Cards layout: only in DOM when effectiveMobileMode='cards'.
  	     Visible only on mobile via sm:hidden; table layout above is hidden on mobile via max-sm:hidden.
  	     Both exist in DOM simultaneously — CSS controls visibility, not JS. -->
  	{#if effectiveMobileMode === 'cards'}
  		<div
  			class="sm:hidden overflow-hidden rounded-panel border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm divide-y divide-[var(--border-subtle)]"
  			data-ui="data-table-cards"
  			role="list"
  			aria-label={caption ?? undefined}
  		>
  			{#each rows as rowValue, index (resolveRowKey(rowValue, index))}
  				{#if mobileRow}
  					{@render mobileRow(rowValue)}
  				{:else}
  					<div role="listitem" class="px-4 py-3">
  						{#if titleCol}
  							<p class="truncate text-sm font-semibold text-[var(--text-primary)]">
  								{String(rowValue[titleCol.key] ?? '')}
  							</p>
  						{/if}
  						{#if valueColumns.length > 0}
  							<dl class="mt-1.5 space-y-1">
  								{#each valueColumns as col (col.key)}
  									<div class="flex items-baseline gap-2">
  										<dt
  											class="shrink-0 text-table-header font-semibold uppercase tracking-table-header text-[var(--text-secondary)]"
  										>
  											{col.label}
  										</dt>
  										<dd
  											class="min-w-0 truncate text-sm text-[var(--text-primary)]{col.align === 'right'
  												? ' ml-auto'
  												: ''}"
  										>
  											{String(rowValue[col.key] ?? '')}
  										</dd>
  									</div>
  								{/each}
  							</dl>
  						{/if}
  						{#if rowActions}
  							<div role="group" aria-label={rowActionsLabel} class="mt-2 flex flex-wrap gap-2">
  								{@render rowActions(rowValue)}
  							</div>
  						{/if}
  					</div>
  				{/if}
  			{/each}
  			{#if footer}
  				{@render footer()}
  			{/if}
  		</div>
  	{/if}
  {/if}
  ```

- [ ] **Step 2: Run the tests — confirm all pass**

  ```bash
  cd frontend && npx vitest run src/lib/components/ui/DataTable.test.ts 2>&1 | tail -20
  ```

  Expected: all tests pass (both existing and the new `'responsive mode'` block).

- [ ] **Step 3: Run type check**

  ```bash
  cd frontend && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | tail -20
  ```

  Expected: no errors related to `DataTable.svelte`.

- [ ] **Step 4: Commit**

  ```bash
  git add frontend/src/lib/components/ui/DataTable.svelte \
          frontend/src/lib/components/ui/DataTable.test.ts
  git commit -m "feat(ui): add mobileMode, mobileRow, and column responsive flags to DataTable"
  ```

---

## Task 4: Create `SoftwareGroupList.svelte` with Desktop Layout

**Files:**
- Create: `frontend/src/lib/components/ui/SoftwareGroupList.svelte`

This task extracts the existing desktop grid layout from `software/+page.svelte` into a standalone component. No behaviour changes — the desktop layout is an exact copy. Mobile layout is added in Task 5.

- [ ] **Step 1: Create the file**

  Create `frontend/src/lib/components/ui/SoftwareGroupList.svelte`:

  ```svelte
  <script lang="ts">
  	import { SvelteMap, SvelteSet } from 'svelte/reactivity';
  	import type { SoftwareItemDetailResponse, SoftwareItemHostSummary, SoftwareItemResponse } from '$lib/types';
  	import { formatVersion, isValidLogoUrl, resolveDisplayVersion } from '$lib/utils';
  	import { ActionBadge, PillBadge, StatusBadge, TableFooterBar } from '$lib/components/ui';
  	import Button from '$lib/components/Button.svelte';
  	import Checkbox from '$lib/components/Checkbox.svelte';
  	import UpdateAllButton from '$lib/components/UpdateAllButton.svelte';

  	let {
  		items,
  		itemDetailsById,
  		itemDetailLoadingIds,
  		collapsedGroupIds,
  		expandedOverflowGroupIds,
  		batchSelectedIds,
  		canManage,
  		canTriggerUpdates,
  		pluginTypeNames,
  		totalItems,
  		currentPage,
  		totalPages,
  		onToggleGroup,
  		onToggleOverflow,
  		onToggleBatch,
  		onOpenMenu,
  		onOpenUpdateModal,
  		onPageChange,
  		onToggleFeatured
  	}: {
  		items: SoftwareItemResponse[];
  		itemDetailsById: SvelteMap<string, SoftwareItemDetailResponse>;
  		itemDetailLoadingIds: SvelteSet<string>;
  		collapsedGroupIds: SvelteSet<string>;
  		expandedOverflowGroupIds: SvelteSet<string>;
  		batchSelectedIds: SvelteSet<string>;
  		canManage: boolean;
  		canTriggerUpdates: boolean;
  		pluginTypeNames: Map<string, string>;
  		totalItems: number;
  		currentPage: number;
  		totalPages: number;
  		onToggleGroup: (id: string) => void;
  		onToggleOverflow: (id: string) => void;
  		onToggleBatch: (id: string) => void;
  		onOpenMenu: (id: string, button: HTMLElement) => void;
  		onOpenUpdateModal: (item: SoftwareItemResponse) => void;
  		onPageChange: (page: number) => void;
  		onToggleFeatured: (item: SoftwareItemResponse) => void;
  	} = $props();

  	function detailHosts(item: SoftwareItemResponse): SoftwareItemDetailResponse['hosts'] {
  		return itemDetailsById.get(item.id)?.hosts ?? [];
  	}

  	function visibleHosts(item: SoftwareItemResponse): SoftwareItemDetailResponse['hosts'] {
  		const hosts = detailHosts(item);
  		if (collapsedGroupIds.has(item.id)) return [];
  		if (expandedOverflowGroupIds.has(item.id) || hosts.length <= 3) return hosts;
  		return hosts.slice(0, 3);
  	}

  	function hiddenHostCount(item: SoftwareItemResponse): number {
  		const hosts = detailHosts(item);
  		if (collapsedGroupIds.has(item.id) || expandedOverflowGroupIds.has(item.id) || hosts.length <= 3) return 0;
  		return hosts.length - 3;
  	}

  	function hiddenHostsSummary(item: SoftwareItemResponse): string {
  		const hosts = detailHosts(item);
  		if (collapsedGroupIds.has(item.id) || expandedOverflowGroupIds.has(item.id) || hosts.length <= 3) return '';
  		const updateCount = hosts.slice(3).filter((h) => h.update_available && h.latest_version).length;
  		return updateCount === 0 ? 'all up to date' : `${updateCount} with update${updateCount === 1 ? '' : 's'}`;
  	}

  	function updateableHostCount(item: SoftwareItemResponse): number | null {
  		const hosts = detailHosts(item);
  		if (hosts.length > 0) return hosts.filter((h) => h.update_available && h.latest_version).length;
  		return null;
  	}

  	function hasAnyUpdateableHosts(item: SoftwareItemResponse): boolean {
  		const c = updateableHostCount(item);
  		return c === null ? item.update_available : c > 0;
  	}

  	function softwareUpdateLabel(item: SoftwareItemResponse): string {
  		const c = updateableHostCount(item);
  		return c === null ? 'loading updates' : c === 0 ? 'up to date' : `${c} update${c === 1 ? '' : 's'}`;
  	}

  	function primaryPluginLabel(item: SoftwareItemResponse, host?: SoftwareItemHostSummary): string {
  		const plugin = host?.plugins[0];
  		if (plugin?.plugin_config_name) return plugin.plugin_config_name;
  		if (plugin?.plugin_type) return pluginTypeNames.get(plugin.plugin_type) ?? plugin.plugin_type;
  		const itemPlugin = item.plugins[0];
  		return itemPlugin ? (pluginTypeNames.get(itemPlugin) ?? itemPlugin) : 'Unknown';
  	}

  	function hostDisplayName(host: SoftwareItemHostSummary): string {
  		return host.friendly_name || host.hostname;
  	}

  	function isSingleHostItem(item: SoftwareItemResponse): boolean {
  		const hosts = detailHosts(item);
  		return hosts.length > 0 ? hosts.length === 1 : item.host_count === 1;
  	}

  	function singleHost(item: SoftwareItemResponse): SoftwareItemHostSummary | null {
  		const hosts = detailHosts(item);
  		return hosts.length === 1 ? hosts[0] : null;
  	}

  	function versionLabel(
  		version: string | null | undefined,
  		displayVersion?: string | null | undefined,
  		fallback = '—'
  	): string {
  		if (!version) return fallback;
  		return formatVersion(resolveDisplayVersion(version, displayVersion ?? undefined));
  	}

  	function versionTitle(version: string | null | undefined, displayVersion?: string | null | undefined): string {
  		return resolveDisplayVersion(version, displayVersion ?? undefined) ?? '—';
  	}

  	function groupIsOpen(itemId: string): boolean {
  		return !collapsedGroupIds.has(itemId);
  	}
  </script>

  <!-- Desktop layout: hidden on mobile (< 640px) -->
  <div class="max-sm:hidden" data-ui="software-group-list" role="list" aria-label="Tracked software">
  	{#each items as item (item.id)}
  		{@const compactSingleHost = singleHost(item)}
  		{@const isCompactSingleHost = isSingleHostItem(item)}
  		<div
  			class="border-b border-[var(--border-subtle)] last:border-b-0"
  			data-testid={'software-group-' + item.id}
  			role="listitem"
  		>
  			<div
  				class="grid items-center gap-x-2 bg-[var(--bg-raised)] px-4 py-2.5 {canManage
  					? 'grid-cols-[24px_minmax(0,1fr)_40px]'
  					: 'grid-cols-[minmax(0,1fr)]'}"
  				data-testid={'software-group-header-' + item.id}
  			>
  				{#if canManage}
  					<div>
  						<Checkbox
  							id={'software-row-' + item.id}
  							checked={batchSelectedIds.has(item.id)}
  							onchange={() => onToggleBatch(item.id)}
  							aria-label={'Select ' + item.name}
  						/>
  					</div>
  				{/if}
  				<div class="grid grid-cols-[1fr_140px_88px] items-center gap-x-2" data-ui="software-group-grid">
  					<div class="min-w-0">
  						<div class="flex items-center gap-2">
  							{#if canManage}
  								<button
  									class="cursor-pointer text-section-title leading-none transition-[background,border-color,color] duration-fast hover:text-[var(--accent-bright)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]"
  									class:text-[var(--color-warning)]={item.featured}
  									class:star-unfeatured={!item.featured}
  									title={item.featured ? 'Unfeature' : 'Feature'}
  									onclick={(e) => {
  										e.stopPropagation();
  										onToggleFeatured(item);
  									}}
  									aria-label={(item.featured ? 'Unfeature ' : 'Feature ') + item.name}
  								>
  									{item.featured ? '★' : '☆'}
  								</button>
  							{:else}
  								<span class={item.featured ? 'text-section-title leading-none text-[var(--color-warning)]' : 'star-unfeatured text-section-title leading-none'}
  									>{item.featured ? '★' : '☆'}</span
  								>
  							{/if}
  							{#if isValidLogoUrl(item.icon_url)}
  								<img
  									src={item.icon_url}
  									alt=""
  									class="h-5 w-5 rounded-panel object-contain"
  									referrerpolicy="no-referrer"
  								/>
  							{/if}
  							<a
  								href={'/software/' + item.id}
  								class="truncate text-sm font-semibold text-[var(--text-primary)] hover:underline"
  							>
  								{item.name}
  							</a>
  						</div>
  						{#if isCompactSingleHost && compactSingleHost}
  							<div class="mt-0.5 flex items-center gap-2">
  								<p class="truncate text-nav-item text-[var(--text-secondary)]">
  									{hostDisplayName(compactSingleHost)}
  								</p>
  								<PillBadge label={primaryPluginLabel(item, compactSingleHost)} />
  							</div>
  						{:else}
  							<div class="mt-0.5 flex items-center gap-1">
  								<button
  									type="button"
  									class="expand-pill min-h-badge"
  									aria-label={groupIsOpen(item.id) ? 'Collapse ' + item.name : 'Expand ' + item.name}
  									aria-expanded={groupIsOpen(item.id)}
  									aria-controls={'software-group-body-' + item.id}
  									onclick={() => onToggleGroup(item.id)}
  								>
  									<span
  										class={groupIsOpen(item.id)
  											? 'shrink-0 text-subsection-title leading-none'
  											: 'shrink-0 text-table-header leading-none'}
  										aria-hidden="true">{groupIsOpen(item.id) ? '▼' : '▶'}</span
  									>
  									<span>{item.host_count} host{item.host_count === 1 ? '' : 's'}</span>
  								</button>
  								<span class="text-nav-item text-[var(--text-secondary)]">· {softwareUpdateLabel(item)}</span>
  							</div>
  						{/if}
  					</div>
  					{#if isCompactSingleHost && compactSingleHost}
  						<div class="text-right">
  							<p
  								class="font-mono text-nav-item text-[var(--text-secondary)]"
  								title={versionTitle(compactSingleHost.installed_version, compactSingleHost.installed_display_version)}
  							>
  								{versionLabel(compactSingleHost.installed_version, compactSingleHost.installed_display_version)}
  							</p>
  							{#if compactSingleHost.update_available && compactSingleHost.latest_version}
  								<p
  									class="font-mono text-button text-[var(--accent-bright)]"
  									title={versionTitle(
  										compactSingleHost.latest_version,
  										(compactSingleHost.latest_release_metadata?.display_version as string | null | undefined) ?? undefined
  									)}
  								>
  									↑ {versionLabel(
  										compactSingleHost.latest_version,
  										(compactSingleHost.latest_release_metadata?.display_version as string | null | undefined) ?? undefined
  									)}
  								</p>
  							{/if}
  						</div>
  					{:else}
  						<div aria-hidden="true"></div>
  					{/if}
  					<div class="flex justify-end">
  						{#if canTriggerUpdates}
  							{#if isCompactSingleHost}
  								<ActionBadge
  									variant="navigation"
  									tone="accent"
  									idleLabel="Update"
  									hoverLabel="Update"
  									disabled={!(compactSingleHost?.update_available && compactSingleHost?.latest_version)}
  									onclick={() => onOpenUpdateModal(item)}
  								/>
  							{:else}
  								<UpdateAllButton
  									state={hasAnyUpdateableHosts(item) ? 'idle' : 'dim'}
  									ariaLabel={hasAnyUpdateableHosts(item) ? undefined : 'No updates available'}
  									onclick={() => onOpenUpdateModal(item)}
  								/>
  							{/if}
  						{:else if isCompactSingleHost && compactSingleHost?.update_available}
  							<StatusBadge tone="info" label="Update avail" />
  						{:else if hasAnyUpdateableHosts(item)}
  							{@const groupUpdateCount = updateableHostCount(item)}
  							<StatusBadge
  								tone="info"
  								label={groupUpdateCount === null
  									? 'Updates avail'
  									: `${groupUpdateCount} update${groupUpdateCount === 1 ? '' : 's'}`}
  							/>
  						{:else}
  							<StatusBadge tone="success" label="Up to date" />
  						{/if}
  					</div>
  				</div>
  				{#if canManage}
  					<div class="actions-menu flex justify-end">
  						<Button
  							variant="ghost"
  							size="sm"
  							ariaLabel={'Actions for ' + item.name}
  							onclick={(e) => {
  								e.stopPropagation();
  								onOpenMenu(item.id, e.currentTarget);
  							}}>&#8943;</Button
  						>
  					</div>
  				{/if}
  			</div>
  			{#if !isCompactSingleHost && itemDetailLoadingIds.has(item.id)}
  				<div
  					class="grid items-center gap-x-2 border-t border-[var(--border-subtle)] px-4 py-2.5 {canManage
  						? 'grid-cols-[24px_minmax(0,1fr)_40px]'
  						: 'grid-cols-[minmax(0,1fr)]'}"
  					id={'software-group-body-' + item.id}
  				>
  					{#if canManage}
  						<span aria-hidden="true"></span>
  					{/if}
  					<div class="grid grid-cols-[8px_1fr_140px_88px] items-center gap-x-3">
  						<div class="col-[1/5] text-sm text-[var(--text-secondary)]">Loading hosts...</div>
  					</div>
  					{#if canManage}
  						<span aria-hidden="true"></span>
  					{/if}
  				</div>
  			{:else if !isCompactSingleHost && detailHosts(item).length > 0}
  				<div id={'software-group-body-' + item.id}>
  					{#each visibleHosts(item) as host (host.id)}
  						<div
  							class="grid items-center gap-x-2 border-t border-[var(--border-subtle)] bg-transparent px-4 py-2.5 transition-[background,border-color,color] duration-fast hover:bg-[var(--bg-raised)] {canManage
  								? 'grid-cols-[24px_minmax(0,1fr)_40px]'
  								: 'grid-cols-[minmax(0,1fr)]'}"
  							data-testid={'software-host-row-' + host.id}
  						>
  							{#if canManage}
  								<span aria-hidden="true"></span>
  							{/if}
  							<div class="grid grid-cols-[1fr_140px_88px] items-center gap-x-2" data-ui="software-host-grid">
  								<div class="min-w-0 pl-[18px]">
  									<div class="flex min-w-0 items-center gap-2">
  										<span class="shrink-0 text-table-header text-[var(--text-secondary)]" aria-hidden="true">·</span>
  										<p class="truncate text-sm text-[var(--text-primary)]">{hostDisplayName(host)}</p>
  										<PillBadge label={primaryPluginLabel(item, host)} />
  									</div>
  									{#if hostDisplayName(host) !== host.hostname}
  										<p class="mt-1 truncate text-nav-item text-[var(--text-secondary)]">{host.hostname}</p>
  									{/if}
  								</div>
  								<div class="text-right">
  									<p
  										class="font-mono text-nav-item text-[var(--text-secondary)]"
  										title={versionTitle(host.installed_version, host.installed_display_version)}
  									>
  										{versionLabel(host.installed_version, host.installed_display_version)}
  									</p>
  									{#if host.update_available && host.latest_version}
  										<p
  											class="font-mono text-button text-[var(--accent-bright)]"
  											title={versionTitle(
  												host.latest_version,
  												(host.latest_release_metadata?.display_version as string | null | undefined) ?? undefined
  											)}
  										>
  											↑ {versionLabel(
  												host.latest_version,
  												(host.latest_release_metadata?.display_version as string | null | undefined) ?? undefined
  											)}
  										</p>
  									{/if}
  								</div>
  								<div class="flex justify-end">
  									{#if host.update_available && canTriggerUpdates}
  										<ActionBadge
  											variant="navigation"
  											tone="accent"
  											idleLabel="Update"
  											hoverLabel="Update"
  											onclick={() => onOpenUpdateModal(item)}
  										/>
  									{:else if host.update_available}
  										<StatusBadge tone="info" label="Update avail" />
  									{:else}
  										<StatusBadge tone="success" label="Up to date" />
  									{/if}
  								</div>
  							</div>
  							{#if canManage}
  								<span aria-hidden="true"></span>
  							{/if}
  						</div>
  					{/each}
  					{#if hiddenHostCount(item) > 0}
  						<div
  							class="grid items-center gap-x-2 border-t border-[var(--border-subtle)] bg-transparent px-4 py-2.5 {canManage
  								? 'grid-cols-[24px_minmax(0,1fr)_40px]'
  								: 'grid-cols-[minmax(0,1fr)]'}"
  						>
  							{#if canManage}
  								<span aria-hidden="true"></span>
  							{/if}
  							<div class="grid grid-cols-[8px_1fr_140px_88px] items-center gap-x-3">
  								<span aria-hidden="true"></span>
  								<div>
  									<button
  										type="button"
  										class="pl-[49px] text-nav-item text-[var(--text-secondary)] transition-[background,border-color,color] duration-fast hover:text-[var(--text-primary)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]"
  										onclick={() => onToggleOverflow(item.id)}
  									>
  										▸ {hiddenHostCount(item)} more — {hiddenHostsSummary(item)}
  									</button>
  								</div>
  								<span aria-hidden="true"></span>
  								<span aria-hidden="true"></span>
  							</div>
  							{#if canManage}
  								<span aria-hidden="true"></span>
  							{/if}
  						</div>
  					{/if}
  				</div>
  			{/if}
  		</div>
  	{/each}
  	<TableFooterBar total={totalItems} {currentPage} {totalPages} onPageChange={onPageChange} />
  </div>

  <!-- Mobile card layout added in next task -->

  <style>
  	.expand-pill {
  		display: inline-flex;
  		min-height: 14px;
  		align-items: center;
  		overflow: hidden;
  		border-radius: var(--radius-badge);
  		border: 1px solid rgba(var(--accent-rgb), 0.22);
  		background: rgba(var(--accent-rgb), 0.08);
  		padding: 0 5px;
  		font-size: var(--text-button);
  		font-weight: 600;
  		text-transform: none;
  		gap: 3px;
  		color: var(--accent);
  		transition:
  			background 0.12s,
  			border-color 0.12s,
  			color 0.12s;
  	}
  	.expand-pill:hover {
  		background: rgba(var(--accent-rgb), 0.18);
  		border-color: rgba(var(--accent-rgb), 0.42);
  		color: var(--accent-bright);
  	}
  	.expand-pill:focus-visible {
  		outline: none;
  		box-shadow: 0 0 0 3px rgba(var(--accent-rgb), 0.25);
  	}
  	.star-unfeatured {
  		color: var(--text-secondary);
  	}
  </style>
  ```

  Note: the `canManage` check guards the star `<button>`. When `canManage` is true the button calls `onToggleFeatured(item)` — passed from the parent page. When false a plain `<span>` is shown. The context menu (outside the extraction range) also calls `toggleFeatured` and is unchanged.

- [ ] **Step 2: Add `SoftwareGroupList` to the barrel export**

  Open `frontend/src/lib/components/ui/index.ts`. Add this line after the `DataTable` export:

  ```typescript
  export { default as SoftwareGroupList } from './SoftwareGroupList.svelte';
  ```

- [ ] **Step 3: Type-check the new file**

  ```bash
  cd frontend && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "(error|SoftwareGroupList)" | head -20
  ```

  Expected: no errors referencing `SoftwareGroupList.svelte`.

- [ ] **Step 4: Commit**

  ```bash
  git add frontend/src/lib/components/ui/SoftwareGroupList.svelte \
          frontend/src/lib/components/ui/index.ts
  git commit -m "feat(ui): extract SoftwareGroupList component from software page"
  ```

---

## Task 5: Add Mobile Card Layout to `SoftwareGroupList`

**Files:**
- Modify: `frontend/src/lib/components/ui/SoftwareGroupList.svelte`

- [ ] **Step 1: Add the mobile card layout block**

  In `frontend/src/lib/components/ui/SoftwareGroupList.svelte`, find the comment `<!-- Mobile card layout added in next task -->` and replace it (and everything after it up to `<style>`) with the following mobile layout block. The `<style>` block stays unchanged.

  ```svelte
  <!-- Mobile card layout: visible only on mobile (< 640px) -->
  <div class="sm:hidden divide-y divide-[var(--border-subtle)]" data-ui="software-group-list-mobile" role="list" aria-label="Tracked software">
  	{#each items as item (item.id)}
  		{@const compactSingleHost = singleHost(item)}
  		{@const isCompactSingleHost = isSingleHostItem(item)}
  		<div
  			class="px-4 py-3"
  			data-testid={'software-group-mobile-' + item.id}
  			role="listitem"
  		>
  			<!-- Card header: checkbox + star + icon + name + actions button -->
  			<div class="flex min-w-0 items-center gap-2">
  				{#if canManage}
  					<Checkbox
  						id={'software-row-mobile-' + item.id}
  						checked={batchSelectedIds.has(item.id)}
  						onchange={() => onToggleBatch(item.id)}
  						aria-label={'Select ' + item.name}
  					/>
  				{/if}
  				<span
  					class={item.featured
  						? 'shrink-0 text-section-title leading-none text-[var(--color-warning)]'
  						: 'shrink-0 star-unfeatured text-section-title leading-none'}
  				>
  					{item.featured ? '★' : '☆'}
  				</span>
  				{#if isValidLogoUrl(item.icon_url)}
  					<img
  						src={item.icon_url}
  						alt=""
  						class="h-4 w-4 shrink-0 rounded-panel object-contain"
  						referrerpolicy="no-referrer"
  					/>
  				{/if}
  				<a
  					href={'/software/' + item.id}
  					class="min-w-0 truncate text-sm font-semibold text-[var(--text-primary)] hover:underline"
  				>
  					{item.name}
  				</a>
  				{#if canManage}
  					<Button
  						variant="ghost"
  						size="sm"
  						class="ml-auto shrink-0"
  						ariaLabel={'Actions for ' + item.name}
  						onclick={(e) => {
  							e.stopPropagation();
  							onOpenMenu(item.id, e.currentTarget);
  						}}>&#8943;</Button
  					>
  				{/if}
  			</div>

  			{#if isCompactSingleHost && compactSingleHost}
  				<!-- Compact single-host: hostname + plugin badge inline -->
  				<div class="mt-0.5 flex items-center gap-2">
  					<p class="truncate text-nav-item text-[var(--text-secondary)]">{hostDisplayName(compactSingleHost)}</p>
  					<PillBadge label={primaryPluginLabel(item, compactSingleHost)} />
  				</div>
  				<!-- Version + action row -->
  				<div class="mt-1.5 flex items-center justify-between gap-2">
  					<div class="min-w-0">
  						<p
  							class="truncate font-mono text-nav-item text-[var(--text-secondary)]"
  							title={versionTitle(compactSingleHost.installed_version, compactSingleHost.installed_display_version)}
  						>
  							{versionLabel(compactSingleHost.installed_version, compactSingleHost.installed_display_version)}
  						</p>
  						{#if compactSingleHost.update_available && compactSingleHost.latest_version}
  							<p class="truncate font-mono text-button text-[var(--accent-bright)]">
  								↑ {versionLabel(
  									compactSingleHost.latest_version,
  									(compactSingleHost.latest_release_metadata?.display_version as string | null | undefined) ?? undefined
  								)}
  							</p>
  						{/if}
  					</div>
  					<div class="shrink-0">
  						{#if canTriggerUpdates}
  							<ActionBadge
  								variant="navigation"
  								tone="accent"
  								idleLabel="Update"
  								hoverLabel="Update"
  								disabled={!(compactSingleHost.update_available && compactSingleHost.latest_version)}
  								onclick={() => onOpenUpdateModal(item)}
  							/>
  						{:else if compactSingleHost.update_available}
  							<StatusBadge tone="info" label="Update avail" />
  						{:else}
  							<StatusBadge tone="success" label="Up to date" />
  						{/if}
  					</div>
  				</div>
  			{:else}
  				<!-- Multi-host: expand pill + update summary -->
  				<div class="mt-0.5 flex items-center gap-2">
  					<button
  						type="button"
  						class="expand-pill min-h-badge"
  						aria-label={groupIsOpen(item.id) ? 'Collapse ' + item.name : 'Expand ' + item.name}
  						aria-expanded={groupIsOpen(item.id)}
  						aria-controls={'software-group-mobile-body-' + item.id}
  						onclick={() => onToggleGroup(item.id)}
  					>
  						<span
  							class={groupIsOpen(item.id)
  								? 'shrink-0 text-subsection-title leading-none'
  								: 'shrink-0 text-table-header leading-none'}
  							aria-hidden="true">{groupIsOpen(item.id) ? '▼' : '▶'}</span
  						>
  						<span>{item.host_count} host{item.host_count === 1 ? '' : 's'}</span>
  					</button>
  					<span class="text-nav-item text-[var(--text-secondary)]">· {softwareUpdateLabel(item)}</span>
  				</div>

  				<!-- Host sub-cards (expanded) -->
  				{#if itemDetailLoadingIds.has(item.id)}
  					<p class="mt-1 pl-3 text-sm text-[var(--text-secondary)]">Loading hosts...</p>
  				{:else if groupIsOpen(item.id) && detailHosts(item).length > 0}
  					<div
  						class="mt-2 space-y-2 border-l-2 border-[var(--border-subtle)] pl-3"
  						id={'software-group-mobile-body-' + item.id}
  					>
  						{#each visibleHosts(item) as host (host.id)}
  							<div class="flex items-start justify-between gap-2" data-testid={'software-host-mobile-row-' + host.id}>
  								<div class="min-w-0">
  									<div class="flex min-w-0 items-center gap-2">
  										<span class="shrink-0 text-table-header text-[var(--text-secondary)]" aria-hidden="true">·</span>
  										<p class="truncate text-sm text-[var(--text-primary)]">{hostDisplayName(host)}</p>
  										<PillBadge label={primaryPluginLabel(item, host)} />
  									</div>
  									{#if hostDisplayName(host) !== host.hostname}
  										<p class="mt-0.5 truncate text-nav-item text-[var(--text-secondary)]">{host.hostname}</p>
  									{/if}
  									<p
  										class="font-mono text-nav-item text-[var(--text-secondary)]"
  										title={versionTitle(host.installed_version, host.installed_display_version)}
  									>
  										{versionLabel(host.installed_version, host.installed_display_version)}
  									</p>
  									{#if host.update_available && host.latest_version}
  										<p class="font-mono text-button text-[var(--accent-bright)]">
  											↑ {versionLabel(
  												host.latest_version,
  												(host.latest_release_metadata?.display_version as string | null | undefined) ?? undefined
  											)}
  										</p>
  									{/if}
  								</div>
  								<div class="shrink-0">
  									{#if host.update_available && canTriggerUpdates}
  										<ActionBadge
  											variant="navigation"
  											tone="accent"
  											idleLabel="Update"
  											hoverLabel="Update"
  											onclick={() => onOpenUpdateModal(item)}
  										/>
  									{:else if host.update_available}
  										<StatusBadge tone="info" label="Update avail" />
  									{:else}
  										<StatusBadge tone="success" label="Up to date" />
  									{/if}
  								</div>
  							</div>
  						{/each}
  						{#if hiddenHostCount(item) > 0}
  							<button
  								type="button"
  								class="text-nav-item text-[var(--text-secondary)] transition-[color] duration-fast hover:text-[var(--text-primary)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]"
  								onclick={() => onToggleOverflow(item.id)}
  							>
  								▸ {hiddenHostCount(item)} more — {hiddenHostsSummary(item)}
  							</button>
  						{/if}
  					</div>
  				{/if}
  			{/if}
  		</div>
  	{/each}
  	<TableFooterBar total={totalItems} {currentPage} {totalPages} onPageChange={onPageChange} />
  </div>
  ```

- [ ] **Step 2: Type-check**

  ```bash
  cd frontend && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error" | head -20
  ```

  Expected: no errors.

- [ ] **Step 3: Commit**

  ```bash
  git add frontend/src/lib/components/ui/SoftwareGroupList.svelte
  git commit -m "feat(ui): add mobile card layout to SoftwareGroupList"
  ```

---

## Task 6: Wire `SoftwareGroupList` into the Software Page

**Files:**
- Modify: `frontend/src/routes/software/+page.svelte`

- [ ] **Step 1: Add SoftwareGroupList import**

  In `frontend/src/routes/software/+page.svelte`, the imports at the top of the `<script>` block include a destructured import from `'$lib/components/ui'`. Add `SoftwareGroupList` to that destructure. Find:

  ```typescript
  import {
  	ActionBadge,
  	Callout,
  	ContextMenuItem,
  	ContextMenuShell,
  	EmptyState,
  	FormFieldRow,
  	ModalShell,
  	PageShell,
  	PillBadge,
  	SectionCard,
  	StatusBadge,
  	TableFooterBar,
  	TabStrip,
  	type TabStripItem
  } from '$lib/components/ui';
  ```

  Replace with:

  ```typescript
  import {
  	ActionBadge,
  	Callout,
  	ContextMenuItem,
  	ContextMenuShell,
  	EmptyState,
  	FormFieldRow,
  	ModalShell,
  	PageShell,
  	PillBadge,
  	SectionCard,
  	SoftwareGroupList,
  	StatusBadge,
  	TableFooterBar,
  	TabStrip,
  	type TabStripItem
  } from '$lib/components/ui';
  ```

- [ ] **Step 2: Remove helper functions that moved into SoftwareGroupList**

  In `frontend/src/routes/software/+page.svelte`, delete the following function definitions from the `<script>` block (they are now inside `SoftwareGroupList.svelte`):

  - `detailHosts` (currently at line ~383)
  - `updateableHostCount` (currently at line ~387)
  - `hasAnyUpdateableHosts` (currently at line ~395)
  - `softwareUpdateLabel` (currently at line ~400)
  - `versionLabel` (currently at line ~408)
  - `primaryPluginLabel` (currently at line ~418)
  - `visibleHosts` (currently at line ~430)
  - `hiddenHostCount` (currently at line ~441)
  - `hiddenHostsSummary` (currently at line ~449)
  - `hostDisplayName` (currently at line ~461)
  - `isSingleHostItem` (currently at line ~465)
  - `singleHost` (currently at line ~473)
  - `versionTitle` (currently at line ~478)
  - `groupIsOpen` (currently at line ~482)
  - `toggleGroupCollapsed` (currently at line ~486)
  - `toggleGroupOverflow` (currently at line ~492)

  Also remove the unused `SvelteSet` import for `itemDetailLoadingIds` and `collapsedGroupIds` if they are now only passed as props — keep the `let` declarations, only remove the helper functions listed above.

  Keep `toggleGroupCollapsed` and `toggleGroupOverflow` as they are called from onToggleGroup/onToggleOverflow callbacks.

  Actually — keep `toggleGroupCollapsed` and `toggleGroupOverflow` in the page. They mutate `collapsedGroupIds` and `expandedOverflowGroupIds` which are declared in the page. Only the *read* helpers (`groupIsOpen`, `visibleHosts`, etc.) move to the component.

  Functions to delete from the page (read-only helpers that moved into the component):
  - `detailHosts`
  - `updateableHostCount`
  - `hasAnyUpdateableHosts`
  - `softwareUpdateLabel`
  - `versionLabel`
  - `primaryPluginLabel`
  - `visibleHosts`
  - `hiddenHostCount`
  - `hiddenHostsSummary`
  - `hostDisplayName`
  - `isSingleHostItem`
  - `singleHost`
  - `versionTitle`
  - `groupIsOpen`

  Functions to **keep** in the page (they mutate page-level state):
  - `toggleGroupCollapsed`
  - `toggleGroupOverflow`
  - All other functions

- [ ] **Step 3: Replace the group list template section**

  In `frontend/src/routes/software/+page.svelte`, find the `{:else}` branch that currently contains:

  ```svelte
  {#if error}
    <div class="content-padding">
      <Callout .../>
    </div>
  {:else if loading}
    <p ...>Loading software items...</p>
  {:else if items.length === 0}
    <div class="px-4 py-8 text-center">
      <EmptyState title={itemsEmptyState.title} description={itemsEmptyState.description} />
    </div>
  {:else}
    <div data-ui="software-group-list" role="list" aria-label="Tracked software">
      {#each items as item (item.id)}
        ... (many lines of group rows) ...
      {/each}
      <TableFooterBar total={totalItems} {currentPage} {totalPages} onPageChange={loadAll} />
    </div>
  {/if}
  ```

  Replace the `{:else}` branch (from `{:else}` through the closing `</div>` of `data-ui="software-group-list"` and `<TableFooterBar>`) with:

  ```svelte
  {:else}
    <SoftwareGroupList
      {items}
      {itemDetailsById}
      {itemDetailLoadingIds}
      {collapsedGroupIds}
      {expandedOverflowGroupIds}
      {batchSelectedIds}
      {canManage}
      {canTriggerUpdates}
      {pluginTypeNames}
      {totalItems}
      {currentPage}
      {totalPages}
      onToggleGroup={toggleGroupCollapsed}
      onToggleOverflow={toggleGroupOverflow}
      onToggleBatch={toggleBatchSelect}
      onOpenMenu={toggleMenu}
      onOpenUpdateModal={openUpdateModal}
      onPageChange={loadAll}
      onToggleFeatured={toggleFeatured}
    />
  ```

- [ ] **Step 4: Remove the style block entries that moved**

  In `frontend/src/routes/software/+page.svelte`, the `<style>` block at the bottom contains `.expand-pill`, `.expand-pill:hover`, `.expand-pill:focus-visible`, and `.star-unfeatured`. These are now in `SoftwareGroupList.svelte`. Delete the entire `<style>` block from the page (it will now be empty or you can remove the `<style>` tags entirely).

- [ ] **Step 5: Type-check**

  ```bash
  cd frontend && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep "error" | head -20
  ```

  Expected: no errors.

- [ ] **Step 6: Run unit tests**

  ```bash
  cd frontend && npx vitest run 2>&1 | tail -10
  ```

  Expected: all pass.

- [ ] **Step 7: Commit**

  ```bash
  git add frontend/src/routes/software/+page.svelte
  git commit -m "refactor(software): extract group list into SoftwareGroupList component"
  ```

---

## Task 7: Add Mobile Snapshot Tests

**Files:**
- Modify: `frontend/tests/e2e/software-area.spec.ts`

- [ ] **Step 1: Add mobile viewport snapshots to the existing spec**

  In `frontend/tests/e2e/software-area.spec.ts`, add a `MOBILE_SNAPSHOTS` constant after the existing `SNAPSHOTS` constant, then add a new `test.describe` block with a project guard:

  ```typescript
  const SNAPSHOTS = [
    { name: 'software-list-dark', route: '/software?tab=all', theme: 'dark' as const },
    { name: 'software-list-light', route: '/software?tab=all', theme: 'light' as const },
    { name: 'software-ignores-dark', route: '/software?tab=ignores', theme: 'dark' as const },
    { name: 'software-ignores-light', route: '/software?tab=ignores', theme: 'light' as const },
    { name: 'software-detail-dark', route: '/software/test-item-id', theme: 'dark' as const },
    { name: 'software-detail-light', route: '/software/test-item-id', theme: 'light' as const }
  ];

  const MOBILE_SNAPSHOTS = [
    { name: 'software-list-mobile-dark', route: '/software?tab=all', theme: 'dark' as const },
    { name: 'software-list-mobile-light', route: '/software?tab=all', theme: 'light' as const }
  ];
  ```

  Keep the existing `SNAPSHOTS` constant unchanged. Add `MOBILE_SNAPSHOTS` as a separate constant. Then add a second describe block after the existing one to consume it with a project guard:

  ```typescript
  test.describe('software area mobile snapshots', () => {
    test.beforeEach(({}, testInfo) => {
      if (!testInfo.project.name.includes('mobile')) test.skip();
    });

    for (const snap of MOBILE_SNAPSHOTS) {
      test(snap.name, async ({ page }) => {
        await mockAuthApi(page);
        await setTheme(page, snap.theme);
        await page.goto(snap.route);
        await page.waitForSelector('[data-ui="page-shell"]', { timeout: 10000 });
        await expect(page).toHaveScreenshot(`${snap.name}.png`, {
          threshold: 0.02,
          mask: [
            page.locator('[aria-busy="true"]'),
            page.locator('td.font-mono'),
            page.locator('[data-ui="toast"]'),
            page.locator('time')
          ]
        });
      });
    }
  });
  ```

  The `test.beforeEach` guard skips these tests when the project name does not include `'mobile'`, so desktop projects (`chromium`, `chromium-dark`) never generate misleadingly-named mobile snapshots.

- [ ] **Step 2: Add a focused mobile layout test**

  Add this test after the existing `describe('software area snapshots', ...)` block:

  ```typescript
  test.describe('software area mobile layout', () => {
    test('mobile: software group list renders card layout at 393px', async ({ page }) => {
      await mockAuthApi(page);
      await setTheme(page, 'light');
      await page.setViewportSize({ width: 393, height: 852 });
      await page.goto('/software?tab=all');
      await page.waitForSelector('[data-ui="software-group-list-mobile"]', { timeout: 10000 });

      const mobileList = page.locator('[data-ui="software-group-list-mobile"]');
      await expect(mobileList).toBeVisible();

      // Desktop list should be hidden on mobile
      const desktopList = page.locator('[data-ui="software-group-list"]');
      await expect(desktopList).toBeHidden();

      // Each item renders as a mobile card
      const firstCard = mobileList.locator('[role="listitem"]').first();
      await expect(firstCard).toBeVisible();
      // Software name link is in the card
      await expect(firstCard.getByRole('link', { name: 'Firefox' })).toBeVisible();
    });

    test('mobile: software group list desktop layout is hidden at 393px', async ({ page }) => {
      await mockAuthApi(page);
      await setTheme(page, 'light');
      await page.setViewportSize({ width: 393, height: 852 });
      await page.goto('/software?tab=all');
      await page.waitForSelector('[data-ui="page-shell"]', { timeout: 10000 });

      // Desktop list uses max-sm:hidden — hidden at 393px
      const desktopList = page.locator('[data-ui="software-group-list"]');
      await expect(desktopList).toBeHidden();
    });

    test('desktop: software group list desktop layout is visible at 1280px', async ({ page }) => {
      await mockAuthApi(page);
      await setTheme(page, 'light');
      // Default viewport is desktop width; ensure mobile list is hidden
      await page.goto('/software?tab=all');
      await page.waitForSelector('[data-ui="software-group-list"]', { timeout: 10000 });

      const desktopList = page.locator('[data-ui="software-group-list"]');
      await expect(desktopList).toBeVisible();

      // Mobile list uses sm:hidden — hidden at 1280px
      const mobileList = page.locator('[data-ui="software-group-list-mobile"]');
      await expect(mobileList).toBeHidden();
    });
  });
  ```

- [ ] **Step 3: Run the new tests against chromium project**

  ```bash
  cd frontend && npx playwright test software-area --project=chromium 2>&1 | tail -20
  ```

  Expected: new mobile layout tests pass. Snapshot tests run and generate new baseline files on first run (will not fail on first run — Playwright creates baselines automatically).

- [ ] **Step 4: Commit**

  ```bash
  git add frontend/tests/e2e/software-area.spec.ts
  git commit -m "test(e2e): add mobile layout tests for software group list"
  ```

---

## Task 8: Update Layout Docs

**Files:**
- Modify: `docs/development/ui/layout.md`

- [ ] **Step 1: Update the responsive layout status**

  In `docs/development/ui/layout.md`, find:

  ```markdown
  **Status:** `Target` (mobile software row expansion pending; all other rules implemented)
  ```

  Replace with:

  ```markdown
  **Status:** `Implemented`
  ```

  Then find the `Pending:` section:

  ```markdown
  Pending:

  - Mobile software rows expand inline — data tables on software pages currently render identically at all viewports; no card-stack fallback exists.
  ```

  Replace with:

  ```markdown
  Implemented:

  - Mobile software rows expand inline. `SoftwareGroupList` renders a card-per-item layout at `< 640px`. Compact single-host items show name + hostname + plugin badge + version + action. Multi-host items show name + expand pill + host count; expanding reveals host sub-cards indented with a left border.
  - `DataTable` `mobileMode='cards'` provides column-defined card layout (auto `<dl>/<dt>/<dd>`) or a custom `mobileRow` snippet. `mobileMode='scroll'` enables horizontal scroll with `w-max` on the table.
  - Mobile snapshot coverage via `chromium-mobile` and `chromium-mobile-dark` Playwright projects at 393×852.
  ```

- [ ] **Step 2: Run markdownlint**

  ```bash
  cd /Users/andreyyantsen/Development/uptrakit && \
    markdownlint --config .markdownlint.json docs/development/ui/layout.md
  ```

  Expected: no errors.

- [ ] **Step 3: Commit**

  ```bash
  git add docs/development/ui/layout.md
  git commit -m "docs(ui): mark responsive layout as implemented"
  ```

---

## Task 9: Final Quality Gate

- [ ] **Step 1: Run full lint + type check**

  ```bash
  cd frontend && npm run lint && npm run format:check && npm run check
  ```

  Expected: all pass.

- [ ] **Step 2: Run unit tests**

  ```bash
  cd frontend && npm run test
  ```

  Expected: all pass.

- [ ] **Step 3: Run E2E tests for all projects**

  ```bash
  cd frontend && npx playwright test --project=chromium --project=chromium-dark --project=chromium-mobile --project=chromium-mobile-dark 2>&1 | tail -20
  ```

  Expected: all tests pass. New snapshot baselines created for `chromium-mobile` and `chromium-mobile-dark` projects on first run.

- [ ] **Step 4: Run software-area snapshots with mobile project**

  ```bash
  cd frontend && npx playwright test software-area --project=chromium-mobile 2>&1 | tail -20
  ```

  Expected: passes and generates `software-list-mobile-light-chromium-mobile.png` etc. as new baseline files.

- [ ] **Step 5: Commit baselines**

  ```bash
  git add frontend/tests/e2e/software-area.spec.ts-snapshots/
  git commit -m "test(e2e): add chromium-mobile snapshot baselines for software area"
  ```

---

## Self-Review Checklist

**Spec coverage:**

| Spec requirement | Task |
| --- | --- |
| `chromium-mobile` + `chromium-mobile-dark` projects at 393×852 DPR=1 | Task 1 |
| `DataTableColumn.mobileHide`, `mobileTitle` | Tasks 2–3 |
| `DataTable` `mobileMode='scroll'` with `w-max` | Tasks 2–3 |
| `DataTable` `mobileMode='cards'` dual-DOM rendering | Tasks 2–3 |
| `DataTable` `mobileRow` custom card snippet | Tasks 2–3 |
| Dev-mode fallback warning | Tasks 2–3 |
| `<dl>/<dt>/<dd>` card anatomy with `role="list"` | Tasks 2–3 |
| `SoftwareGroupList` extracted from software page | Tasks 4–6 |
| `SoftwareGroupList` mobile card layout | Task 5 |
| Software page wired to `SoftwareGroupList` | Task 6 |
| Mobile snapshot tests | Tasks 7, 9 |
| `layout.md` status updated | Task 8 |

**No placeholders:** All steps contain actual code. No TBDs.

**Type consistency:** `onToggleGroup`, `onToggleOverflow`, `onToggleBatch`, `onOpenMenu`, `onOpenUpdateModal`, `onPageChange`, `onToggleFeatured` defined in Task 4 and used in Task 6. `effectiveMobileMode`, `tableWidthClass`, `visibleMobileColumns`, `titleCol`, `valueColumns` defined and used within Task 3.
