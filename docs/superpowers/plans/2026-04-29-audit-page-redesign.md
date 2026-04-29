# Audit Page Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Redesign the audit-logs page to align with the design language, and apply the new
zebra+hover row style system-wide via DataTable.

**Architecture:** DataTable gains `<tbody>` child-selector classes that apply zebra striping and
hover highlight unconditionally. All existing per-`<tr>` `even:bg-*` classes are removed from row
snippets passed as the `row` prop. The audit page then gets filter panel, tab-strip, and actor
column updates.

**Tech Stack:** Svelte 5 (runes), Tailwind CSS v4, vitest + @testing-library/svelte

---

## File Map

**Modified:**

- `frontend/src/lib/components/ui/DataTable.svelte` — tbody child selectors, remove default
  `<tr>` `even:`, replace mobile-cards inline style
- `frontend/src/lib/components/ui/DataTable.test.ts` — update/add zebra+hover and mobile-cards tests
- `frontend/src/routes/audit-logs/+page.svelte` — filter panel, tab strip, actor column
- `frontend/src/routes/audit-logs/audit-logs.test.ts` — update label assertions, actor column test
- `docs/development/ui/primitives.md` — already updated in brainstorm session (no change needed)
- All 22 caller files below (sweep only — remove `even:bg-[var(--bg-raised)]` from `row` snippets):
  - `frontend/src/routes/services/+page.svelte:497`
  - `frontend/src/routes/hosts/[id]/+page.svelte:532`
  - `frontend/src/routes/hosts/[id]/+page.svelte:564`
  - `frontend/src/routes/hosts/[id]/+page.svelte:649`
  - `frontend/src/routes/hosts/[id]/+page.svelte:693`
  - `frontend/src/routes/hosts/+page.svelte:445` *(special: also remove `hover:bg-[var(--bg-raised)]`)*
  - `frontend/src/routes/host-tags/+page.svelte:377`
  - `frontend/src/routes/software/IgnoreRulesTab.svelte:192`
  - `frontend/src/routes/settings/SystemServicesSettings.svelte:356`
  - `frontend/src/routes/profile/+page.svelte:349`
  - `frontend/src/routes/software/[id]/+page.svelte:897`
  - `frontend/src/routes/settings/SchedulerTab.svelte:128`
  - `frontend/src/routes/settings/OidcProvidersSettings.svelte:230`
  - `frontend/src/routes/settings/EnrollmentTokenSettings.svelte:380`
  - `frontend/src/routes/system-services/+page.svelte:483`
  - `frontend/src/lib/components/surfaces/SurfaceTable.svelte:235` *(entityLinkRow — sweep only)*
  - `frontend/src/routes/settings/NotificationLogView.svelte:173`
  - `frontend/src/routes/settings/NotificationRulesSettings.svelte:194`
  - `frontend/src/routes/settings/PluginConfigsTab.svelte:809`
  - `frontend/src/routes/settings/PluginConfigsTab.svelte:992`
  - `frontend/src/routes/settings/PluginConfigsTab.svelte:1100`
  - `frontend/src/routes/+page.svelte:272`

---

## Task 1: DataTable — tbody row styles

**Files:**

- Modify: `frontend/src/lib/components/ui/DataTable.svelte:122-174`
- Modify: `frontend/src/lib/components/ui/DataTable.test.ts`

- [ ] **Step 1: Write failing tests for zebra+hover on tbody and mobile-cards container**

Add to `DataTable.test.ts` (inside `describe('DataTable')`):

```typescript
it('tbody carries zebra and hover child-selector classes', () => {
  const { container } = render(DataTable, {
    columns: [{ key: 'name', label: 'Name' }],
    rows: [{ name: 'alpha' }, { name: 'beta' }]
  });

  const tbody = container.querySelector('tbody');
  expect(tbody?.className).toContain('[&>tr:nth-child(even)]:bg-[var(--bg-raised)]');
  expect(tbody?.className).toContain('[&>tr:hover]:bg-[var(--bg-hover)]');
});

it('default auto-rendered tr does not carry even:bg-[var(--bg-raised)]', () => {
  const { container } = render(DataTable, {
    columns: [{ key: 'name', label: 'Name' }],
    rows: [{ name: 'alpha' }, { name: 'beta' }]
  });

  const trs = container.querySelectorAll('tbody tr');
  trs.forEach((tr) => {
    expect(tr.className).not.toContain('even:bg-[var(--bg-raised)]');
  });
});

it('auto-generated mobile cards wrapper carries zebra and hover child-selector classes', () => {
  const { container } = render(DataTable, {
    columns: [{ key: 'name', label: 'Name', mobileTitle: true }],
    rows: [{ name: 'alpha' }, { name: 'beta' }],
    mobileMode: 'cards'
  });

  const cardsEl = container.querySelector('[data-ui="data-table-cards"]');
  // wrapper div is the direct non-role child of the cards container
  const wrapper = cardsEl?.querySelector(':scope > div:not([role])');
  expect(wrapper?.className).toContain('[&>div:nth-child(even)]:bg-[var(--bg-raised)]');
  expect(wrapper?.className).toContain('[&>div:hover]:bg-[var(--bg-hover)]');
});
```

