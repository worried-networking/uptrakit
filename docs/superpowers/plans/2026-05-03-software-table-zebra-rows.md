# Software Table Zebra Rows and Hover — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add zebra row striping and hover backgrounds to the `/software` data table,
matching `DataTable`'s visual contract, with stripes re-numbering when multi-host items
are expanded or collapsed.

**Architecture:** A `$derived.by` named `flatRowIndices` walks the visible row sequence
and assigns a flat integer index to every rendered row element (header, loading, host
sub-row, overflow). A plain `zebraClass(idx)` helper maps the index to either
`'bg-[var(--bg-raised)]'` (odd) or `''` (even/transparent). Both desktop and mobile
layouts are updated; mobile uses the simpler `{#each ... i}` index since mobile cards
never shift peer cards on expand.

**Tech Stack:** Svelte 5 runes (`$derived.by`, `{@const}`), `SvelteMap`/`SvelteSet`
from `svelte/reactivity`, Tailwind CSS arbitrary-value utilities,
Vitest + `@testing-library/svelte`.

---

## File Map

| File                                                       | Action                                                                                             |
| ---------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `frontend/src/lib/components/ui/SoftwareGroupList.svelte`  | Modify — add `flatRowIndices` derived + `zebraClass` function; update desktop and mobile templates |
| `frontend/src/lib/components/ui/SoftwareGroupList.test.ts` | Create — zebra stripe and hover tests                                                              |

---

## Task 1: Write Failing Tests

**Files:**

- Create: `frontend/src/lib/components/ui/SoftwareGroupList.test.ts`

- [ ] **Step 1: Create the test file**

