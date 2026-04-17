<!-- markdownlint-disable MD013 -->

# UI Design Language Page Spec Alignment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Align the built-in page layouts and route-level interactions with the approved UI design-language spec now that the shared foundation and shared visual contract are in place.

**Architecture:** Consume the corrected shared primitives rather than layering more route-local styling. Tackle the highest-drift route families first (`Software`, `Hosts`, `History`), then move through the remaining built-in pages so every route uses the same badges, menu items, tables, pagination footers, and shell metrics. End with parity and responsive verification that proves built-in and surface-backed UI still look native to one another after the page-level redesign.

**Tech Stack:** SvelteKit routes, Svelte 5 shared UI primitives, shared Surfaces runtime, Vitest, Testing Library, Playwright, Markdown docs for any new route-pattern guidance.

**Execution Context:** Run commands from the repository root. This plan assumes `2026-04-17-ui-design-language-shared-visual-alignment.md` has landed first. On a clean machine, run `cd frontend && npm ci && npx svelte-kit sync` once before Task 1.

---

## File Map

<!-- markdownlint-disable MD060 -->

| File                                                                                                                                                                                                                                     | Change                                                                                                                                         |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `frontend/src/routes/software/+page.svelte`                                                                                                                                                                                              | Replace the flat item table with the spec’s grouped software-header and host-subrow layout                                                     |
| `frontend/src/routes/software/software-trigger-status.test.ts`                                                                                                                                                                           | Extend modal/update tests to the grouped Software route                                                                                        |
| `frontend/src/routes/software/surface-tabs.test.ts`                                                                                                                                                                                      | Preserve built-in vs surface-tab parity on the redesigned Software route                                                                       |
| `frontend/src/routes/software/[id]/+page.svelte`                                                                                                                                                                                         | Align host table badges, context menu items, and update affordances with the shared primitives                                                 |
| `frontend/src/routes/software/[id]/software-detail*.test.ts`                                                                                                                                                                             | Keep update-trigger and detail-state behavior stable while badge/menu visuals change                                                           |
| `crates/shared/web-api-types/src/hosts.rs`                                                                                                                                                                                               | Extend the host summary contract with route-level software status fields required by the spec-aligned Hosts page                               |
| `crates/ui/web-api-queries/src/queries/hosts.rs`                                                                                                                                                                                         | Query and map the new host software summary fields through the frontend data layer                                                             |
| `frontend/src/lib/types.ts`                                                                                                                                                                                                              | Reflect the host software summary shape used by the redesigned Hosts route                                                                     |
| `frontend/src/routes/hosts/+page.svelte`                                                                                                                                                                                                 | Implement the spec’s navigable software-status badge and stat-card color mapping                                                               |
| `frontend/src/routes/hosts/hosts.test.ts`                                                                                                                                                                                                | Add assertions for navigable host-status badges and table footer alignment                                                                     |
| `frontend/src/routes/history/+page.svelte`                                                                                                                                                                                               | Replace the current expandable table treatment with the spec’s chronological feed and transitional terminal layout                             |
| `frontend/src/routes/history/history.test.ts`                                                                                                                                                                                            | Add feed-layout, badge-stack, date-grouping, and status-glyph assertions                                                                       |
| `frontend/src/routes/settings/+page.svelte` and settings leaf panels                                                                                                                                                                     | Tighten tab/body spacing and two-column form rhythm to match Section 5.4                                                                       |
| `frontend/src/routes/+page.svelte`                                                                                                                                                                                                       | Align dashboard stat-card color treatment and recent-history cards to the spec                                                                 |
| `frontend/src/routes/services/+page.svelte`, `frontend/src/routes/system-services/+page.svelte`, `frontend/src/routes/host-tags/+page.svelte`, `frontend/src/routes/audit-logs/+page.svelte`, `frontend/src/routes/profile/+page.svelte` | Migrate remaining route-local menu rows, pagination placement, footer alignment, and badge usage to the shared contract                        |
| `frontend/src/routes/login/+page.svelte`, `frontend/src/routes/register/+page.svelte`, `frontend/src/routes/device/+page.svelte`                                                                                                         | Either align auth/device shells with the shared page language or defer with an explicit rationale note in docs if still intentionally separate |
| `frontend/tests/e2e/ui-parity.test.ts`                                                                                                                                                                                                   | Extend parity coverage for redesigned Software, Hosts, History, and menu flows                                                                 |
| `frontend/tests/e2e/ui-parity-responsive.test.ts`                                                                                                                                                                                        | Refresh responsive baselines once page layouts are in spec                                                                                     |

