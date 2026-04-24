<!-- markdownlint-disable MD013 -->

# Responsive Layout Design

**Status:** Approved 2026-04-24

---

## Goal

Close the last open responsive gap from `docs/development/ui/layout.md`: data views on `/software` and all `DataTable` usages render identically at every viewport; no mobile-friendly fallback exists.

---

## Scope

Two independent tracks, one plan:

| Track | Deliverable |
| --- | --- |
| A | `DataTable` gains `mobileMode`, column-level `mobileHide`/`mobileTitle`, optional `mobileRow` snippet, and scroll-mode table-width fix |
| B | `SoftwareGroupList.svelte` extracted from `/software/+page.svelte`; adds its own mobile card layout for the hierarchical group/host structure |

Two new Playwright projects (`chromium-mobile`, `chromium-mobile-dark`) at 393×852 with DPR=1 cover mobile-width snapshots for both tracks.

---

## Key Decisions

### Do NOT migrate /software into DataTable

The group/host hierarchy, per-group async child loading, batch-select checkbox column, truncation rows, and permission-gated context menu are architecturally incompatible with DataTable's flat row model. Forcing DataTable to absorb this would require adding `rowGroup`, per-item loading state, and domain logic into a shared primitive used by 23+ other consumers.

Correct direction: DataTable stays simple, SoftwareGroupList extracts and owns its own structure.

### DataTable minimal extension only

DataTable gains exactly:

- `mobileMode?: 'scroll' | 'cards'` (absent = no change to existing behavior; opt-in per table)
- `DataTableColumn.mobileHide?: boolean` — exclude column from auto-generated cards
- `DataTableColumn.mobileTitle?: boolean` — render as card heading rather than key/value pair
- `mobileRow?: Snippet<[Record<string, unknown>]>` — custom card renderer for rows in cards mode

No `rowGroup`, no `expandedKeys`, no `groupOverflow`. Those stay in SoftwareGroupList.

### Dual-DOM rendering (not JS-driven)

Both table and cards layouts exist in the DOM simultaneously. CSS media query classes (`max-sm:hidden`, `sm:hidden`) control visibility. This avoids SSR/hydration flash — the browser applies media queries before JS runs, so the correct layout is visible from first paint.

### Scroll mode: `w-max` not `min-w-full`

`mobileMode` is optional with no default — absence is distinct from `'scroll'`. When `mobileMode` is **absent** (`undefined`), the `<table>` keeps `min-w-full` — no visual regression for existing consumers. When `mobileMode='scroll'` is **explicitly** set, the `<table>` gets `w-max` so content-wide tables can overflow and trigger horizontal scroll.

### Accessibility in cards mode

Auto-generated cards use `role="list"` on the container, `role="listitem"` on each card, `<dl>/<dt>/<dd>` for key/value pairs. Action cells render in `<div role="group" aria-label={rowActionsLabel}>` outside the `<dl>`. `display:none` via Tailwind media-query classes hides elements from the ARIA tree in all modern browsers; this is sufficient for current targets (modern Chromium). If older AT support is required in the future, add `aria-hidden="true"` to the hidden layout container.

### `mobileTitle` enforcement

At most one column should have `mobileTitle: true`. No runtime enforcement — caller responsibility. If multiple columns carry `mobileTitle: true`, the component uses the first matching column. No warning is emitted; this is a design-time constraint.

### `mobileRow` and auto-generated cards

`mobileMode='cards'` without `mobileRow` is the normal auto-generated path — the component renders `<dl>/<dt>/<dd>` cards using `String(rowValue[col.key])` for each non-hidden column. `mobileRow` is only needed when cell values cannot be represented as strings (e.g., icon renderers, interactive elements). The presence of the `row` snippet (desktop custom renderer) does **not** affect mobile card generation — `row` is only consulted for the desktop table layout.

---

## DataTable API After Changes

```typescript
export type DataTableColumn = {
  key: string;
  label: string;
  align?: 'left' | 'center' | 'right';
  mobileHide?: boolean;   // exclude from cards key/value list
  mobileTitle?: boolean;  // render as card heading (at most one; caller responsibility)
};

// New props added to existing prop set:
mobileMode?: 'scroll' | 'cards';               // absent = legacy min-w-full; 'scroll' = w-max + horizontal scroll
mobileRow?: Snippet<[Record<string, unknown>]>; // custom card per row; absent = auto-generated dl/dt/dd cards
```

Existing props and their behavior are unchanged.

**`mobileMode` behavior summary:**

| `mobileMode` value | Desktop table width | Mobile behaviour |
| --- | --- | --- |
| absent (`undefined`) | `min-w-full` | table shown at all viewports (no change from today) |
| `'scroll'` | `w-max` | table shown at all viewports, scrollable on mobile |
| `'cards'` | `min-w-full` | table wrapper gets `max-sm:hidden` (hidden on mobile); cards container gets `sm:hidden` (hidden on desktop ≥640px) |

---

## SoftwareGroupList Component

Extracted from `frontend/src/routes/software/+page.svelte` lines 956–1256 (the `div[data-ui="software-group-list"]` block and its `TableFooterBar`).

**Props interface:**

```typescript
{
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
}
```

Helper functions (`detailHosts`, `visibleHosts`, `hiddenHostCount`, `hiddenHostsSummary`, `updateableHostCount`, `hasAnyUpdateableHosts`, `softwareUpdateLabel`, `primaryPluginLabel`, `hostDisplayName`, `isSingleHostItem`, `singleHost`, `versionLabel`, `versionTitle`, `groupIsOpen`) move from the page into the component.

**Desktop layout:** `max-sm:hidden` wrapper — hidden on mobile (<640px), visible on desktop. Contains the existing CSS Grid rows unchanged.

**Mobile layout:** `sm:hidden` wrapper — hidden on desktop (≥640px), visible on mobile. Card-per-item approach. Compact single-host items show name + hostname + plugin badge + version stacked. Multi-host items show name + expand pill + host count + update label; expanding reveals host sub-cards indented with a left border.

Both layouts carry `data-ui` attributes and `data-testid` attributes for test targeting.

---

## Playwright Projects

Added to `frontend/playwright.config.ts`:

```typescript
{
  name: 'chromium-mobile',
  use: { ...devices['Desktop Chrome'], colorScheme: 'light', viewport: { width: 393, height: 852 } }
},
{
  name: 'chromium-mobile-dark',
  use: { ...devices['Desktop Chrome'], colorScheme: 'dark', viewport: { width: 393, height: 852 } }
},
```

Using `devices['Desktop Chrome']` as base ensures DPR=1 (required by parity harness). Both project names start with `chromium` (required by `PARITY_REQUIRED_PROJECT` guard). `chromium-mobile-dark` includes `'dark'` (required by dark-mode detection in `assertDeterministicCaptureProfile`).

---

## Files Changed

| File | Change |
| --- | --- |
| `frontend/playwright.config.ts` | Add `chromium-mobile` and `chromium-mobile-dark` projects |
| `frontend/src/lib/components/ui/DataTable.svelte` | Add `mobileMode`, `mobileRow`, column flags, dual-DOM rendering, scroll mode `w-max` |
| `frontend/src/lib/components/ui/DataTable.test.ts` | Tests for all new behavior |
| `frontend/src/lib/components/ui/SoftwareGroupList.svelte` | New: extracted group list + mobile card layout |
| `frontend/src/routes/software/+page.svelte` | Replace inline group list with `<SoftwareGroupList>` |
| `frontend/tests/e2e/software-area.spec.ts` | Add mobile snapshot variants |
| `docs/development/ui/layout.md` | Update responsive status to `Implemented` |