```typescript
import { cleanup, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SvelteMap, SvelteSet } from "svelte/reactivity";
import SoftwareGroupList from "./SoftwareGroupList.svelte";
import type {
  SoftwareItemDetailResponse,
  SoftwareItemHostSummary,
  SoftwareItemResponse,
} from "$lib/types";

afterEach(() => {
  cleanup();
});

function makeItem(id: string, hostCount: number): SoftwareItemResponse {
  return {
    id,
    name: `Item ${id}`,
    plugins: ["generic_shell"],
    featured: false,
    last_checked_at: null,
    host_count: hostCount,
    installed_version: null,
    installed_display_version: null,
    latest_version: null,
    latest_release_metadata: null,
    update_available: false,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    icon_url: null,
  };
}

function makeHost(rowId: string, hostId: string): SoftwareItemHostSummary {
  return {
    id: rowId,
    host_id: hostId,
    hostname: `host-${hostId}`,
    friendly_name: `host-${hostId}`,
    qualifier: null,
    installed_version: "1.0.0",
    installed_version_detected_at: "2024-01-01T00:00:00Z",
    installed_display_version: null,
    latest_version: null,
    latest_release_metadata: null,
    update_available: false,
    active_update_history_id: null,
    last_updated_at: null,
    linked_at: "2024-01-01T00:00:00Z",
    plugins: [],
  };
}

function makeDetail(
  item: SoftwareItemResponse,
  hosts: SoftwareItemHostSummary[],
): SoftwareItemDetailResponse {
  return { ...item, hosts };
}

function makeProps(
  overrides: Partial<{
    items: SoftwareItemResponse[];
    itemDetailsById: SvelteMap<string, SoftwareItemDetailResponse>;
    itemDetailLoadingIds: SvelteSet<string>;
    collapsedGroupIds: SvelteSet<string>;
    expandedOverflowGroupIds: SvelteSet<string>;
  }> = {},
) {
  return {
    items: overrides.items ?? [],
    itemDetailsById: overrides.itemDetailsById ?? new SvelteMap(),
    itemDetailLoadingIds: overrides.itemDetailLoadingIds ?? new SvelteSet(),
    collapsedGroupIds: overrides.collapsedGroupIds ?? new SvelteSet(),
    expandedOverflowGroupIds:
      overrides.expandedOverflowGroupIds ?? new SvelteSet(),
    batchSelectedIds: new SvelteSet<string>(),
    canManage: false,
    canTriggerUpdates: false,
    pluginTypeNames: new Map<string, string>(),
    totalItems: 0,
    currentPage: 1,
    totalPages: 1,
    onToggleGroup: vi.fn(),
    onToggleOverflow: vi.fn(),
    onToggleBatch: vi.fn(),
    onOpenMenu: vi.fn(),
    onOpenUpdateModal: vi.fn(),
    onPageChange: vi.fn(),
    onToggleFeatured: vi.fn(),
  };
}

describe("SoftwareGroupList — zebra rows", () => {
  it("alternates bg on single-host item headers: idx=0 transparent, idx=1 raised, idx=2 transparent", () => {
    const itemA = makeItem("a", 1);
    const itemB = makeItem("b", 1);
    const itemC = makeItem("c", 1);
    const detailsById = new SvelteMap([
      ["a", makeDetail(itemA, [makeHost("row-a1", "h-a1")])],
      ["b", makeDetail(itemB, [makeHost("row-b1", "h-b1")])],
      ["c", makeDetail(itemC, [makeHost("row-c1", "h-c1")])],
    ]);

    render(
      SoftwareGroupList,
      makeProps({ items: [itemA, itemB, itemC], itemDetailsById: detailsById }),
    );

    const headerA = screen.getByTestId("software-group-header-a");
    const headerB = screen.getByTestId("software-group-header-b");
    const headerC = screen.getByTestId("software-group-header-c");

    expect(headerA.className).not.toContain("bg-[var(--bg-raised)]"); // idx 0: transparent
    expect(headerB.className).toContain("bg-[var(--bg-raised)]"); // idx 1: raised
    expect(headerC.className).not.toContain("bg-[var(--bg-raised)]"); // idx 2: transparent
  });

  it("all header rows have hover:bg-[var(--bg-hover)] and transition classes", () => {
    const itemA = makeItem("a", 1);
    const detailsById = new SvelteMap([
      ["a", makeDetail(itemA, [makeHost("row-a1", "h-a1")])],
    ]);

    render(
      SoftwareGroupList,
      makeProps({ items: [itemA], itemDetailsById: detailsById }),
    );

    const headerA = screen.getByTestId("software-group-header-a");
    expect(headerA.className).toContain("hover:bg-[var(--bg-hover)]");
    expect(headerA.className).toContain(
      "transition-[background,border-color,color]",
    );
    expect(headerA.className).toContain("duration-fast");
  });

  it("host sub-rows continue flat index: itemA=0, itemB_header=1, host1=2, host2=3, itemC=4", () => {
    const itemA = makeItem("a", 1);
    const itemB = makeItem("b", 2);
    const itemC = makeItem("c", 1);
    const host1 = makeHost("row-h1", "hid1");
    const host2 = makeHost("row-h2", "hid2");
    const detailsById = new SvelteMap([
      ["a", makeDetail(itemA, [makeHost("row-a1", "h-a1")])],
      ["b", makeDetail(itemB, [host1, host2])],
      ["c", makeDetail(itemC, [makeHost("row-c1", "h-c1")])],
    ]);

    render(
      SoftwareGroupList,
      makeProps({ items: [itemA, itemB, itemC], itemDetailsById: detailsById }),
    );

    const headerA = screen.getByTestId("software-group-header-a");
    const headerB = screen.getByTestId("software-group-header-b");
    const hostRow1 = screen.getByTestId("software-host-row-row-h1");
    const hostRow2 = screen.getByTestId("software-host-row-row-h2");
    const headerC = screen.getByTestId("software-group-header-c");

    expect(headerA.className).not.toContain("bg-[var(--bg-raised)]"); // idx 0
    expect(headerB.className).toContain("bg-[var(--bg-raised)]"); // idx 1
    expect(hostRow1.className).not.toContain("bg-[var(--bg-raised)]"); // idx 2
    expect(hostRow2.className).toContain("bg-[var(--bg-raised)]"); // idx 3
    expect(headerC.className).not.toContain("bg-[var(--bg-raised)]"); // idx 4
  });

  it("host sub-rows have hover:bg-[var(--bg-hover)]", () => {
    const itemB = makeItem("b", 2);
    const host1 = makeHost("row-h1", "hid1");
    const host2 = makeHost("row-h2", "hid2");
    const detailsById = new SvelteMap([
      ["b", makeDetail(itemB, [host1, host2])],
    ]);

    render(
      SoftwareGroupList,
      makeProps({ items: [itemB], itemDetailsById: detailsById }),
    );

    const hostRow1 = screen.getByTestId("software-host-row-row-h1");
    expect(hostRow1.className).toContain("hover:bg-[var(--bg-hover)]");
  });

  it("collapsing a multi-host item re-stripes downstream headers", () => {
    // With B expanded (3 hosts): A=0(T), B=1(R), h1=2, h2=3, h3=4, C=5(R), D=6(T)
    // With B collapsed:          A=0(T), B=1(R),                   C=2(T), D=3(R)
    // C: raised → transparent. D: transparent → raised.
    const itemA = makeItem("a", 1);
    const itemB = makeItem("b", 3);
    const itemC = makeItem("c", 1);
    const itemD = makeItem("d", 1);
    const host1 = makeHost("row-h1", "hid1");
    const host2 = makeHost("row-h2", "hid2");
    const host3 = makeHost("row-h3", "hid3");
    const detailsById = new SvelteMap([
      ["a", makeDetail(itemA, [makeHost("row-a1", "h-a1")])],
      ["b", makeDetail(itemB, [host1, host2, host3])],
      ["c", makeDetail(itemC, [makeHost("row-c1", "h-c1")])],
      ["d", makeDetail(itemD, [makeHost("row-d1", "h-d1")])],
    ]);
    const items = [itemA, itemB, itemC, itemD];

    // B expanded
    const { rerender } = render(
      SoftwareGroupList,
      makeProps({
        items,
        itemDetailsById: detailsById,
        collapsedGroupIds: new SvelteSet(),
      }),
    );
    expect(screen.getByTestId("software-group-header-c").className).toContain(
      "bg-[var(--bg-raised)]",
    ); // idx 5
    expect(
      screen.getByTestId("software-group-header-d").className,
    ).not.toContain("bg-[var(--bg-raised)]"); // idx 6

    // B collapsed
    rerender(
      makeProps({
        items,
        itemDetailsById: detailsById,
        collapsedGroupIds: new SvelteSet(["b"]),
      }),
    );
    expect(
      screen.getByTestId("software-group-header-c").className,
    ).not.toContain("bg-[var(--bg-raised)]"); // idx 2
    expect(screen.getByTestId("software-group-header-d").className).toContain(
      "bg-[var(--bg-raised)]",
    ); // idx 3
  });

  it("mobile cards alternate bg: idx=0 transparent, idx=1 raised", () => {
    const itemA = makeItem("a", 1);
    const itemB = makeItem("b", 1);
    const detailsById = new SvelteMap([
      ["a", makeDetail(itemA, [makeHost("row-a1", "h-a1")])],
      ["b", makeDetail(itemB, [makeHost("row-b1", "h-b1")])],
    ]);

    render(
      SoftwareGroupList,
      makeProps({ items: [itemA, itemB], itemDetailsById: detailsById }),
    );

    const cardA = screen.getByTestId("software-group-mobile-a");
    const cardB = screen.getByTestId("software-group-mobile-b");

    expect(cardA.className).not.toContain("bg-[var(--bg-raised)]");
    expect(cardB.className).toContain("bg-[var(--bg-raised)]");
    expect(cardA.className).toContain("hover:bg-[var(--bg-hover)]");
  });
});
```

