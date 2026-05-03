# Spec: Tabbed Layout for Software Item Detail Page

**Date:** 2026-05-03
**File in scope:** `frontend/src/routes/software/[id]/+page.svelte`

## Problem

The software item detail page currently renders all `software_item.tabs` surfaces as full-width
stacked `SectionCard` blocks above the hosts table. With the Proxmox plugin registered, "Proxmox
Update Protection" consumes more than half the visible page before the user reaches the hosts
table, which is the primary content.

The slot is named `software_item.tabs` — it was designed for tabs — but the UI never implemented
them.

## Goal

Replace the stacked-surface layout with a `TabStrip`-based tab layout. The "Proxmox Update
Protection" panel (and any future surfaces) becomes a tab, invisible until the user clicks it.
The hosts table remains the default view.

## Decisions

| Decision               | Choice                                                                    |
| ---------------------- | ------------------------------------------------------------------------- |
| Tab implementation     | `TabStrip.svelte` (already exists, used on settings page)                 |
| Default tab            | "Overview" (hosts table)                                                  |
| URL persistence        | `?tab=<surface_id>` or `?tab=overview`; same pattern as settings page     |
| Fallback (no surfaces) | Hide TabStrip entirely; render current flat layout                        |
| Header                 | Extracted to its own `SectionCard`, always visible above tabs             |
| Card structure         | Split: header card, bare TabStrip, content panels — mirrors settings page |

## Page Structure After Change

```text
PageShell
  ← Back to Software
  [SectionCard — header]            ← always visible
    name / star / version / buttons
  [TabStrip]                        ← only when surfaces exist
    Overview | <surface label> …
  [content area — keyed to activeTab]
    Overview tab → SectionCard wrapping the hosts DataTable
    Surface tab  → bare SurfaceReadPanel (no extra SectionCard; tab label is already the title)
```

When `softwareItemTabSurfaces.length === 0`, the TabStrip is absent and the hosts `SectionCard`
renders directly below the header card (same visual result as today).

## Tab List

```ts
tabItems = [
  { id: 'overview', label: 'Overview' },   // always first
  ...softwareItemTabSurfaces.map(s => ({ id: s.surface_id, label: s.label }))
]
```

Computed as `$derived`.

## URL Sync

- Read: `let activeTab: string = $state(page.url.searchParams.get('tab') ?? 'overview')`
- Write: `$effect` mirrors the settings page — `goto(..., { replaceState: true, keepFocus: true, noScroll: true })`
- For the default tab, omit the query string: `activeTab === 'overview' ? '' : '?tab=' + activeTab`
- Invalid slug (unknown `?tab=` value): use the same `$effect` validation pattern from settings
  — wait for `surfaceRegistryLoaded`, then if neither `'overview'` nor any `surface_id` matches,
  reset to `'overview'`

## Surface Tab Active-Tab Validation

Reuse the exact pattern from the settings page:

```ts
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
// if !getSurfaceRegistryLoaded() && activeTab !== 'overview' → wait
// if not accessible and not pending → reset to 'overview'
```

Imports needed: `getSurfaceRegistryLoaded`, `getSurfaceReadRequested`, `getSurfaceReadLoading`
from `$lib/surfaces/registry.svelte`; `isSurfaceTabPending` from `$lib/surfaces/read-model`.

## Data Loading

- **Surface descriptors** (read models): keep the existing eager `$effect` that calls
  `loadSurfaceReadModels(softwareItemTabSurfaces.map(s => s.surface_id))` on mount. This
  is lightweight (metadata only) and needed to populate the tab labels for validation.
- **Surface preload data**: naturally lazy — `SurfaceReadPanel` only mounts when its surface tab
  is active, so the preload interaction fires on first tab activation.
- **`reloadToken`**: unchanged. It increments on every `loadItem()` call. Because surface panels
  are unmounted when their tab is inactive, only the visible panel reacts to the token change.

## Template Structure (abbreviated)

```svelte
<!-- Header card — always visible -->
<SectionCard>
  <!-- name, star, version info, action buttons — unchanged from today -->
</SectionCard>

<!-- TabStrip — only when surfaces exist -->
{#if softwareItemTabSurfaces.length > 0}
  <TabStrip items={tabItems} activeId={activeTab} idBase="software-detail" onSelect={(id) => (activeTab = id)} />
{/if}

<!-- Tab content -->
{#if activeTab === 'overview' || softwareItemTabSurfaces.length === 0}
  <SectionCard>
    <!-- hosts DataTable — unchanged -->
  </SectionCard>
{:else}
  {#each softwareItemTabSurfaces as surface (surface.surface_id)}
    {#if activeTab === surface.surface_id}
      <SurfaceReadPanel
        {surface}
        read={softwareItemTabReads[surface.surface_id]}
        baseParams={softwareItemTabBaseParams}
        reloadToken={softwareItemTabsReloadToken}
      />
    {/if}
  {/each}
{/if}
```

The `|| softwareItemTabSurfaces.length === 0` guard on the overview branch ensures the hosts
table always renders when there are no surfaces (fallback mode).

## Test Changes

### `software-detail.test.ts`

The existing test `'loads software-item tab surfaces and passes software_item_id to panel reads'`
currently asserts:

```ts
expect(
  screen.getByRole("heading", { name: "Software Item Diagnostics" }),
).toBeInTheDocument();
```

After this change, that assertion **fails**: the surface panel is not mounted until the user
clicks its tab. Update the test to:

1. Assert the tab button is visible: `screen.getByRole('tab', { name: 'Software Item Diagnostics' })`
2. Click the tab: `await fireEvent.click(...)`
3. Assert the panel renders (the `invokeSurfaceInteraction` call and/or panel heading)

Add a new test: `'defaults to Overview tab and shows hosts table'` — verify the hosts DataTable
is visible without any tab interaction.

Add a new test: `'renders flat layout when no surfaces are registered'` — verify no TabStrip
renders when `getSurfacesBySlot` returns `[]`.

### `software-detail-update-trigger.test.ts`

Tests that trigger updates via the hosts table do not need to change — the hosts table is still
in the Overview tab which is the default. No tab navigation is required. Verify these tests still
pass unchanged (they should, because the DataTable is always rendered when `activeTab === 'overview'`).

## Out of Scope

- Changes to the `SurfaceReadPanel` component itself
- Changes to the Proxmox plugin's surface descriptor or form content
- URL routing changes beyond the `?tab=` query string
- Any other page in the app