- [ ] **Step 2: Run tests to confirm failures**

```bash
cd frontend && npx vitest run src/lib/components/ui/DataTable.test.ts 2>&1 | tail -40
```

Expected: 3 new tests fail.

- [ ] **Step 3: Update DataTable.svelte**

At `DataTable.svelte:122`, change `<tbody>` to:

```html
<tbody class="[&>tr:nth-child(even)]:bg-[var(--bg-raised)] [&>tr:hover]:bg-[var(--bg-hover)]">
```

At `DataTable.svelte:127`, remove `even:bg-[var(--bg-raised)]` from the default `<tr>`:

```html
<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

At `DataTable.svelte:160-212`, replace the entire cards body (the `{#each}` block and its
contents, not the outer `{#if effectiveMobileMode === 'cards'}` guard or the footer block).

The `{#if mobileRow}` check must be lifted **above** the `{#each}` so the wrapper div only
wraps the auto-generated path. Caller-supplied `mobileRow` snippets are unaffected.

Replace from line 167 (`{#each rows as rowValue, index...`) through the closing `{/each}`
(line ~208) with:

```svelte
{#if mobileRow}
  {#each rows as rowValue, index (resolveRowKey(rowValue, index))}
    {@render mobileRow(rowValue)}
  {/each}
{:else}
  <div class="[&>div:nth-child(even)]:bg-[var(--bg-raised)] [&>div:hover]:bg-[var(--bg-hover)]">
    {#each rows as rowValue, index (resolveRowKey(rowValue, index))}
      <div
        role="listitem"
        class="px-4 py-3"
      >
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
    {/each}
  </div>
{/if}
```

Keep the existing `{#if footer}{@render footer()}{/if}` block after this — unchanged.

- [ ] **Step 4: Run tests to confirm passing**

```bash
cd frontend && npx vitest run src/lib/components/ui/DataTable.test.ts 2>&1 | tail -40
```

Expected: all tests pass including the 3 new ones.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/components/ui/DataTable.svelte \
        frontend/src/lib/components/ui/DataTable.test.ts