- [ ] **Step 2: Run tests — expect failures**

```bash
cd frontend && npx vitest run src/lib/components/ui/SoftwareGroupList.test.ts
```

Expected: all tests fail. `software-group-header-a` has `bg-[var(--bg-raised)]`
(currently hardcoded on all headers). `hover:bg-[var(--bg-hover)]` not present.
`software-host-row-*` has `hover:bg-[var(--bg-raised)]` (wrong token).

---

## Task 2: Add `zebraClass` Helper and `flatRowIndices` Derived

**Files:**

- Modify: `frontend/src/lib/components/ui/SoftwareGroupList.svelte` — script block only

The script block ends at line 130 (`</script>`). Insert the two additions before it.

- [ ] **Step 1: Add `zebraClass` function and `flatRowIndices` derived**

In `SoftwareGroupList.svelte`, find the closing `</script>` tag (line 130) and insert immediately before it:

```typescript
const flatRowIndices = $derived.by(() => {
  const indices = new Map<string, number>();
  let idx = 0;
  for (const item of items) {
    indices.set(`header:${item.id}`, idx++);
    if (!isSingleHostItem(item)) {
      if (itemDetailLoadingIds.has(item.id)) {
        indices.set(`loading:${item.id}`, idx++);
      } else if (
        !collapsedGroupIds.has(item.id) &&
        detailHosts(item).length > 0
      ) {
        for (const host of visibleHosts(item)) {
          indices.set(`host:${host.id}`, idx++);
        }
        if (hiddenHostCount(item) > 0) {
          indices.set(`overflow:${item.id}`, idx++);
        }
      }
    }
  }
  return indices;
});

function zebraClass(idx: number): string {
  return idx % 2 !== 0 ? "bg-[var(--bg-raised)]" : "";
}
```