<!-- markdownlint-enable MD060 -->

---

### Task 1: Redesign The Software Route To Match Section 5.1

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte`
- Modify: `frontend/src/routes/software/[id]/+page.svelte`
- Modify: `frontend/src/routes/software/software-trigger-status.test.ts`
- Modify: `frontend/src/routes/software/surface-tabs.test.ts`
- Modify: `frontend/src/routes/software/[id]/software-detail-update-trigger.test.ts`
- Modify: `frontend/src/routes/software/[id]/software-detail.test.ts`
- Modify: `frontend/tests/e2e/ui-parity.test.ts`

- [ ] **Step 1: Write the failing Software route regressions**

Extend `frontend/src/routes/software/software-trigger-status.test.ts` with route-structure assertions that reflect the spec:

```ts
await waitFor(() => expect(screen.getByText("Demo App")).toBeInTheDocument());
expect(screen.getByText("2 hosts · 1 update")).toBeInTheDocument();
expect(screen.getByText("1.0.0")).toBeInTheDocument();
expect(screen.getByText("↓ 1.1.0")).toBeInTheDocument();
expect(
  screen.getByRole("button", { name: "Update Avail" }),
).toBeInTheDocument();
```

Also extend `frontend/tests/e2e/ui-parity.test.ts` with a screenshot assertion for the grouped software row:

```ts
await expect(page.getByTestId("parity-software-group-row")).toHaveScreenshot(
  "ui-parity-software-group-row.png",
);
```

Run:

```bash
cd frontend && npm run test -- src/routes/software/software-trigger-status.test.ts src/routes/software/surface-tabs.test.ts
cd frontend && npm run test:e2e -- --grep "software group row"
```

Expected: FAIL because the current Software route still renders a flat item table with `Update Available` pills in the name cell.

- [ ] **Step 2: Implement the grouped Software page layout**

Refactor `frontend/src/routes/software/+page.svelte` to render the Section 5.1 structure instead of a generic table.

Target rendering shape:

```svelte
<div class="rounded-[4px] border border-[var(--border-subtle)] bg-[var(--bg-surface)]" data-ui="software-group-list">
    {#each groupedItems as group (group.item.id)}
        <div class="grid grid-cols-[16px_1fr_120px_88px] items-center bg-[var(--bg-raised)] px-3 py-2">
            <button class="contents text-left" onclick={() => toggleGroup(group.item.id)}>
            <span>{expandedGroups.has(group.item.id) ? '▾' : '▸'}</span>
            <div>
                <p class="text-[10px] font-semibold text-[var(--text-primary)]">{group.item.name}</p>
                <p class="text-[10px] text-[var(--text-secondary)]">{group.summary}</p>
            </div>
            <span />
            </button>
            {#if group.updateableHostCount > 0}
                <UpdateAllBadge idleLabel="↑ Update all" hoverLabel="↑ Update all" onclick={() => updateGroup(group.item.id)} />
            {:else}
                <UpdateAllBadge idleLabel="↑ Update all" hoverLabel="↑ Update all" disabled />
            {/if}
        </div>
    {/each}
</div>
```

Host subrows must:

- use the same `16px 1fr 120px 88px` grid
- show a plugin pill in column 2
- render the two-line version stack in column 3
- use `ClickableBadge` for `Update Avail` / `↑ Update`
- keep truncation logic for 4+ hosts with a `▸ N more` summary row

This task must introduce a dedicated `UpdateAllBadge` shared primitive or an equivalent dedicated variant rather than reusing the generic info-tone clickable badge. Its idle, hover, and disabled values must match spec Section 4.3 exactly.

Run:

```bash
cd frontend && npm run test -- src/routes/software/software-trigger-status.test.ts src/routes/software/surface-tabs.test.ts "src/routes/software/[id]/software-detail-update-trigger.test.ts" "src/routes/software/[id]/software-detail.test.ts"
```

Expected: PASS with the Software route behavior preserved and the structure matching the spec.

- [ ] **Step 3: Refresh desktop parity coverage for Software**

Update `frontend/tests/e2e/ui-parity.test.ts` and fixture data so the grouped Software route is captured in both built-in and surface-tab contexts.

Run:

```bash
cd frontend && npm run test:e2e -- --grep "software"
```

Expected: PASS with updated screenshots for the grouped layout and hover-swap badge behavior.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/software/+page.svelte "frontend/src/routes/software/[id]/+page.svelte" frontend/src/routes/software/software-trigger-status.test.ts frontend/src/routes/software/surface-tabs.test.ts "frontend/src/routes/software/[id]/software-detail-update-trigger.test.ts" "frontend/src/routes/software/[id]/software-detail.test.ts" frontend/tests/e2e/ui-parity.test.ts frontend/tests/e2e/ui-parity.test.ts-snapshots
git commit -m "refactor: align software page with design spec"
```

---

### Task 2: Align Hosts And History With Sections 5.2 And 5.3

**Files:**

- Modify: `crates/shared/web-api-types/src/hosts.rs`
- Modify: `crates/ui/web-api-queries/src/queries/hosts.rs`
- Modify: `frontend/src/lib/types.ts`
- Modify: `frontend/src/routes/hosts/+page.svelte`
- Modify: `frontend/src/routes/hosts/hosts.test.ts`
- Modify: `frontend/src/routes/hosts/[id]/host-detail.test.ts`
- Modify: `frontend/src/routes/history/+page.svelte`
- Modify: `frontend/src/routes/history/history.test.ts`
- Modify: `frontend/src/routes/history/history-trigger-status.test.ts`
- Modify: `frontend/tests/e2e/ui-parity.test.ts`

- [ ] **Step 1: Write the failing Hosts and History regressions**

Add host badge assertions in `frontend/src/routes/hosts/hosts.test.ts`:

```ts
const updatesBadge = screen.getByRole("button", { name: "2 updates" });
expect(updatesBadge).toHaveAttribute("data-ui", "clickable-badge");

const historyBadge = screen.getByRole("button", { name: "1 error" });
expect(historyBadge).toHaveAttribute("data-tone", "danger");
```

Add feed assertions in `frontend/src/routes/history/history.test.ts`:

```ts
expect(screen.getByText("nginx on prod-01")).toBeInTheDocument();
expect(screen.getByText("1.24.0 → 1.25.0")).toBeInTheDocument();
expect(screen.getByText("▶ view log")).toBeInTheDocument();
expect(screen.getByText("Today")).toBeInTheDocument();
expect(screen.getByText("✓")).toBeInTheDocument();
expect(screen.getByText("✕")).toBeInTheDocument();
expect(screen.getByText("↑")).toBeInTheDocument();
expect(screen.getByText("·")).toBeInTheDocument();
```

Run:

```bash
cd frontend && npm run test -- src/routes/hosts/hosts.test.ts src/routes/history/history.test.ts
```

Expected: FAIL because the Hosts route still shows agent-count badges and the History route is still a row-expansion table instead of the spec feed.

- [ ] **Step 2: Extend the host contract, then implement the Hosts route badge and stat-card treatment**

Before refactoring the route, extend the hosts API/query/type contract so the page receives a dedicated summary object instead of inventing client-only fields. The contract should carry the route-level counts and known/unknown state the spec needs, for example:

```ts
software_status: {
  known: boolean;
  update_count: number;
  error_count: number;
}
```

Update `crates/shared/web-api-types/src/hosts.rs`, `crates/ui/web-api-queries/src/queries/hosts.rs`, and `frontend/src/lib/types.ts` first, then refactor `frontend/src/routes/hosts/+page.svelte` so the software-status column uses `ClickableBadge` and the stat cards use the spec color mapping.

Target host-software cell pattern:

```svelte
{#if host.software_status.update_count > 0}
    <ClickableBadge tone="info" idleLabel={`${host.software_status.update_count} updates`} hoverLabel="→ Software" onclick={() => goto(`/software?host=${host.id}`)} />
{:else if host.software_status.error_count > 0}
    <ClickableBadge tone="danger" idleLabel={`${host.software_status.error_count} error`} hoverLabel="→ History" onclick={() => goto(`/history?host=${host.id}`)} />
{:else if host.software_status.known}
    <StatusBadge tone="success" label="Up to date" />
{:else}
    <StatusBadge tone="neutral" label="Unknown" />
{/if}
```

Run:

```bash
cd frontend && npm run test -- src/routes/hosts/hosts.test.ts "src/routes/hosts/[id]/host-detail.test.ts"
```

Expected: PASS with the host list using the spec’s navigable badge pattern.

- [ ] **Step 3: Replace the History table with the transitional feed layout**

Refactor `frontend/src/routes/history/+page.svelte` to a chronological feed with inline terminal expansion, keeping the current live-stream data flow intact.

Target item shell:

```svelte
<article class="grid grid-cols-[24px_1fr_auto] gap-3 rounded-[3px] border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-3">
    <div class={`flex h-6 w-6 items-center justify-center rounded-[3px] ${historyStatusGlyphClasses(item.status)}`} data-state={item.status}>
        {historyStatusGlyph(item.status)}
    </div>
    <div>
        <p class="text-[10px] text-[var(--text-primary)]">{item.software_item_name} on {item.host_name}</p>
        <p class="font-mono text-[10px] text-[var(--text-secondary)]">{formatVersion(item.from_version)} → <span class="text-[var(--accent-bright)]">{formatVersion(item.to_version)}</span></p>
    </div>
    <div class="flex flex-col items-end gap-1">
        <StatusBadge tone={statusBadgeTone(item.status)} label={statusLabel(item.status)} />
        <span class="text-[10px] text-[var(--text-secondary)]">{formatRelativeTime(item.started_at)}</span>
    </div>
</article>
```

Keep inline `TerminalOutput` and `Input Required` semantics, but render them inside the feed item body instead of a nested table row.

This step must also introduce explicit date-group separator rows or headings such as `Today`, `Yesterday`, or absolute calendar dates, because the spec requires chronological grouping by date rather than a flat feed.
It must also map each history state to an explicit icon-square treatment that matches the spec: success `✓`, failure `✕`, in-progress `↑`, and waiting/input-required `·`, each with state-specific background/border colors and matching test coverage.

Run:

```bash
cd frontend && npm run test -- src/routes/history/history.test.ts src/routes/history/history-trigger-status.test.ts src/lib/components/TerminalOutput.test.ts
```

Expected: PASS with the feed layout and transitional terminal styling aligned.

- [ ] **Step 4: Refresh parity coverage and commit**

Run:

```bash
cd frontend && npm run check
cd frontend && npm run test:e2e -- --grep "hosts|history"
```

Expected: PASS with updated screenshots for host badges and the history feed.

Commit:

```bash
git add crates/shared/web-api-types/src/hosts.rs crates/ui/web-api-queries/src/queries/hosts.rs frontend/src/lib/types.ts frontend/src/routes/hosts/+page.svelte frontend/src/routes/hosts/hosts.test.ts "frontend/src/routes/hosts/[id]/host-detail.test.ts" frontend/src/routes/history/+page.svelte frontend/src/routes/history/history.test.ts frontend/src/routes/history/history-trigger-status.test.ts frontend/tests/e2e/ui-parity.test.ts frontend/tests/e2e/ui-parity.test.ts-snapshots
git commit -m "refactor: align hosts and history pages with design spec"
```

---

### Task 3: Sweep The Remaining Built-In Pages Onto The Spec-Aligned Route Language

**Files:**

- Modify: `frontend/src/routes/settings/+page.svelte`
- Modify: `frontend/src/routes/settings/*.svelte`
- Modify: `frontend/src/routes/+page.svelte`
- Modify: `frontend/src/routes/services/+page.svelte`
- Modify: `frontend/src/routes/system-services/+page.svelte`
- Modify: `frontend/src/routes/host-tags/+page.svelte`
- Modify: `frontend/src/routes/audit-logs/+page.svelte`
- Modify: `frontend/src/routes/profile/+page.svelte`
- Modify or explicitly defer with rationale: `frontend/src/routes/login/+page.svelte`
- Modify or explicitly defer with rationale: `frontend/src/routes/register/+page.svelte`
- Modify or explicitly defer with rationale: `frontend/src/routes/device/+page.svelte`
- Modify: `frontend/src/routes/services/services.test.ts`
- Modify: `frontend/src/routes/system-services/system-services.test.ts`
- Modify: `frontend/src/routes/host-tags/host-tags.test.ts`
- Modify: `frontend/src/routes/audit-logs/audit-logs.test.ts`
- Modify: `frontend/src/routes/home.test.ts`
- Modify: `frontend/src/routes/profile/profile.test.ts`
- Modify: `frontend/src/routes/settings/surface-tabs.test.ts`
- Modify: `frontend/tests/e2e/ui-parity.test.ts`
- Modify: `frontend/tests/e2e/ui-parity-responsive.test.ts`

- [ ] **Step 1: Write the failing route regressions for the remaining pages**

Add route-level assertions that lock the shared spec usage rather than just shell presence:

`frontend/src/routes/services/services.test.ts`

```ts
expect(screen.getByText("Approved")).toBeInTheDocument();
expect(
  document.querySelector('[data-ui="table-footer-bar"]'),
).toBeInTheDocument();
expect(
  document.querySelector(
    '[data-ui="table-footer-bar"] nav[aria-label="Pagination"]',
  ),
).toBeInTheDocument();
```

`frontend/src/routes/settings/surface-tabs.test.ts`

```ts
expect(screen.getByText("Global")).toBeInTheDocument();
expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
```

`frontend/src/routes/home.test.ts`

```ts
expect(screen.getByText("Updates pending")).toBeInTheDocument();
expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
```

Run:

```bash
cd frontend && npm run test -- src/routes/services/services.test.ts src/routes/system-services/system-services.test.ts src/routes/host-tags/host-tags.test.ts src/routes/audit-logs/audit-logs.test.ts src/routes/home.test.ts src/routes/profile/profile.test.ts src/routes/settings/surface-tabs.test.ts
```

Expected: FAIL where routes still use route-local menu rows, free-floating pagination, or stat-card colors that do not match the spec.

- [ ] **Step 2: Migrate the remaining routes to the spec-aligned shared contract**

Apply the shared primitives across the remaining route files:

- replace raw menu buttons with `ContextMenuItem`
- replace free-floating pagination blocks with `TableFooterBar`
- make sure totals text and pagination controls sit in the same shared footer row on every table-backed route instead of being wrapped in route-local `mt-4` containers
- tighten Settings tab/body spacing to the spec’s two-column form rhythm
- align Dashboard stat cards to Section 4.5 color mapping
- keep Services/System Services/Host Tags/Audit Logs/Profile on shared table, badge, and menu primitives
- either align `login`, `register`, and `device` to the shared page language or document an explicit defer note in `docs/development/frontend-components.md` if they intentionally keep a separate auth/device shell

Representative replacement for menu rows:

```svelte
<ContextMenuShell top={menuPos.top} left={menuPos.left} onclose={closeMenu}>
    <li><ContextMenuItem label="Edit" onclick={() => openEditModal(item)} /></li>
    <li><ContextMenuItem label="Delete" destructive onclick={() => requestDelete(item)} /></li>
</ContextMenuShell>
```

Representative replacement for table footer:

```svelte
<DataTable ...>
    {#snippet footer()}
        <TableFooterBar {currentPage} {totalPages} total={totalItems} onPageChange={load} />
    {/snippet}
</DataTable>
```

Run:

```bash
cd frontend && npm run test -- src/routes/services/services.test.ts src/routes/system-services/system-services.test.ts src/routes/host-tags/host-tags.test.ts src/routes/audit-logs/audit-logs.test.ts src/routes/home.test.ts src/routes/profile/profile.test.ts src/routes/settings/surface-tabs.test.ts
cd frontend && npm run check && npm run lint
```

Expected: PASS with the remaining route families consuming the same shared page language.

- [ ] **Step 3: Refresh parity and responsive coverage, then commit**

Run:

```bash
cd frontend && npm run test:e2e -- --grep "ui parity"
cd frontend && npm run format:check && npm run build
```

Expected: PASS with refreshed desktop and responsive baselines for the remaining route changes.

Commit:

```bash
git add frontend/src/routes/settings/+page.svelte frontend/src/routes/settings/*.svelte frontend/src/routes/settings/global/+page.svelte frontend/src/routes/+page.svelte frontend/src/routes/services/+page.svelte frontend/src/routes/system-services/+page.svelte frontend/src/routes/host-tags/+page.svelte frontend/src/routes/audit-logs/+page.svelte frontend/src/routes/profile/+page.svelte frontend/src/routes/login/+page.svelte frontend/src/routes/register/+page.svelte frontend/src/routes/device/+page.svelte
git add frontend/src/routes/services/services.test.ts frontend/src/routes/system-services/system-services.test.ts frontend/src/routes/host-tags/host-tags.test.ts frontend/src/routes/audit-logs/audit-logs.test.ts frontend/src/routes/home.test.ts frontend/src/routes/profile/profile.test.ts frontend/src/routes/settings/surface-tabs.test.ts frontend/tests/e2e/ui-parity.test.ts frontend/tests/e2e/ui-parity-responsive.test.ts frontend/tests/e2e/ui-parity.test.ts-snapshots frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots
git commit -m "refactor: align remaining pages with design spec"
```