git commit -m "feat(ui): apply zebra+hover row styles via tbody child selectors in DataTable"
```

---

## Task 2: Caller sweep — remove even:bg from row snippets

**Files:** 22 caller files listed in the file map above.

**Scope rule:** Remove `even:bg-[var(--bg-raised)]` from `<tr>` elements inside a
`{#snippet row(...)}` block passed to DataTable. Do not touch `<tr>` elements outside such
snippets. Keep `last:border-b-0` and all other classes intact.

**Special cases:**

- `hosts/+page.svelte:445` — `<tr>` carries both `even:bg-[var(--bg-raised)]` and
  `hover:bg-[var(--bg-raised)]`. Remove both.
- `SurfaceTable.svelte:235` — sweep only the `entityLinkRow` snippet `<tr>`. Do not touch any
  other `<tr>` in the file.

- [ ] **Step 1: Remove `even:bg-[var(--bg-raised)]` from all 22 locations**

For every file below, apply the precise edit shown.

**`frontend/src/routes/services/+page.svelte:497`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/routes/hosts/[id]/+page.svelte:532`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/routes/hosts/[id]/+page.svelte:564`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/routes/hosts/[id]/+page.svelte:649`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/routes/hosts/[id]/+page.svelte:693`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/routes/hosts/+page.svelte:445` (special — remove both `even:` and `hover:`)**

```diff
-class="border-b border-[var(--border-subtle)] last:border-b-0 hover:bg-[var(--bg-raised)] even:bg-[var(--bg-raised)]"
+class="border-b border-[var(--border-subtle)] last:border-b-0"
```

**`frontend/src/routes/host-tags/+page.svelte:377`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/routes/software/IgnoreRulesTab.svelte:192`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/routes/settings/SystemServicesSettings.svelte:356`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 text-table-body even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0 text-table-body">
```

**`frontend/src/routes/profile/+page.svelte:349`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/routes/software/[id]/+page.svelte:897`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/routes/settings/SchedulerTab.svelte:128`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/routes/settings/OidcProvidersSettings.svelte:230`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/routes/settings/EnrollmentTokenSettings.svelte:380`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/routes/system-services/+page.svelte:483`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/lib/components/surfaces/SurfaceTable.svelte:235` (entityLinkRow snippet only)**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/routes/settings/NotificationLogView.svelte:173`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/routes/settings/NotificationRulesSettings.svelte:194`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/routes/settings/PluginConfigsTab.svelte:809`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/routes/settings/PluginConfigsTab.svelte:992`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/routes/settings/PluginConfigsTab.svelte:1100`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

**`frontend/src/routes/+page.svelte:272`**

```diff
-<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
+<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
```

- [ ] **Step 2: Verify no `even:bg-[var(--bg-raised)]` remains in row snippets**

```bash
cd frontend && grep -rn 'even:bg-\[var(--bg-raised)\]' src/ | grep -v 'DataTable.test.ts'
```

Expected: zero matches.

- [ ] **Step 3: Run the full frontend test suite to catch regressions**

```bash
cd frontend && npx vitest run 2>&1 | tail -60
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add \
  frontend/src/routes/services/+page.svelte \
  frontend/src/routes/hosts/+page.svelte \
  "frontend/src/routes/hosts/[id]/+page.svelte" \
  frontend/src/routes/host-tags/+page.svelte \
  frontend/src/routes/software/IgnoreRulesTab.svelte \
  "frontend/src/routes/software/[id]/+page.svelte" \
  frontend/src/routes/system-services/+page.svelte \
  frontend/src/routes/profile/+page.svelte \
  frontend/src/routes/+page.svelte \
  frontend/src/routes/settings/SystemServicesSettings.svelte \
  frontend/src/routes/settings/SchedulerTab.svelte \
  frontend/src/routes/settings/OidcProvidersSettings.svelte \
  frontend/src/routes/settings/EnrollmentTokenSettings.svelte \
  frontend/src/routes/settings/NotificationLogView.svelte \
  frontend/src/routes/settings/NotificationRulesSettings.svelte \
  frontend/src/routes/settings/PluginConfigsTab.svelte \
  frontend/src/lib/components/surfaces/SurfaceTable.svelte
git commit -m "refactor(ui): remove per-tr even:bg from DataTable row snippets — tbody now owns zebra"
```

---

## Task 3: Audit page — filter panel

**Files:**

- Modify: `frontend/src/routes/audit-logs/+page.svelte:12-22` (imports)
- Modify: `frontend/src/routes/audit-logs/+page.svelte:228-266` (filter grid)
- Modify: `frontend/src/routes/audit-logs/audit-logs.test.ts` (label assertions)

### Background

Current filter panel uses `FormFieldRow` inside a `grid-cols-3`. Replace with a compact
`grid grid-cols-1 gap-x-4 gap-y-3 sm:grid-cols-2 lg:grid-cols-4` where each cell is a
stacked `<label>` + input, no `FormFieldRow`.

Field order at `lg:grid-cols-4`: Action, Outcome, Actor Type, Target Type, Target ID, From, To.
7 fields = 4+3 layout (one empty cell). Labels: `text-xs font-medium text-[var(--text-secondary)] mb-1`.

Date labels: "From (RFC 3339)" → "From", "To (RFC 3339)" → "To".

Accessibility: each `<label for="...">` must be paired with the matching input. The existing IDs
(`filter-action-type`, `filter-outcome`, `filter-actor-type`, `filter-target-type`,
`filter-target-id`, `filter-from`, `filter-to`) are preserved — tests use `getByLabelText`.

- [ ] **Step 1: Update filter panel test for RFC 3339 label removal**

In `audit-logs.test.ts`, add:

```typescript
it('renders date filters with simplified From/To labels (no RFC 3339 text)', async () => {
  render(AuditLogsPage);
  await waitFor(() => expect(screen.getByText('Audit Logs')).toBeInTheDocument());
  expect(screen.getByLabelText('From')).toBeInTheDocument();
  expect(screen.getByLabelText('To')).toBeInTheDocument();
  expect(screen.queryByText(/RFC 3339/)).not.toBeInTheDocument();
});
```

- [ ] **Step 2: Run test to confirm it fails**

```bash
cd frontend && npx vitest run src/routes/audit-logs/audit-logs.test.ts 2>&1 | tail -20
```

Expected: new test fails (labels still say "From (RFC 3339)").

- [ ] **Step 3: Remove `FormFieldRow` import and update filter panel**

In `+page.svelte`, remove `FormFieldRow` from the `$lib/components/ui` import:

```typescript
import {
  Callout,
  DataTable,
  PageShell,
  SectionCard,
  StatusBadge,
  TableFooterBar,
  TabStrip,
  type TabStripItem
} from '$lib/components/ui';
```

Replace lines 228–266 (the filter `<div class="grid ...">` block) with:

```svelte
<div class="grid grid-cols-1 gap-x-4 gap-y-3 sm:grid-cols-2 lg:grid-cols-4">
  <div>
    <label for="filter-action-type"
      class="mb-1 block text-xs font-medium text-[var(--text-secondary)]">Action</label>
    <Input id="filter-action-type" type="text" placeholder="e.g. login" bind:value={filterActionType} />
  </div>

  <div>
    <label for="filter-outcome"
      class="mb-1 block text-xs font-medium text-[var(--text-secondary)]">Outcome</label>
    <select id="filter-outcome" class="select" bind:value={filterOutcome}>
      <option value="">All</option>
      {#each OUTCOME_TYPES as outcome (outcome)}
        <option value={outcome}>{outcomeLabel(outcome)}</option>
      {/each}
    </select>
  </div>

  <div>
    <label for="filter-actor-type"
      class="mb-1 block text-xs font-medium text-[var(--text-secondary)]">Actor Type</label>
    <select id="filter-actor-type" class="select" bind:value={filterActorType}>
      <option value="">All</option>
      {#each ACTOR_TYPES as t (t)}
        <option value={t}>{t}</option>
      {/each}
    </select>
  </div>

  <div>
    <label for="filter-target-type"
      class="mb-1 block text-xs font-medium text-[var(--text-secondary)]">Target Type</label>
    <Input id="filter-target-type" type="text" placeholder="e.g. software_item" bind:value={filterTargetType} />
  </div>

  <div>
    <label for="filter-target-id"
      class="mb-1 block text-xs font-medium text-[var(--text-secondary)]">Target ID</label>
    <Input id="filter-target-id" type="text" placeholder="Specific target id" bind:value={filterTargetId} />
  </div>

  <div>
    <label for="filter-from"
      class="mb-1 block text-xs font-medium text-[var(--text-secondary)]">From</label>
    <Input id="filter-from" type="datetime-local" bind:value={filterFrom} />
  </div>

  <div>
    <label for="filter-to"
      class="mb-1 block text-xs font-medium text-[var(--text-secondary)]">To</label>
    <Input id="filter-to" type="datetime-local" bind:value={filterTo} />
  </div>
</div>
```

- [ ] **Step 4: Run tests to confirm passing**

```bash
cd frontend && npx vitest run src/routes/audit-logs/audit-logs.test.ts 2>&1 | tail -20
```

Expected: all tests pass including the new RFC 3339 test.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/routes/audit-logs/+page.svelte \
        frontend/src/routes/audit-logs/audit-logs.test.ts
git commit -m "feat(audit-logs): replace FormFieldRow grid with compact label-above-input filter panel"
```

---

## Task 4: Audit page — tab strip and system-only card

**Files:**

- Modify: `frontend/src/routes/audit-logs/+page.svelte:206-220`
- No test changes needed (existing tablist test does not assert SectionCard wrapping)

### Background

When `hasBoth` is true: render `TabStrip` directly in `PageShell` body — remove the
`<SectionCard>` wrapper.

When system-only (`canViewSystem && !canViewTenant`): remove the `<SectionCard>` that shows
"Showing system-level audit logs." Render nothing.

Tenant-only (`canViewTenant && !canViewSystem`): no change — this branch renders nothing already.

- [ ] **Step 1: Replace lines 206–220 in +page.svelte**

Current block:

```svelte
{#if hasBoth}
  <SectionCard title="Log Scope" description="Switch between tenant and system audit streams.">
    <TabStrip
      items={SCOPE_TAB_ITEMS}
      activeId={activeTab}
      ariaLabel="Audit log scope"
      idBase="audit-logs"
      onSelect={(tab) => switchTab(tab as TabKey)}
    />
  </SectionCard>
{:else if canViewSystem}
  <SectionCard>
    <p class="text-sm text-[var(--text-muted)]">Showing system-level audit logs.</p>
  </SectionCard>
{/if}
```

Replace with:

```svelte
{#if hasBoth}
  <TabStrip
    items={SCOPE_TAB_ITEMS}
    activeId={activeTab}
    ariaLabel="Audit log scope"
    idBase="audit-logs"
    onSelect={(tab) => switchTab(tab as TabKey)}
  />
{/if}
```

- [ ] **Step 2: Run tests to confirm no regressions**

```bash
cd frontend && npx vitest run src/routes/audit-logs/audit-logs.test.ts 2>&1 | tail -20
```

Expected: all tests pass. The tablist test checks `getByRole('tablist', …)` — TabStrip still
renders the tablist element.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/routes/audit-logs/+page.svelte
git commit -m "feat(audit-logs): remove SectionCard wrapper from TabStrip and system-only info card"
```

---

## Task 5: Audit page — actor column with PillBadge

**Files:**

- Modify: `frontend/src/routes/audit-logs/+page.svelte` (imports, actor `<td>`, remove `actorLabel`)
- Modify: `frontend/src/routes/audit-logs/audit-logs.test.ts`

### Background

The current actor `<td>` renders `actorLabel(entry)`. Replace with two sub-elements:

1. `<PillBadge label={entry.actor_type} />` — always present
2. Enriched display name — `entry.actor_display` if set, else `entry.actor_id` if set, else nothing.

`<td>` layout: `flex items-center gap-2`. Title attribute:
`entry.actor_display ?? entry.actor_id ?? entry.actor_type`.

`actorLabel()` is only used in the actor `<td>` and `title` — both replaced here. Remove it.

- [ ] **Step 1: Write failing test for actor column structure**

Add to `audit-logs.test.ts` (inside `describe('Audit Logs Page')`):

```typescript
it('actor column shows PillBadge for actor_type and enriched display name', async () => {
  vi.mocked(api.listAuditLogs).mockResolvedValue(makePage([sampleEntry]));

  render(AuditLogsPage);
  await waitFor(() => expect(screen.getByRole('columnheader', { name: 'Actor' })).toBeInTheDocument());

  // sampleEntry.actor_type = 'user', actor_display = 'Audit Viewer'
  expect(screen.getByText('user')).toBeInTheDocument();
  expect(screen.getByText('Audit Viewer')).toBeInTheDocument();
});

it('actor column shows only PillBadge when actor_display and actor_id are absent', async () => {
  const systemEntry: AuditLogEntry = {
    ...sampleEntry,
    id: 'audit-sys',
    actor_type: 'system',
    actor_id: null,
    actor_display: null
  };
  vi.mocked(api.listAuditLogs).mockResolvedValue(makePage([systemEntry]));

  render(AuditLogsPage);
  await waitFor(() => expect(screen.getByRole('columnheader', { name: 'Actor' })).toBeInTheDocument());

  expect(screen.getByText('system')).toBeInTheDocument();
});
```

- [ ] **Step 2: Run tests to confirm failures**

```bash
cd frontend && npx vitest run src/routes/audit-logs/audit-logs.test.ts 2>&1 | tail -20
```

Expected: 2 new tests fail (actor column still uses actorLabel).

- [ ] **Step 3: Add PillBadge import and update actor column**

In the `$lib/components/ui` import (line 12–22), add `PillBadge`:

```typescript
import {
  Callout,
  DataTable,
  PageShell,
  PillBadge,
  SectionCard,
  StatusBadge,
  TableFooterBar,
  TabStrip,
  type TabStripItem
} from '$lib/components/ui';
```

Replace the actor `<td>` (currently lines 321–323):

```svelte
<td
  class="table-cell-pad"
  title={entry.actor_display ?? entry.actor_id ?? entry.actor_type}
>
  <div class="flex items-center gap-2">
    <PillBadge label={entry.actor_type} />
    {#if entry.actor_display}
      <span class="text-table-body text-[var(--text-primary)]">{entry.actor_display}</span>
    {:else if entry.actor_id}
      <span class="text-table-body text-[var(--text-primary)]">{entry.actor_id}</span>
    {/if}
  </div>
</td>
```

Remove the `actorLabel` function (lines 184–188):

```typescript
// DELETE this block:
function actorLabel(entry: AuditLogEntry): string {
  if (entry.actor_display) return entry.actor_display;
  if (entry.actor_id) return `${entry.actor_type}:${entry.actor_id}`;
  return entry.actor_type;
}
```

- [ ] **Step 4: Run tests to confirm passing**

```bash
cd frontend && npx vitest run src/routes/audit-logs/audit-logs.test.ts 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 5: Run full frontend checks**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test \
  && npm run build 2>&1 | tail -60
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/routes/audit-logs/+page.svelte \
        frontend/src/routes/audit-logs/audit-logs.test.ts
git commit -m "feat(audit-logs): replace actor column with PillBadge + enriched display name"
```