The `$derived.by` block calls `isSingleHostItem`, `itemDetailLoadingIds.has`,
`collapsedGroupIds.has`, `detailHosts`, `visibleHosts`, and `hiddenHostCount` — all of
which read from the reactive `SvelteMap`/`SvelteSet` props. Svelte 5 tracks these reads
automatically inside `$derived.by`.

`zebraClass(-1)` returns `'bg-[var(--bg-raised)]'` because `-1 % 2 === -1` and
`-1 !== 0`. This is intentional: a missing key (should be impossible in practice)
renders raised rather than silently passing as transparent.

- [ ] **Step 2: Verify TypeScript compiles**

```bash
cd frontend && npx svelte-check --tsconfig tsconfig.json 2>&1 | grep -i "SoftwareGroupList" | head -20
```

Expected: no errors for `SoftwareGroupList.svelte`.

---

## Task 3: Apply Zebra to Desktop Template Rows

**Files:**

- Modify: `frontend/src/lib/components/ui/SoftwareGroupList.svelte` — desktop template section (lines ~134–417)

Four row types require changes. The outer `{#each items as item (item.id)}` stays unchanged (no `i` needed for desktop).

- [ ] **Step 1: Update the header row div**

Find (line ~135–136 inside `{#each items as item (item.id)}`):

```svelte
		{@const compactSingleHost = singleHost(item)}
		{@const isCompactSingleHost = isSingleHostItem(item)}
```

Add a third `{@const}` immediately after:

```svelte
		{@const compactSingleHost = singleHost(item)}
		{@const isCompactSingleHost = isSingleHostItem(item)}
		{@const headerRowIdx = flatRowIndices.get(`header:${item.id}`) ?? -1}
```

Then find the header `<div>` class (line ~142–145):

```svelte
			<div
				class="grid items-center gap-x-2 bg-[var(--bg-raised)] px-4 py-2.5 {canManage
					? 'grid-cols-[24px_minmax(0,1fr)_40px]'
					: 'grid-cols-[minmax(0,1fr)]'}"
				data-testid={'software-group-header-' + item.id}
			>
```

Replace with:

