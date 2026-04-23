<!-- markdownlint-disable MD013 -->

# Software Page Row Alignment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unify the Software page list so single-host and multi-host rows share the same `1fr 120px 88px` grid, replacing the misaligning 16px caret column with an inline expand/collapse pill in the sub-line.

**Architecture:** All changes live in one file — `frontend/src/routes/software/+page.svelte` — plus one test file update. The expand/collapse trigger moves from a standalone button in the first grid column into a pill `<button>` inside the sub-line beneath the software name. The 16px grid column and all associated spacer `<div>`/`<span>` elements are removed. A scoped `<style>` block is added for the unfeatured-star color.

**Tech Stack:** SvelteKit, Tailwind CSS (arbitrary values), Vitest + Testing Library for tests.

---

## File Map

| File | Change |
| ---- | ------ |
| `frontend/src/routes/software/+page.svelte` | All structural changes (grid, pill, star, skeleton) |
| `frontend/src/routes/software/software-trigger-status.test.ts` | Update assertions that match the old summary string |

---

### Task 1: Add `softwareUpdateLabel` helper and update test assertions

The existing test at line 211 asserts `getByText('4 hosts · 2 updates')` — a combined string that will no longer exist as a single element after the pill is added. Update the test first so the failure guides implementation.

**Files:**

- Modify: `frontend/src/routes/software/software-trigger-status.test.ts:211`
- Modify: `frontend/src/routes/software/+page.svelte:398–408`

- [ ] **Step 1: Locate the two assertions that need updating**

In `software-trigger-status.test.ts`, find these lines:

```ts
// line 211:
await waitFor(() => expect(screen.getByText('4 hosts · 2 updates')).toBeInTheDocument());
// line 255:
expect(screen.getByRole('button', { name: 'Collapse Demo App' })).toHaveAttribute('aria-expanded', 'true');
```

Line 211 expects the old combined summary string — it will fail after the pill splits the text. Line 255 asserts on the collapse button by aria-label, which is preserved in the new pill; it stays correct.

- [ ] **Step 2: Replace the combined-text assertion at line 211**

Open `frontend/src/routes/software/software-trigger-status.test.ts`. Replace line 211:

```ts
// OLD
await waitFor(() => expect(screen.getByText('4 hosts · 2 updates')).toBeInTheDocument());
// NEW — wait for the pill to appear, then check the trailing update label
await waitFor(() => expect(screen.getByRole('button', { name: 'Collapse Demo App' })).toBeInTheDocument());
expect(screen.getByText('· 2 updates')).toBeInTheDocument();
```

- [ ] **Step 3: Run the test suite to confirm this test now fails (as expected)**

```bash
cd frontend && npm run test -- software-trigger-status
```

Expected: the test fails because `getByText('· 2 updates')` finds nothing — the impl still renders the old combined `<p>`.

- [ ] **Step 4: Add `softwareUpdateLabel` helper inside `+page.svelte`**

Open `frontend/src/routes/software/+page.svelte`. After the `softwareSummary` function (around line 408), insert:

```ts
function softwareUpdateLabel(item: SoftwareItemResponse): string {
    const updateCount = updateableHostCount(item);
    return updateCount === null
        ? 'loading updates'
        : updateCount === 0
            ? 'up to date'
            : `${updateCount} update${updateCount === 1 ? '' : 's'}`;
}
```

Do **not** delete `softwareSummary` yet — it is still used at line 1045 and will be removed in Task 2.

- [ ] **Step 5: Run tests again — should still fail (pill template not added yet)**

```bash
cd frontend && npm run test -- software-trigger-status
```

Expected: still fails. The new helper exists but `· 2 updates` isn't rendered in the DOM yet.

- [ ] **Step 6: Commit the helper and test update**

```bash
cd frontend && cd ..
git add frontend/src/routes/software/+page.svelte \
        frontend/src/routes/software/software-trigger-status.test.ts
git commit -m "refactor(software): add softwareUpdateLabel helper, update test assertion for split pill layout"
```

---

