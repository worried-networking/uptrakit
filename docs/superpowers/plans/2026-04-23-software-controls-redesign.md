# Software Controls Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consolidate the Software page's two floating control divs into a single card header,
aligning the page with the SectionCard pattern used on History and Hosts pages.

**Architecture:** Single-file template refactor in `+page.svelte`. No logic changes, no new
components. The outer floating `div.mb-4` (outside `{#if isItemsTab}`) and the standalone
select-all `div.flex.justify-end` (inside `{:else}`) are removed. Their contents move into a
new card header that wraps the entire list section. The card wrapper replaces the
`div.space-y-4` container and provides the visual surface.

**Tech Stack:** Svelte 5 (runes API, Snippets), Tailwind CSS v4, semantic CSS custom properties per `docs/development/ui/tokens.md`.

---

## File Map

| File | Change |
| ---- | ------ |
| `frontend/src/routes/software/+page.svelte` | Template-only refactor — remove two floating divs, add card wrapper + header, adjust state container padding, strip redundant border/bg from inner list div |

No other files change.

---

## Reference: Current Template Structure

Lines referenced below are from the file as it exists before this plan is applied.

```text
line 887–922  <div class="mb-4 flex items-center justify-end gap-2 flex-wrap">
                — floating controls div, OUTSIDE {#if isItemsTab}
                — children each guarded with their own {#if isItemsTab} checks

line 924      {#if isItemsTab}
line 925        <div class="space-y-4" data-ui="software-route-groups">
line 926          {#if error}   → Callout + retry
line 930          {:else if loading}   → loading <p>
line 932          {:else if items.length === 0}   → <EmptyState>
line 934          {:else}
line 935–944          {#if canManage} select-all div  {/if}
line 945–1252         <div data-ui="software-group-list" …overflow-hidden rounded-panel border …>
line 1252               <TableFooterBar …/>
line 1253             </div>
line 1254          {/if}
line 1255       </div>  ← closes space-y-4

line 1257–1405  BatchActionBar, ConfirmDialogs, ContextMenuShell,
                AssignToHostModal, SoftwareMergeWizard, AddSoftwareModal
                — all inside {#if isItemsTab}, outside the card

line 1406       {:else if activeTab === 'ignores'}   …
```

---

