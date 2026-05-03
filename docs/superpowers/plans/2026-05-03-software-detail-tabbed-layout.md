# Software Detail Tabbed Layout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the stacked-surface layout on the software item detail page with a `TabStrip`
tab layout so the Proxmox Update Protection panel (and future surfaces) appear as tabs rather than
dominating the page above the hosts table.

**Architecture:** A single `activeTab` state variable (default `'overview'`) controls which
content panel renders. The tab list is `['overview', ...surfaces]`. Two `$effect` blocks mirror
the settings page pattern: one validates `activeTab` after the surface registry loads, one syncs
the active tab to `?tab=` in the URL. When no surfaces are registered the TabStrip is hidden and
the hosts table renders directly below the header card.

**Tech Stack:** Svelte 5 runes (`$state`, `$derived`, `$effect`), SvelteKit `goto`, existing
`TabStrip.svelte` component, `SurfaceReadPanel.svelte`, Vitest + Testing Library.

---

## File Map

| File                                                                       | Change                                                    |
| -------------------------------------------------------------------------- | --------------------------------------------------------- |
| `frontend/src/routes/software/[id]/+page.svelte`                           | Main change — imports, script logic, template restructure |
| `frontend/src/routes/software/[id]/software-detail.test.ts`                | Expand mocks, update broken test, add two new tests       |
| `frontend/src/routes/software/[id]/software-detail-update-trigger.test.ts` | Expand registry mock only                                 |

---

### Task 1: Expand registry mocks in both test files

The new `$effect` in `+page.svelte` will import three additional functions from
`$lib/surfaces/registry.svelte`: `getSurfaceRegistryLoaded`, `getSurfaceReadRequested`, and
`getSurfaceReadLoading`. Both test files mock that module but don't include those functions —
without them the component throws at runtime and every test in both suites fails.

**Files:**

- Modify: `frontend/src/routes/software/[id]/software-detail.test.ts:39-44`
- Modify: `frontend/src/routes/software/[id]/software-detail-update-trigger.test.ts:57-61`

- [ ] **Step 1: Update the registry mock in `software-detail.test.ts`**

Replace the existing `vi.mock('$lib/surfaces/registry.svelte', ...)` block (lines 39–44):

```ts
vi.mock("$lib/surfaces/registry.svelte", () => ({
  getSurfaceReadModel: vi.fn(() => undefined),
  getSurfaceProviders: vi.fn(() => []),
  getSurfacesBySlot: vi.fn(() => []),
  loadSurfaceReadModels: vi.fn(() => Promise.resolve()),
  getSurfaceRegistryLoaded: vi.fn(() => true),
  getSurfaceReadRequested: vi.fn(() => false),
  getSurfaceReadLoading: vi.fn(() => false),
}));
```

- [ ] **Step 2: Update the registry mock in `software-detail-update-trigger.test.ts`**

Replace the existing `vi.mock('$lib/surfaces/registry.svelte', ...)` block (lines 57–61):

```ts
vi.mock("$lib/surfaces/registry.svelte", () => ({
  getSurfaceReadModel: vi.fn(() => undefined),
  getSurfacesBySlot: vi.fn(() => []),
  loadSurfaceReadModels: vi.fn(() => Promise.resolve()),
  getSurfaceRegistryLoaded: vi.fn(() => true),
  getSurfaceReadRequested: vi.fn(() => false),
  getSurfaceReadLoading: vi.fn(() => false),
}));
```

- [ ] **Step 3: Run the test suites to confirm current baseline**

```bash
cd frontend && npx vitest run src/routes/software/\\[id\\]/ --reporter=verbose 2>&1 | tail -30
```

Expected: both files pass (tests are not yet changed to expect the new tab behaviour).

---

### Task 2: Add failing tests for the new tab layout

Add three new tests to `software-detail.test.ts` and update the one existing test that will break
once the template is restructured. Write them now so they fail before the implementation lands —
that confirms the tests actually cover the new behaviour.

**Files:**