### Task 2: Replace the old summary `<p>` with the expand/collapse pill

The multi-host sub-line currently renders `<p>{softwareSummary(item)}</p>`. Replace it with a pill `<button>` (glyph + host count) followed by a trailing `<span>` (update label). Simultaneously remove the standalone chevron `<button>` from the old first grid column.

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte:979–1046`

The region you are editing:

```svelte
<!-- BEFORE: lines ~979–1046 (abridged) -->
<div
  class={`grid items-center gap-x-3 ${
    isCompactSingleHost
      ? 'grid-cols-[minmax(0,1fr)_120px_88px]'
      : 'grid-cols-[16px_minmax(0,1fr)_120px_88px]'
  }`}
  data-ui="software-group-grid"
>
  {#if !isCompactSingleHost}
    <div>
      <button
        type="button"
        class="flex h-4 w-4 items-center justify-center rounded-[2px] text-[11px] text-[var(--text-secondary)] hover:bg-[var(--bg-surface)]"
        aria-label={groupIsOpen(item.id) ? 'Collapse ' + item.name : 'Expand ' + item.name}
        aria-expanded={groupIsOpen(item.id)}
        aria-controls={'software-group-body-' + item.id}
        onclick={() => toggleGroupCollapsed(item.id)}
      >
        {groupIsOpen(item.id) ? '▾' : '▸'}
      </button>
    </div>
  {/if}
  <div class="min-w-0">
    ... name-line ...
    {#if isCompactSingleHost && compactSingleHost}
      <div class="mt-0.5 flex items-center gap-2">... host + plugin pill ...</div>
    {:else}
      <p class="mt-0.5 text-[10px] text-[var(--text-secondary)]">{softwareSummary(item)}</p>
    {/if}
  </div>
  {#if isCompactSingleHost && compactSingleHost}
    ... version stack ...
  {:else}
    <div aria-hidden="true"></div>
  {/if}
  ...
```

- [ ] **Step 1: Unify the grid class — remove the conditional 4-col / 3-col split**

Find the `<div data-ui="software-group-grid">` block. Change:

```svelte
class={`grid items-center gap-x-3 ${
  isCompactSingleHost
    ? 'grid-cols-[minmax(0,1fr)_120px_88px]'
    : 'grid-cols-[16px_minmax(0,1fr)_120px_88px]'
}`}
```

to:

```svelte
class="grid grid-cols-[minmax(0,1fr)_120px_88px] items-center gap-x-3"
```

- [ ] **Step 2: Remove the `{#if !isCompactSingleHost}` chevron block**

Delete the entire `{#if !isCompactSingleHost} … {/if}` block that wraps the standalone chevron `<button>` (it was occupying the old 16px col 1). This block is roughly:

```svelte
{#if !isCompactSingleHost}
  <div>
    <button
      type="button"
      class="flex h-4 w-4 items-center justify-center rounded-[2px] text-[11px] text-[var(--text-secondary)] hover:bg-[var(--bg-surface)]"
      aria-label={groupIsOpen(item.id) ? 'Collapse ' + item.name : 'Expand ' + item.name}
      aria-expanded={groupIsOpen(item.id)}
      aria-controls={'software-group-body-' + item.id}
      onclick={() => toggleGroupCollapsed(item.id)}
    >
      {groupIsOpen(item.id) ? '▾' : '▸'}
    </button>
  </div>
{/if}
```

- [ ] **Step 3: Replace the old multi-host summary `<p>` with the expand pill + trailing text**

In the `<div class="min-w-0">` name cell, find the `{:else}` branch of `{#if isCompactSingleHost && compactSingleHost}`:

```svelte
{:else}
  <p class="mt-0.5 text-[10px] text-[var(--text-secondary)]">{softwareSummary(item)}</p>
{/if}
```

Replace it with:

```svelte
{:else}
  <div class="mt-0.5 flex items-center gap-1">
    <button
      type="button"
      class="inline-flex h-[14px] items-center overflow-hidden rounded-[2px] border bg-[rgba(var(--accent-rgb),.08)] px-[5px] text-[9px] font-semibold text-[var(--accent)] transition-[background,border-color,color] duration-[120ms] hover:bg-[rgba(var(--accent-rgb),.18)] hover:text-[var(--accent-bright)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]"
      style="border-color: rgba(var(--accent-rgb), .22);"
      aria-label={groupIsOpen(item.id) ? 'Collapse ' + item.name : 'Expand ' + item.name}
      aria-expanded={groupIsOpen(item.id)}
      aria-controls={'software-group-body-' + item.id}
      onclick={() => toggleGroupCollapsed(item.id)}
    >
      <span
        class={groupIsOpen(item.id) ? 'text-[13px] leading-none' : 'text-[11px] leading-none'}
        aria-hidden="true"
      >{groupIsOpen(item.id) ? '▼' : '▶'}</span>
      <span class="ml-[3px]">{item.host_count} host{item.host_count === 1 ? '' : 's'}</span>
    </button>
    <span class="text-[10px] text-[var(--text-secondary)]">· {softwareUpdateLabel(item)}</span>
  </div>
{/if}
```

Note: Tailwind cannot generate arbitrary `border-color` with opacity via `rgba()`, so the hover border color uses an inline `style` attribute for the idle state. The hover border uses the standard Tailwind arbitrary syntax with the accent-rgb token. Alternatively, use a CSS variable approach — but the inline style is explicit and correct here.

Actually, to avoid an inline style, use the `[border-color:...]` Tailwind escape:

```svelte
class="inline-flex h-[14px] items-center overflow-hidden rounded-[2px] border-[rgba(var(--accent-rgb),.22)] border bg-[rgba(var(--accent-rgb),.08)] px-[5px] text-[9px] font-semibold text-[var(--accent)] transition-[background,border-color,color] duration-[120ms] hover:bg-[rgba(var(--accent-rgb),.18)] hover:border-[rgba(var(--accent-rgb),.42)] hover:text-[var(--accent-bright)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]"
```

Remove the `style` attribute entirely when using this class form.

- [ ] **Step 4: Remove the now-unused `softwareSummary` function**

Delete the `softwareSummary` function from the script section (it is no longer called anywhere). It was:

```ts
function softwareSummary(item: SoftwareItemResponse): string {
    const hostLabel = `${item.host_count} host${item.host_count === 1 ? '' : 's'}`;
    const updateCount = updateableHostCount(item);
    const updateLabel =
        updateCount === null
            ? 'loading updates'
            : updateCount === 0
                ? 'up to date'
                : `${updateCount} update${updateCount === 1 ? '' : 's'}`;
    return `${hostLabel} · ${updateLabel}`;
}
```

- [ ] **Step 5: Run the test suite**

```bash
cd frontend && npm run test -- software-trigger-status
```

Expected: **PASS** — the `Collapse Demo App` pill button now has the correct aria-label and aria-expanded attribute; `· 2 updates` is now a standalone `<span>`.

- [ ] **Step 6: Run type check**

```bash
cd frontend && npm run check
```

Expected: no errors.

- [ ] **Step 7: Commit**

```bash
cd ..
git add frontend/src/routes/software/+page.svelte
git commit -m "feat(software): unify grid to 1fr 120px 88px, add expand/collapse pill to sub-line"
```

---

### Task 3: Restructure host sub-rows to match the unified grid

Host sub-rows currently use a `16px 1fr 120px 88px` inner grid with the `·` dot occupying the 16px first column. Remove that column; move the `·` inside the name cell with `padding-left: 18px`.

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte:1165–1218`

Current structure (abridged):

```svelte
<div
  class="grid grid-cols-[16px_minmax(0,1fr)_120px_88px] items-center gap-x-3"
  data-ui="software-host-grid"
>
  <span class="text-[11px] text-[var(--text-secondary)]" aria-hidden="true">·</span>
  <div class="min-w-0">
    <div class="flex items-center gap-2">
      <p class="truncate text-sm text-[var(--text-primary)]">{hostDisplayName(host)}</p>
      <PillBadge label={primaryPluginLabel(item, host)} />
    </div>
    {#if hostDisplayName(host) !== host.hostname}
      <p class="mt-1 truncate text-[10px] text-[var(--text-secondary)]">{host.hostname}</p>
    {/if}
  </div>
  <div class="text-right">... version stack ...</div>
  <div class="flex justify-end">... badge ...</div>
</div>
```

- [ ] **Step 1: Change the host sub-row grid and move the dot inside the name cell**

Replace the entire `<div data-ui="software-host-grid">` block with:

```svelte
<div
  class="grid grid-cols-[minmax(0,1fr)_120px_88px] items-center gap-x-3"
  data-ui="software-host-grid"
>
  <div class="min-w-0 pl-[18px]">
    <div class="flex items-center gap-2">
      <span class="text-[11px] text-[var(--text-secondary)]" aria-hidden="true">·</span>
      <p class="truncate text-sm text-[var(--text-primary)]">{hostDisplayName(host)}</p>
      <PillBadge label={primaryPluginLabel(item, host)} />
    </div>
    {#if hostDisplayName(host) !== host.hostname}
      <p class="mt-1 truncate text-[10px] text-[var(--text-secondary)]">{host.hostname}</p>
    {/if}
  </div>
  <div class="text-right">
    <p
      class="font-mono text-[10px] text-[var(--text-secondary)]"
      title={versionTitle(host.installed_version, host.installed_display_version)}
    >
      {versionLabel(host.installed_version, host.installed_display_version)}
    </p>
    {#if host.update_available && host.latest_version}
      <p
        class="font-mono text-[9px] text-[var(--accent-bright)]"
        title={versionTitle(
          host.latest_version,
          (host.latest_release_metadata?.display_version as string | null | undefined) ?? undefined
        )}
      >
        ↓ {versionLabel(
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
        idleLabel="Update Avail"
        hoverLabel="↑ Update"
        onclick={() => openUpdateModal(item)}
      />
    {:else if host.update_available}
      <StatusBadge tone="info" label="Update avail" />
    {:else}
      <StatusBadge tone="success" label="Up to date" />
    {/if}
  </div>
</div>
```

- [ ] **Step 2: Run tests**

```bash
cd frontend && npm run test -- software-trigger-status
```

Expected: PASS — tests check text content (`host-one`, `host-two`) which is unaffected.

- [ ] **Step 3: Run type check**

```bash
cd frontend && npm run check
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
cd ..
git add frontend/src/routes/software/+page.svelte
git commit -m "feat(software): restructure host sub-rows to unified 3-col grid, move dot inside name cell"
```

---

### Task 4: Fix the loading skeleton row

The loading skeleton row uses `grid-cols-[16px_minmax(0,1fr)_120px_88px]` and `col-[2/5]` for the "Loading hosts…" text. Update to the unified 3-col grid.

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte:1135–1152`

Current structure:

```svelte
{#if !isCompactSingleHost && itemDetailLoadingIds.has(item.id)}
  <div
    class={`grid items-center gap-x-3 border-t border-[var(--border-subtle)] px-4 py-3 ${
      canManage ? 'grid-cols-[24px_minmax(0,1fr)_40px]' : 'grid-cols-[minmax(0,1fr)]'
    }`}
    id={'software-group-body-' + item.id}
  >
    {#if canManage}
      <span aria-hidden="true"></span>
    {/if}
    <div class="grid grid-cols-[16px_minmax(0,1fr)_120px_88px] items-center gap-x-3">
      <span aria-hidden="true"></span>
      <div class="col-[2/5] text-sm text-[var(--text-secondary)]">Loading hosts...</div>
    </div>
    {#if canManage}
      <span aria-hidden="true"></span>
    {/if}
  </div>
{/if}
```

- [ ] **Step 1: Update the inner skeleton grid**

Replace:

```svelte
<div class="grid grid-cols-[16px_minmax(0,1fr)_120px_88px] items-center gap-x-3">
  <span aria-hidden="true"></span>
  <div class="col-[2/5] text-sm text-[var(--text-secondary)]">Loading hosts...</div>
</div>
```

With:

```svelte
<div class="grid grid-cols-[minmax(0,1fr)_120px_88px] items-center gap-x-3">
  <div class="col-[1/4] text-sm text-[var(--text-secondary)]">Loading hosts...</div>
</div>
```

The empty 16px spacer `<span>` is removed. `col-[2/5]` becomes `col-[1/4]` because the `1fr` name column is now col 1 (not col 2).

- [ ] **Step 2: Run tests and type check**

```bash
cd frontend && npm run test -- software-trigger-status && npm run check
```

Expected: PASS, no type errors.

- [ ] **Step 3: Commit**

```bash
cd ..
git add frontend/src/routes/software/+page.svelte
git commit -m "fix(software): update loading skeleton to unified 3-col grid"
```

---

### Task 5: Fix the `▸ N more` truncation row

The `▸ N more` row also uses `16px 1fr 120px 88px`. Update to `1fr 120px 88px` and adjust the button's `padding-left` from `21px` (within old col 2) to `49px` (within new col 1, preserving the same visual offset from the left edge of the row).

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte:1224–1251`

Current structure:

```svelte
{#if hiddenHostCount(item) > 0}
  <div
    class={`grid items-center gap-x-3 border-t border-[var(--border-subtle)] bg-transparent px-4 py-2 ${
      canManage ? 'grid-cols-[24px_minmax(0,1fr)_40px]' : 'grid-cols-[minmax(0,1fr)]'
    }`}
  >
    {#if canManage}
      <span aria-hidden="true"></span>
    {/if}
    <div class="grid grid-cols-[16px_minmax(0,1fr)_120px_88px] items-center gap-x-3">
      <span aria-hidden="true"></span>
      <div>
        <button
          type="button"
          class="pl-[21px] text-[10px] text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
          onclick={() => toggleGroupOverflow(item.id)}
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
```

- [ ] **Step 1: Update the truncation row inner grid**

Replace the inner `<div class="grid grid-cols-[16px_...]">` block:

```svelte
<div class="grid grid-cols-[minmax(0,1fr)_120px_88px] items-center gap-x-3">
  <div>
    <button
      type="button"
      class="pl-[49px] text-[10px] text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
      onclick={() => toggleGroupOverflow(item.id)}
    >
      ▸ {hiddenHostCount(item)} more — {hiddenHostsSummary(item)}
    </button>
  </div>
  <span aria-hidden="true"></span>
  <span aria-hidden="true"></span>
</div>
```

Why `49px`: the old `21px` was within col 2 (1fr), which started at `16px col + 12px gap = 28px` from the row edge, placing the text at `28 + 21 = 49px` from the row edge. The new grid has no col 1, so the name cell starts at the row edge — `pl-[49px]` places the text at the same `49px` visual position.

- [ ] **Step 2: Run tests**

```bash
cd frontend && npm run test -- software-trigger-status
```

Expected: PASS — tests use `getByRole('button', { name: '▸ 1 more — all up to date' })` which matches the text content unchanged.

- [ ] **Step 3: Run type check**

```bash
cd frontend && npm run check
```

- [ ] **Step 4: Commit**

```bash
cd ..
git add frontend/src/routes/software/+page.svelte
git commit -m "fix(software): update N-more truncation row to unified 3-col grid, adjust padding to 49px"
```

---

### Task 6: Update star-off color to scoped `.star-unfeatured` class

The unfeatured star `☆` currently uses `text-[var(--text-muted)]`. Replace with a scoped `.star-unfeatured` class that delivers the higher-contrast values specified in the spec.

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte` (script section stars + add `<style>` block)

- [ ] **Step 1: Add the `<style>` block at the end of the file**

Append after the last `{/if}` closing tag at the bottom of `+page.svelte`:

```svelte
<style>
  .star-unfeatured {
    color: #8496a8;
  }
  :global(.dark) .star-unfeatured {
    color: #78788a;
  }
</style>
```

- [ ] **Step 2: Update the canManage star button (interactive star)**

Find (around line 1005–1007):

```svelte
<button
  class="cursor-pointer text-lg leading-none transition-opacity hover:opacity-70"
  class:text-[var(--color-warning)]={item.featured}
  class:text-[var(--text-muted)]={!item.featured}
```

Change to:

```svelte
<button
  class="cursor-pointer text-lg leading-none transition-opacity hover:opacity-70"
  class:text-[var(--color-warning)]={item.featured}
  class:star-unfeatured={!item.featured}
```

- [ ] **Step 3: Update the view-only star span**

Find (around line 1018):

```svelte
<span class={item.featured ? 'text-[var(--color-warning)]' : 'text-[var(--text-muted)]'}>
```

Change to:

```svelte
<span class={item.featured ? 'text-[var(--color-warning)]' : 'star-unfeatured'}>
```

- [ ] **Step 4: Run the full test suite**

```bash
cd frontend && npm run test
```

Expected: PASS across all software tests.

- [ ] **Step 5: Run lint, format check, type check**

```bash
cd frontend && npm run lint && npm run format:check && npm run check
```

Expected: all pass. If `npm run lint` flags the `<style>` block or scoped class usage, investigate — Svelte scoped styles are standard and should not trigger lint errors.

- [ ] **Step 6: Commit**

```bash
cd ..
git add frontend/src/routes/software/+page.svelte
git commit -m "fix(software): replace text-muted star-off with scoped .star-unfeatured for higher contrast"
```

---

### Task 7: Final validation

Run the full frontend quality gate and confirm the build passes.

- [ ] **Step 1: Full test suite**

```bash
cd frontend && npm run test
```

Expected: all pass.

- [ ] **Step 2: Lint, format, type check, build**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run build
```

Expected: all pass, build succeeds with no warnings about unused CSS selectors or missing classes. (Svelte may warn about `.star-unfeatured` if it appears to be unused due to `class:star-unfeatured={}` conditional binding — if so, add `/* @vite-ignore */` or suppress via Svelte config. Alternatively, confirm Svelte's scoped CSS purger handles conditional class bindings correctly — it should.)

- [ ] **Step 3: Manual visual check**

Start the dev server and open the Software page in a browser. Verify:

1. Single-host and multi-host rows have names at the same x-position (flush left)
2. Multi-host rows show `▼ N hosts · up to date` pill in the sub-line when expanded
3. Multi-host rows show `▶ N hosts · 2 updates` pill in the sub-line when collapsed
4. Clicking the pill toggles expand/collapse
5. `▸ N more` button appears and expands correctly
6. Unfeatured star `☆` is visibly higher contrast than before (not washed out)
7. Featured star `★` is still amber/warning color
8. Host sub-rows are indented with the `·` dot inside the name cell

- [ ] **Step 4: Commit if any minor adjustments were made during visual check**

```bash
cd ..
git add frontend/src/routes/software/+page.svelte
git commit -m "fix(software): visual adjustments from manual review"
```

---

## Notes

**Tablet grid (§7):** The current codebase has no tablet-specific grid breakpoint classes in `+page.svelte` — the `90px` tablet column spec from §7 is not yet implemented. Implementation scope item 2 (`grid-cols-[16px_minmax(0,1fr)_90px_88px]` → `grid-cols-[minmax(0,1fr)_90px_88px]`) is a no-op in this implementation. It will be addressed when §7 tablet support lands.

**Pill hover border color:** Tailwind cannot express `rgba(var(--accent-rgb), .42)` as a `hover:border-*` utility with the CSS custom property pattern. Use `hover:border-[rgba(var(--accent-rgb),.42)]` — Tailwind supports `rgba()` in arbitrary values when the value is a literal string (not a computed property). If the build generates an incorrect class name, fall back to `style:hover:border-color="rgba(var(--accent-rgb), .42)"` using Svelte's style directive.

**`softwareSummary` removal:** The function is only called in one place (the multi-host summary `<p>` replaced in Task 2). Removing it eliminates dead code; no other route or component imports it.