```svelte
			<div
				class="grid items-center gap-x-2 {zebraClass(headerRowIdx)} px-4 py-2.5 hover:bg-[var(--bg-hover)] transition-[background,border-color,color] duration-fast {canManage
					? 'grid-cols-[24px_minmax(0,1fr)_40px]'
					: 'grid-cols-[minmax(0,1fr)]'}"
				data-testid={'software-group-header-' + item.id}
			>
```

- [ ] **Step 2: Update the loading row div**

Find (line ~301–306):

```svelte
			{#if !isCompactSingleHost && itemDetailLoadingIds.has(item.id)}
				<div
					class="grid items-center gap-x-2 border-t border-[var(--border-subtle)] px-4 py-2.5 {canManage
						? 'grid-cols-[24px_minmax(0,1fr)_40px]'
						: 'grid-cols-[minmax(0,1fr)]'}"
					id={'software-group-body-' + item.id}
				>
```

Replace with:

```svelte
			{#if !isCompactSingleHost && itemDetailLoadingIds.has(item.id)}
				{@const loadingRowIdx = flatRowIndices.get(`loading:${item.id}`) ?? -1}
				<div
					class="grid items-center gap-x-2 border-t border-[var(--border-subtle)] {zebraClass(loadingRowIdx)} px-4 py-2.5 hover:bg-[var(--bg-hover)] transition-[background,border-color,color] duration-fast {canManage
						? 'grid-cols-[24px_minmax(0,1fr)_40px]'
						: 'grid-cols-[minmax(0,1fr)]'}"
					id={'software-group-body-' + item.id}
				>
```

- [ ] **Step 3: Update the host sub-row div**

Find (line ~320–325 inside `{#each visibleHosts(item) as host (host.id)}`):

```svelte
					<div
						class="grid items-center gap-x-2 border-t border-[var(--border-subtle)] bg-transparent px-4 py-2.5 transition-[background,border-color,color] duration-fast hover:bg-[var(--bg-raised)] {canManage
							? 'grid-cols-[24px_minmax(0,1fr)_40px]'
							: 'grid-cols-[minmax(0,1fr)]'}"
						data-testid={'software-host-row-' + host.id}
					>
```

Replace with (add `{@const}` before the `<div>`, inside the `{#each visibleHosts ... as host}` block):

```svelte
					{@const hostRowIdx = flatRowIndices.get(`host:${host.id}`) ?? -1}
					<div
						class="grid items-center gap-x-2 border-t border-[var(--border-subtle)] {zebraClass(hostRowIdx)} px-4 py-2.5 transition-[background,border-color,color] duration-fast hover:bg-[var(--bg-hover)] {canManage
							? 'grid-cols-[24px_minmax(0,1fr)_40px]'
							: 'grid-cols-[minmax(0,1fr)]'}"
						data-testid={'software-host-row-' + host.id}
					>
```

- [ ] **Step 4: Update the overflow row div**

Find (line ~384–389, the `{#if hiddenHostCount(item) > 0}` block):

```svelte
					<div
						class="grid items-center gap-x-2 border-t border-[var(--border-subtle)] bg-transparent px-4 py-2.5 {canManage
							? 'grid-cols-[24px_minmax(0,1fr)_40px]'
							: 'grid-cols-[minmax(0,1fr)]'}"
					>
```

Replace with:

```svelte
					{@const overflowRowIdx = flatRowIndices.get(`overflow:${item.id}`) ?? -1}
					<div
						class="grid items-center gap-x-2 border-t border-[var(--border-subtle)] {zebraClass(overflowRowIdx)} px-4 py-2.5 hover:bg-[var(--bg-hover)] transition-[background,border-color,color] duration-fast {canManage
							? 'grid-cols-[24px_minmax(0,1fr)_40px]'
							: 'grid-cols-[minmax(0,1fr)]'}"
					>
```

- [ ] **Step 5: Run the zebra tests — desktop cases should pass**

```bash
cd frontend && npx vitest run src/lib/components/ui/SoftwareGroupList.test.ts
```