- Modify: `frontend/src/routes/software/[id]/software-detail.test.ts`

- [ ] **Step 1: Update the existing surface-slot test that will break**

In `software-detail.test.ts`, find the test named
`'loads software-item tab surfaces and passes software_item_id to panel reads'` and replace its
body. The current body asserts `screen.getByRole('heading', { name: 'Software Item Diagnostics' })`
which will stop working once the surface panel is only mounted after a tab click.

Replace the full test body (keep the surrounding `it(...)` wrapper) with:

```ts
it("loads software-item tab surfaces and passes software_item_id to panel reads", async () => {
  const item = makeSoftwareItem([makeHost()]);
  const softwareItemTabSurface = makeSurface(
    "software.item.tab.surface",
    "software_item.tabs",
    "Software Item Diagnostics",
    Permission.ViewSoftware,
  );
  const hostContextSurface = makeSurface(
    "software.item.host.context.surface",
    "software_item.host_context_menu",
    "Host Context Action",
    Permission.UpdateSoftware,
  );
  const reads = new Map<string, SurfaceReadResponse>([
    [
      softwareItemTabSurface.surface_id,
      makeRenderableRead(softwareItemTabSurface, "load_software_item_tab"),
    ],
    [
      hostContextSurface.surface_id,
      makeRenderableRead(hostContextSurface, "load_host_context"),
    ],
  ]);

  vi.mocked(api.getSoftwareItem).mockResolvedValue(item);
  vi.mocked(surfaceRegistry.getSurfacesBySlot).mockImplementation(
    (slot: string) => {
      if (slot === "software_item.tabs") return [softwareItemTabSurface];
      if (slot === "software_item.host_context_menu")
        return [hostContextSurface];
      return [];
    },
  );
  vi.mocked(surfaceRegistry.getSurfaceReadModel).mockImplementation(
    (surfaceId: string) => reads.get(surfaceId),
  );

  render(SoftwareDetailPage);
  await waitFor(() =>
    expect(
      screen.getByRole("heading", { level: 1, name: "Demo App" }),
    ).toBeInTheDocument(),
  );

  // Tab button is visible before clicking
  const tabBtn = screen.getByRole("tab", { name: "Software Item Diagnostics" });
  expect(tabBtn).toBeInTheDocument();
  expect(vi.mocked(surfaceRegistry.loadSurfaceReadModels)).toHaveBeenCalledWith(
    ["software.item.tab.surface"],
  );

  // Click the tab — panel mounts, preload fires
  await fireEvent.click(tabBtn);
  await waitFor(() =>
    expect(vi.mocked(api.invokeSurfaceInteraction)).toHaveBeenCalledWith(
      "software.item.tab.surface",
      "load_software_item_tab",
      {
        params: { software_item_id: "software-1" },
        target_provider_id: undefined,
      },
    ),
  );
});
```

- [ ] **Step 2: Add `'defaults to Overview tab and shows hosts table'` test**

Append inside the `describe('Software Detail shared-surface slots', ...)` block:

```ts
it("defaults to Overview tab and shows hosts table", async () => {
  const item = makeSoftwareItem([makeHost()]);
  const softwareItemTabSurface = makeSurface(
    "software.item.tab.surface",
    "software_item.tabs",
    "Software Item Diagnostics",
    Permission.ViewSoftware,
  );
  const reads = new Map<string, SurfaceReadResponse>([
    [
      softwareItemTabSurface.surface_id,
      makeRenderableRead(softwareItemTabSurface, "load_software_item_tab"),
    ],
  ]);

  vi.mocked(api.getSoftwareItem).mockResolvedValue(item);
  vi.mocked(surfaceRegistry.getSurfacesBySlot).mockImplementation(
    (slot: string) => {
      if (slot === "software_item.tabs") return [softwareItemTabSurface];
      return [];
    },
  );
  vi.mocked(surfaceRegistry.getSurfaceReadModel).mockImplementation(
    (surfaceId: string) => reads.get(surfaceId),
  );

  render(SoftwareDetailPage);
  await waitFor(() =>
    expect(
      screen.getByRole("heading", { level: 1, name: "Demo App" }),
    ).toBeInTheDocument(),
  );

  // Overview tab is active by default — hosts table visible, surface panel not mounted
  expect(screen.getByRole("tab", { name: "Overview" })).toBeInTheDocument();
  expect(
    screen.getByRole("tab", { name: "Software Item Diagnostics" }),
  ).toBeInTheDocument();
  // DataTable renders a columnheader for Hostname in the Overview tab
  expect(
    screen.getByRole("columnheader", { name: "Hostname" }),
  ).toBeInTheDocument();
  // Surface interaction has not been called (panel not mounted)
  expect(vi.mocked(api.invokeSurfaceInteraction)).not.toHaveBeenCalled();
});
```