## Task 1: Remove the outer floating controls div

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte:887-922`

- [ ] **Step 1: Delete lines 887–922**

  Remove the entire block below. It sits between the closing `/>` of `<TabStrip>` and the opening `{#if isItemsTab}` on line 924.

  ```svelte
  			<div class="mb-4 flex items-center justify-end gap-2 flex-wrap">
  				{#if isItemsTab}
  					<label class="flex items-center gap-2 text-sm cursor-pointer select-none">
  						<Checkbox
  							id="software-filter-updatable-only"
  							bind:checked={showUpdatableOnly}
  							onchange={() => {
  								currentPage = 1;
  								loadAll(1);
  							}}
  						/>
  						Updates available
  					</label>
  				{/if}
  				{#if isItemsTab && pluginTypeOptions.length > 0}
  					<FormFieldRow label="Plugin">
  						<select
  							class="select text-sm"
  							bind:value={pluginTypeFilter}
  							onchange={() => {
  								currentPage = 1;
  								loadAll(1);
  							}}
  							aria-label="Filter by plugin"
  						>
  							<option value="">All plugins</option>
  							{#each pluginTypeOptions as opt (opt.plugin_type)}
  								<option value={opt.plugin_type}>{opt.display_name}</option>
  							{/each}
  						</select>
  					</FormFieldRow>
  				{/if}
  				{#if isItemsTab && canManage}
  					<Button variant="primary" size="sm" onclick={() => (showAddModal = true)}>Add Software</Button>
  				{/if}
  			</div>
  ```

  After deletion, `<TabStrip … />` is immediately followed by `{#if isItemsTab}`.

- [ ] **Step 2: Verify Svelte check passes**

  ```bash
  cd frontend && npm run check 2>&1 | tail -20
  ```

  Expected: `0 errors` (warnings are OK).

- [ ] **Step 3: Commit**

  ```bash
  git add frontend/src/routes/software/+page.svelte
  git commit -m "refactor(software): remove floating controls div outside isItemsTab"
  ```

---

## Task 2: Replace the space-y-4 wrapper with a card wrapper + header

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte:924-925` (opening wrapper line)

- [ ] **Step 1: Replace the opening `<div class="space-y-4">` with the card wrapper + header**

  Find this exact line (now line 925 after Task 1 removed 36 lines above it, but use content match):

  ```svelte
  				<div class="space-y-4" data-ui="software-route-groups">
  ```

  Replace it with:

  ```svelte
  				<div class="rounded-card border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm" data-ui="software-route-groups">
  					<header class="flex flex-col gap-3 border-b border-[var(--border-subtle)] card-padding md:flex-row md:items-center md:justify-between">
  						<div class="flex flex-wrap items-center gap-3">
  							{#if canManage}
  								<label class="flex cursor-pointer select-none items-center gap-2 text-sm">
  									<Checkbox
  										id="software-batch-select-all"
  										checked={allBatchPageSelected}
  										indeterminate={!allBatchPageSelected && batchSelectedIds.size > 0}
  										onchange={toggleBatchSelectAll}
  									/>
  									Select all
  								</label>
  								<span class="h-4 w-px bg-[var(--border-subtle)]" aria-hidden="true"></span>
  							{/if}
  							<label class="flex cursor-pointer select-none items-center gap-2 text-sm">
  								<Checkbox
  									id="software-filter-updatable-only"
  									bind:checked={showUpdatableOnly}
  									onchange={() => {
  										currentPage = 1;
  										loadAll(1);
  									}}
  								/>
  								Updates available
  							</label>
  							{#if pluginTypeOptions.length > 0}
  								<FormFieldRow label="Plugin">
  									<select
  										class="select text-sm"
  										bind:value={pluginTypeFilter}
  										onchange={() => {
  											currentPage = 1;
  											loadAll(1);
  										}}
  										aria-label="Filter by plugin"
  									>
  										<option value="">All plugins</option>
  										{#each pluginTypeOptions as opt (opt.plugin_type)}
  											<option value={opt.plugin_type}>{opt.display_name}</option>
  										{/each}
  									</select>
  								</FormFieldRow>
  							{/if}
  						</div>
  						{#if canManage}
  							<div class="shrink-0">
  								<Button variant="primary" size="sm" onclick={() => (showAddModal = true)}>Add Software</Button>
  							</div>
  						{/if}
  					</header>
  ```

- [ ] **Step 2: Verify Svelte check passes**

  ```bash
  cd frontend && npm run check 2>&1 | tail -20
  ```

  Expected: `0 errors`.

- [ ] **Step 3: Commit**

  ```bash
  git add frontend/src/routes/software/+page.svelte
  git commit -m "refactor(software): add card wrapper and consolidated header"
  ```

---

## Task 3: Update state containers and strip list border/bg

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte` — error/loading/empty blocks + list div + select-all div

### 3a — Error state: add `div.p-5` wrapper

- [ ] **Step 1: Wrap the error Callout in `div.p-5`**

  Find:

  ```svelte
  					{#if error}
  						<Callout tone="danger" title="Unable to load software items" message={error}>
  							<Button variant="primary" size="sm" class="mt-3" onclick={() => loadAll(currentPage)}>Retry</Button>
  						</Callout>
  ```

  Replace with:

  ```svelte
  					{#if error}
  						<div class="p-5">
  							<Callout tone="danger" title="Unable to load software items" message={error}>
  								<Button variant="primary" size="sm" class="mt-3" onclick={() => loadAll(currentPage)}>Retry</Button>
  							</Callout>
  						</div>
  ```

### 3b — Loading state: add `px-5`

- [ ] **Step 2: Add `px-5` to the loading paragraph**

  Find:

  ```svelte
  					{:else if loading}
  						<p class="py-8 text-center text-sm text-[var(--text-secondary)]">Loading software items...</p>
  ```

  Replace with:

  ```svelte
  					{:else if loading}
  						<p class="px-5 py-8 text-center text-sm text-[var(--text-secondary)]">Loading software items...</p>
  ```

### 3c — Empty state: wrap in `div.px-4.py-8.text-center`

- [ ] **Step 3: Wrap EmptyState**

  Find:

  ```svelte
  					{:else if items.length === 0}
  						<EmptyState title={itemsEmptyState.title} description={itemsEmptyState.description} />
  ```

  Replace with:

  ```svelte
  					{:else if items.length === 0}
  						<div class="px-4 py-8 text-center">
  							<EmptyState title={itemsEmptyState.title} description={itemsEmptyState.description} />
  						</div>
  ```

  > Note: the current `<EmptyState>` call has no border or bg classes — only the padding
  > wrapper is needed. The spec's mention of "remove border/bg" refers to non-existent
  > classes; ignore it.

### 3d — Remove the standalone select-all div

- [ ] **Step 4: Delete the `{#if canManage}` select-all wrapper div**

  Find and remove this entire block (it sits at the top of the `{:else}` branch, before the list div):

  ```svelte
  					{:else}
  						{#if canManage}
  							<div class="flex justify-end">
  								<Checkbox
  									id="software-batch-select-all"
  									checked={allBatchPageSelected}
  									indeterminate={!allBatchPageSelected && batchSelectedIds.size > 0}
  									onchange={toggleBatchSelectAll}
  								/>
  							</div>
  						{/if}
  						<div
  ```

  Replace with (keeping only `{:else}` and the opening of the list div):

  ```svelte
  					{:else}
  						<div
  ```

### 3e — Strip border/bg/overflow from the list container

- [ ] **Step 5: Remove visual classes from `data-ui="software-group-list"` div**

  Find:

  ```svelte
  						<div
  							class="overflow-hidden rounded-panel border border-[var(--border-subtle)] bg-[var(--bg-surface)]"
  							data-ui="software-group-list"
  							role="list"
  							aria-label="Tracked software"
  						>
  ```

  Replace with:

  ```svelte
  						<div
  							data-ui="software-group-list"
  							role="list"
  							aria-label="Tracked software"
  						>
  ```

- [ ] **Step 6: Verify Svelte check and lint pass**

  ```bash
  cd frontend && npm run check 2>&1 | tail -20 && npm run lint 2>&1 | tail -20
  ```

  Expected: `0 errors` for check. Lint: no new errors introduced.

- [ ] **Step 7: Commit**

  ```bash
  git add frontend/src/routes/software/+page.svelte
  git commit -m "refactor(software): consolidate state containers and strip redundant list border"
  ```

---

## Task 4: Verify final structure

**Files:**

- Read: `frontend/src/routes/software/+page.svelte`

- [ ] **Step 1: Confirm card wrapper uses correct radius**

  ```bash
  grep -n 'data-ui="software-route-groups"' frontend/src/routes/software/+page.svelte
  ```

  Expected: exactly one match on a line that contains `rounded-card` and does NOT contain
  `rounded-[3px]`, `rounded-2xl`, `rounded-lg`, `rounded-md`, or `rounded-xl`.

- [ ] **Step 2: Confirm no `space-y-4` wrapper remains**

  ```bash
  grep -n 'space-y-4' frontend/src/routes/software/+page.svelte
  ```

  Expected: no output (zero matches).

- [ ] **Step 3: Confirm no `mb-4 flex items-center justify-end gap-2 flex-wrap` remains**

  ```bash
  grep -n 'mb-4 flex items-center justify-end' frontend/src/routes/software/+page.svelte
  ```

  Expected: no output.

- [ ] **Step 4: Confirm `data-ui="software-group-list"` div has no `border` or `bg-` classes**

  ```bash
  grep -A4 'data-ui="software-group-list"' frontend/src/routes/software/+page.svelte
  ```

  Expected: no `border`, `bg-`, `rounded`, or `overflow-hidden` class on that div.

- [ ] **Step 5: Confirm select-all checkbox appears in header, not list body**

  ```bash
  grep -n 'software-batch-select-all' frontend/src/routes/software/+page.svelte
  ```

  Expected: exactly one match, inside the `<header>` block (lines above `{#if error}`).

- [ ] **Step 6: Run full frontend quality gate**

  ```bash
  cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
  ```

  Expected: all pass with no errors.