Expected: all desktop tests pass (`alternates bg`, `all header rows have hover`,
`host sub-rows continue flat index`, `host sub-rows have hover`, `collapsing re-stripes`).
Mobile test still fails.

---

## Task 4: Apply Zebra to Mobile Cards

**Files:**

- Modify: `frontend/src/lib/components/ui/SoftwareGroupList.svelte` — mobile template section (lines ~419–605)

- [ ] **Step 1: Add `i` index to the mobile `{#each}` and update card div**

Find (line ~426):

```svelte
	{#each items as item (item.id)}
		{@const compactSingleHost = singleHost(item)}
		{@const isCompactSingleHost = isSingleHostItem(item)}
		<div class="px-4 py-3" data-testid={'software-group-mobile-' + item.id} role="listitem">
```

Replace with:

```svelte
	{#each items as item, i (item.id)}
		{@const compactSingleHost = singleHost(item)}
		{@const isCompactSingleHost = isSingleHostItem(item)}
		<div
			class="px-4 py-3 {i % 2 !== 0 ? 'bg-[var(--bg-raised)]' : ''} hover:bg-[var(--bg-hover)] transition-[background,border-color,color] duration-fast"
			data-testid={'software-group-mobile-' + item.id}
			role="listitem"
		>
```

- [ ] **Step 2: Run all zebra tests — all should pass**

```bash
cd frontend && npx vitest run src/lib/components/ui/SoftwareGroupList.test.ts
```

Expected: all 6 tests pass.

---

## Task 5: Run Full Suite and Commit

- [ ] **Step 1: Run the full frontend test suite**

```bash
cd frontend && npm run test
```

Expected: all tests pass. The desktop and mobile `SoftwareGroupList.test.ts` tests pass.
Existing software page tests (`software-trigger-status.test.ts`, `surface-tabs.test.ts`,
etc.) continue to pass — they only test behaviour, not CSS class values.

- [ ] **Step 2: Run TypeScript check and lint**

```bash
cd frontend && npm run check && npm run lint
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/components/ui/SoftwareGroupList.svelte \
        frontend/src/lib/components/ui/SoftwareGroupList.test.ts
git commit -m "feat(frontend): zebra rows and hover on software table"
```

---

## Self-Review

**Spec coverage check:**

| Spec requirement                                                       | Covered by                                          |
| ---------------------------------------------------------------------- | --------------------------------------------------- |
| Alternating `--bg-surface` / `--bg-raised` on all rows                 | Task 3 (desktop), Task 4 (mobile)                   |
| `--bg-hover` on hover for all rows                                     | Task 3 steps 1–4, Task 4 step 1                     |
| `transition-[background,border-color,color] duration-fast` on all rows | Task 3 steps 1–4, Task 4 step 1                     |
| Flat index map (`flatRowIndices`) for desktop                          | Task 2 step 1                                       |
| `zebraClass` plain helper function                                     | Task 2 step 1                                       |
| Header rows: remove always-on `bg-[var(--bg-raised)]`                  | Task 3 step 1                                       |
| Loading row gets zebra class                                           | Task 3 step 2                                       |
| Host sub-rows: replace `bg-transparent hover:bg-[var(--bg-raised)]`    | Task 3 step 3                                       |
| Overflow row gets zebra class                                          | Task 3 step 4                                       |
| Mobile: `{#each ... i}` index with `(item.id)` key                     | Task 4 step 1                                       |
| Re-stripe on expand/collapse                                           | Tested in Task 1; behaviour follows from flat index |
| DOM structure unchanged (wrappers, aria-controls, testids)             | No wrapper changes in any task                      |
| No new design tokens                                                   | Confirmed — only `--bg-raised`, `--bg-hover` used   |
| `?? -1` fallback, not `?? 0`                                           | Task 2 step 1                                       |
| `{@const hostRowIdx}` inside `{#each visibleHosts ... as host}`        | Task 3 step 3                                       |

All requirements covered. No gaps found.