- [ ] **Step 3: Add `'renders flat layout when no surfaces are registered'` test**

Append after the previous test, still inside the same `describe` block:

```ts
it("renders flat layout when no surfaces are registered", async () => {
  const item = makeSoftwareItem([makeHost()]);
  vi.mocked(api.getSoftwareItem).mockResolvedValue(item);
  vi.mocked(surfaceRegistry.getSurfacesBySlot).mockReturnValue([]);

  render(SoftwareDetailPage);
  await waitFor(() =>
    expect(
      screen.getByRole("heading", { level: 1, name: "Demo App" }),
    ).toBeInTheDocument(),
  );

  // No tablist — flat layout
  expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
  // Hosts table still present
  expect(
    screen.getByRole("columnheader", { name: "Hostname" }),
  ).toBeInTheDocument();
});
```

- [ ] **Step 4: Run the tests — confirm all three tab-behaviour tests fail**

```bash
cd frontend && npx vitest run src/routes/software/\\[id\\]/software-detail.test.ts --reporter=verbose 2>&1 | tail -40
```

Expected: all three tab-related tests FAIL — the updated `'loads software-item tab surfaces…'`
test (its new body asserts `getByRole('tab', ...)` which doesn't exist until Task 5) plus both
new tests. The `'keeps host-context menu surface behavior active'` test should still PASS.

---

### Task 3: Update imports in `+page.svelte`

Add `TabStrip` and `type TabStripItem` to the UI component import, and add the four new surface
registry/read-model functions.

**Files:**

- Modify: `frontend/src/routes/software/[id]/+page.svelte:41-42`
- Modify: `frontend/src/routes/software/[id]/+page.svelte:43-54`

- [ ] **Step 1: Expand the `$lib/surfaces/registry.svelte` import (line 41)**

Replace:

```ts
import {
  getSurfaceReadModel,
  getSurfacesBySlot,
  loadSurfaceReadModels,
} from "$lib/surfaces/registry.svelte";
```

With:

```ts
import {
  getSurfaceReadModel,
  getSurfacesBySlot,
  getSurfaceRegistryLoaded,
  getSurfaceReadRequested,
  getSurfaceReadLoading,
  loadSurfaceReadModels,
} from "$lib/surfaces/registry.svelte";
```

- [ ] **Step 2: Expand the `$lib/surfaces/read-model` import (line 42)**

Replace:

```ts
import {
  filterSurfacesByPermission,
  shouldUseSurfaceRoute,
} from "$lib/surfaces/read-model";
```

With:

```ts
import {
  filterSurfacesByPermission,
  isSurfaceTabPending,
  shouldUseSurfaceRoute,
} from "$lib/surfaces/read-model";
```

- [ ] **Step 3: Add `TabStrip` and `TabStripItem` to the UI import block (lines 43-54)**

Replace:

```ts
import {
  ActionBadge,
  Callout,
  ContextMenuItem,
  ContextMenuShell,
  DataTable,
  ModalShell,
  PageShell,
  ReleaseNotes,
  SectionCard,
  StatusBadge,
} from "$lib/components/ui";
```

With:

```ts
import {
  ActionBadge,
  Callout,
  ContextMenuItem,
  ContextMenuShell,
  DataTable,
  ModalShell,
  PageShell,
  ReleaseNotes,
  SectionCard,
  StatusBadge,
  TabStrip,
  type TabStripItem,
} from "$lib/components/ui";
```

- [ ] **Step 4: Verify the file compiles**

```bash
cd frontend && npx svelte-check --tsconfig tsconfig.json 2>&1 | grep -E "error|Error" | head -20
```

Expected: no new errors introduced by the import changes.

---

### Task 4: Add script logic — `activeTab`, `tabItems`, validation effect, URL-sync effect

All additions go in the `<script lang="ts">` block of `+page.svelte`, after the existing
`softwareItemTabBaseParams` derived (around line 182) and before `softwareItemTabsReloadToken`.

**Files:**

- Modify: `frontend/src/routes/software/[id]/+page.svelte`

- [ ] **Step 1: Add `activeTab` state and `tabItems` derived**

After line 181 (`const softwareItemTabBaseParams = ...`), insert:

```ts
const tabItems = $derived.by<TabStripItem[]>(() => {
  const items: TabStripItem[] = [{ id: "overview", label: "Overview" }];
  for (const surface of softwareItemTabSurfaces) {
    items.push({ id: surface.surface_id, label: surface.label });
  }
  return items;
});

let activeTab: string = $state(page.url.searchParams.get("tab") ?? "overview");
```

- [ ] **Step 2: Add the validation `$effect` (before URL-sync)**

After the `activeTab` declaration (still in the script block, before the existing `$effect`
blocks that call `loadSurfaceReadModels`), insert:

```ts
// Validate activeTab — must be declared before URL-sync $effect
$effect(() => {
  const surfaceRegistryLoaded = getSurfaceRegistryLoaded();
  const isSurfaceAccessible = softwareItemTabSurfaces.some(
    (s) => s.surface_id === activeTab,
  );
  const isPending = isSurfaceTabPending({
    activeTab,
    slotSurfaces: softwareItemTabSurfaces,
    readBySurface: softwareItemTabReads,
    isReadRequested: getSurfaceReadRequested(activeTab),
    isReadLoading: getSurfaceReadLoading(activeTab),
  });
  if (!surfaceRegistryLoaded && activeTab !== "overview") return;
  if (activeTab !== "overview" && !isSurfaceAccessible && !isPending) {
    activeTab = "overview";
  }
});

// Sync activeTab to URL — declared after validation $effect
$effect(() => {
  const search = activeTab !== "overview" ? `?tab=${activeTab}` : "";
  goto(search ? `${location.pathname}${search}` : location.pathname, {
    replaceState: true,
    keepFocus: true,
    noScroll: true,
  });
});
```

- [ ] **Step 3: Verify the file still compiles**

```bash
cd frontend && npx svelte-check --tsconfig tsconfig.json 2>&1 | grep -E "error|Error" | head -20
```

Expected: no errors.

---

### Task 5: Restructure the template

This is the largest change. The outer `<SectionCard>` (lines 773–997) currently wraps everything:
header, surface blocks, and the hosts DataTable. Split it into:

1. A header-only `<SectionCard>` (no title prop, same as today)
2. A conditional `<TabStrip>` (only when surfaces exist)
3. A conditional content area: `<SectionCard>` with the hosts table for Overview, or
   `<SectionCard title={surface.label}>` with `<SurfaceReadPanel>` for surface tabs

**Files:**

- Modify: `frontend/src/routes/software/[id]/+page.svelte:772-997`

- [ ] **Step 1: Replace the `{:else if item}` block in the template**

Locate the comment `<!-- Header -->` at line 772 through the closing `</SectionCard>` at line 997
(the `{/if}` at line 998 closes the `{:else if item}` branch — keep it).

Replace everything from `<!-- Header -->` (line 772) through `</SectionCard>` (line 997), keeping
the `{/if}` at line 998 intact, with:

```svelte
			<!-- Header — always visible -->
			<SectionCard>
				<div class="mb-6 flex flex-wrap items-start justify-between gap-4">
					<div>
						<h2 class="text-section-title font-semibold text-[var(--text-primary)]">
							{#if isValidLogoUrl(item.icon_url)}
								<img
									src={item.icon_url}
									alt=""
									class="h-8 w-8 inline-block mr-2 rounded-card object-contain align-middle"
									referrerpolicy="no-referrer"
								/>
							{/if}{item.name}
						</h2>
						<div class="mt-2 flex flex-wrap items-center gap-2">
							{#if canManage}
								<button
									class="cursor-pointer text-[1.25rem] leading-none transition-[background,border-color,color] duration-fast"
									class:text-[var(--color-warning)]={item.featured}
									class:text-[var(--text-muted)]={!item.featured}
									title={item.featured ? 'Unfeature' : 'Feature'}
									onclick={toggleFeatured}
									aria-label="{item.featured ? 'Unfeature' : 'Feature'} {item.name}"
								>
									{item.featured ? '★' : '☆'}
								</button>
							{:else}
								<span
									class="text-[1.25rem] {item.featured ? 'text-[var(--color-warning)]' : 'text-[var(--text-muted)]'}"
									>{item.featured ? '★' : '☆'}</span
								>
							{/if}
							{#if item.plugins.length > 0}
								<span class="text-sm text-[var(--text-muted)]">{item.plugins.join(', ')}</span>
							{/if}
						</div>
						<div class="mt-2 space-y-1 text-sm text-[var(--text-muted)]">
							{#if item.latest_version}
								<p>
									Latest version: <span class="font-medium text-[var(--text-primary)]" title={item.latest_version}
										>{formatVersion(item.latest_version)}</span
									>
								</p>
							{/if}
							<p>Last checked: {formatDate(item.last_checked_at)}</p>
							<p>{item.host_count} host{item.host_count !== 1 ? 's' : ''} assigned</p>
						</div>
					</div>
					{#if canManage}
						<div class="flex flex-wrap items-center gap-2">
							{#if canTriggerUpdates && item.update_available}
								<Button variant="primary" onclick={openUpdateAllModal}>Update All</Button>
							{/if}
							<Button variant="secondary" onclick={() => (showAssignModal = true)}>Assign to Host</Button>
							<Button variant="secondary" loading={checkingAll} onclick={checkAllVersions}>Check All Versions</Button>
							{#if canMergeSoftware}
								<Button variant="secondary" onclick={openMergeModal}>Merge...</Button>
							{/if}
							<Button variant="secondary" onclick={openEditModal}>Edit</Button>
							<Button variant="danger" loading={deleteSubmitting} onclick={() => (confirmDelete = true)}>Delete</Button>
						</div>
					{/if}
				</div>
			</SectionCard>

			<!-- TabStrip — only shown when surfaces are registered -->
			{#if softwareItemTabSurfaces.length > 0}
				<TabStrip
					items={tabItems}
					activeId={activeTab}
					ariaLabel="Software detail tabs"
					idBase="software-detail"
					onSelect={(id) => (activeTab = id)}
				/>
			{/if}

			<!-- Tab content -->
			{#if activeTab === 'overview' || softwareItemTabSurfaces.length === 0}
				<SectionCard>
					<!-- Hosts table -->
					<DataTable
						columns={[]}
						rows={item.hosts as unknown as Record<string, unknown>[]}
						emptyTitle="No hosts assigned"
						emptyDescription="Assign hosts to this software item to start tracking."
						rowKey={(row) => (row as unknown as SoftwareItemHostSummary).id}
					>
						{#snippet header()}
							<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
								<th
									class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
									scope="col">Hostname</th
								>
								<th
									class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
									scope="col"
								>
									Installed Version
								</th>
								<th
									class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
									scope="col"
								>
									Latest Version
								</th>
								<th
									class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
									scope="col">Status</th
								>
								<th
									class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
									scope="col"
								>
									Detected At
								</th>
								{#if canManage}
									<th
										class="w-20 table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
										scope="col"
									></th>
								{/if}
							</tr>
						{/snippet}
						{#snippet row(rowValue, _index)}
							{@const host = rowValue as unknown as SoftwareItemHostSummary}
							<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
								<td class="table-cell-pad text-[var(--text-primary)]">
									<a href="/hosts/{host.host_id}" class="hover:underline font-medium">{host.hostname}</a
									>{#if host.qualifier}<StatusBadge tone="info" label={host.qualifier} />{/if}
									{#if host.friendly_name && host.friendly_name !== host.hostname}
										<span class="block text-xs text-[var(--text-muted)]">{host.friendly_name}</span>
									{/if}
									{#if host.plugins.length > 0}
										<div class="mt-1 space-y-0.5">
											{#each groupHostPlugins(host.plugins) as group (group.name)}
												<div class="text-xs text-[var(--text-muted)]">
													<span class="font-medium">{group.name}</span><span class="opacity-60">
														· {group.roles.join(' · ')}</span
													>
												</div>
											{/each}
										</div>
									{:else}
										<span class="mt-1 block text-xs italic text-[var(--text-muted)]">No plugins configured</span>
									{/if}
								</td>
								<td
									class="table-cell-pad whitespace-nowrap text-[var(--text-primary)]"
									title={host.installed_version ?? undefined}
									>{formatVersion(resolveDisplayVersion(host.installed_version, host.installed_display_version))}</td
								>
								<td class="table-cell-pad whitespace-nowrap text-[var(--text-primary)]">
									<span title={host.latest_version ?? item?.latest_version ?? undefined}
										>{formatVersion(
											resolveDisplayVersion(
												host.latest_version ?? item?.latest_version,
												getReleaseMeta(host)?.display_version
											)
										)}</span
									>
									{#if getReleaseMeta(host)}
										<button
											class="mt-0.5 block text-xs text-[var(--accent)] hover:underline"
											onclick={() => openReleaseNotesModal(host)}>Release notes ↗</button
										>
									{/if}
									{#if getReleaseMeta(host)?.attestation_status === 'Verified'}
										<span class="mt-0.5 block" title="GitHub Actions attestation verified">
											<StatusBadge tone="success" label="Attested" />
										</span>
									{:else if getReleaseMeta(host)?.attestation_status === 'NotFound'}
										<span class="mt-0.5 block" title="No GitHub Actions attestation found">
											<StatusBadge tone="danger" label="Not Attested" />
										</span>
									{/if}
								</td>
								<td class="table-cell-pad text-[var(--text-primary)]">
									{#if canView && host.active_update_history_id}
										<span class="inline-flex" title="View update progress">
											<ActionBadge
												variant="navigation"
												tone="info"
												idleLabel="In Progress"
												hoverLabel="→ Log"
												onclick={() => openLiveModal(host.active_update_history_id!, host.hostname)}
											/>
										</span>
									{:else if canTriggerUpdates && host.update_available}
										<span
											class="inline-flex"
											title={`Update to ${formatVersion(resolveDisplayVersion(host.latest_version ?? item?.latest_version, getReleaseMeta(host)?.display_version))}`}
										>
											<ActionBadge
												variant="navigation"
												tone="accent"
												idleLabel="Update"
												hoverLabel="Update"
												onclick={() => openUpdateModal(host)}
											/>
										</span>
									{:else}
										<StatusBadge tone={versionStatusTone(host)} label={versionStatusLabel(host)} />
									{/if}
								</td>
								<td class="table-cell-pad whitespace-nowrap text-sm text-[var(--text-muted)]"
									>{formatDate(host.installed_version_detected_at)}</td
								>
								{#if canManage}
									<td class="table-cell-pad">
										<div class="actions-menu">
											<Button
												variant="ghost"
												size="sm"
												ariaLabel="Actions for {host.hostname}"
												onclick={(e) => {
													e.stopPropagation();
													toggleMenu(host.id, e.currentTarget);
												}}>&#8943;</Button
											>
										</div>
									</td>
								{/if}
							</tr>
						{/snippet}
					</DataTable>
				</SectionCard>
			{:else}
				{#each softwareItemTabSurfaces as surface (surface.surface_id)}
					{#if activeTab === surface.surface_id}
						<SectionCard title={surface.label}>
							<SurfaceReadPanel
								{surface}
								read={softwareItemTabReads[surface.surface_id]}
								baseParams={softwareItemTabBaseParams}
								reloadToken={softwareItemTabsReloadToken}
							/>
						</SectionCard>
					{/if}
				{/each}
			{/if}
```

- [ ] **Step 2: Verify the file compiles with no errors**

```bash
cd frontend && npx svelte-check --tsconfig tsconfig.json 2>&1 | grep -E "error|Error" | head -20
```

Expected: no errors.

---

### Task 6: Run all tests and verify they pass

- [ ] **Step 1: Run the full frontend test suite**

```bash
cd frontend && npx vitest run --reporter=verbose 2>&1 | tail -50
```

Expected: all tests pass, including:

- `software-detail.test.ts` — all three surface-slot tests green
- `software-detail-update-trigger.test.ts` — all tests green (no logic change, mock fix only)

If tests fail, check:

- Did the `vi.mock` for `$lib/surfaces/registry.svelte` in `software-detail-update-trigger.test.ts`
  include all three new functions? (Task 1, Step 2)
- Did the validation `$effect` get declared before the URL-sync `$effect`? (Task 4, Step 2)
- Does the `{:else if item}` branch in the template still exist after the edit? (The `{/if}` at
  the end of the content area closes the `{:else if item}` — do not remove it.)

- [ ] **Step 2: Run the full quality gate**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build 2>&1 | tail -30
```

Expected: all checks pass.

---

### Task 7: Commit

- [ ] **Step 1: Commit the changes**

```bash
git add \
  frontend/src/routes/software/\[id\]/+page.svelte \
  frontend/src/routes/software/\[id\]/software-detail.test.ts \
  frontend/src/routes/software/\[id\]/software-detail-update-trigger.test.ts
git commit -m "feat(frontend): tabbed layout for software item detail page

Replace stacked surface blocks with TabStrip. Overview tab shows the
hosts table by default; each registered surface in software_item.tabs
slot becomes an additional tab. Tab selection persists to ?tab= URL
param. Falls back to flat layout when no surfaces are registered."
```

---

## Self-Review

**Spec coverage check:**

| Spec requirement                                                             | Task                                                |
| ---------------------------------------------------------------------------- | --------------------------------------------------- |
| Full tab layout with `TabStrip`                                              | Task 5                                              |
| Default tab: "Overview"                                                      | Task 4 (activeTab default), Task 5 (content branch) |
| URL persistence `?tab=<id>`                                                  | Task 4 (URL-sync `$effect`)                         |
| Hide TabStrip when no surfaces                                               | Task 5 (`{#if softwareItemTabSurfaces.length > 0}`) |
| Header in own `SectionCard`, always visible                                  | Task 5                                              |
| Surface panels in `SectionCard title={surface.label}`                        | Task 5                                              |
| Validation `$effect` before URL-sync `$effect`                               | Task 4 (ordering note)                              |
| New imports: `TabStrip`, `TabStripItem`, registry fns, `isSurfaceTabPending` | Task 3                                              |
| Mock expansion in `software-detail.test.ts`                                  | Task 1 + Task 2                                     |
| Mock expansion in `software-detail-update-trigger.test.ts`                   | Task 1                                              |
| Update broken existing test                                                  | Task 2, Step 1                                      |
| New test: defaults to Overview                                               | Task 2, Step 2                                      |
| New test: flat layout when no surfaces                                       | Task 2, Step 3                                      |

All requirements covered. ✓
