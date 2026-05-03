# Software Table Zebra Rows and Hover

**Date:** 2026-05-03 **Status:** Approved

## Overview

Apply the project-standard zebra-row and hover pattern to the `/software` data table
(`SoftwareGroupList.svelte`), matching `DataTable`'s visual contract exactly:

- Alternating row backgrounds: `--bg-surface` (transparent) / `--bg-raised`
- Hover: `--bg-hover` on any row
- Transition: `transition-[background,border-color,color] duration-fast` on all rows

The table mixes single-host (compact header-only) rows and multi-host (header + N sub-rows).
Expanding a multi-host item inserts new rows into the visible sequence, intentionally shifting
the stripe of everything below — this is the expected re-render behaviour. CSS `nth-child` cannot
produce this reliably across nested DOM (wrappers shift the count); a JS-derived flat index
is used instead.

---

## 1. Flat Row Index Map (Desktop)

A `$derived.by` named `flatRowIndices` is added to `SoftwareGroupList.svelte`. It walks `items`
and assigns a monotonically increasing flat index to every currently visible rendered row, storing
results in a `Map<string, number>`.

Row types and their keys:

| Row                   | Key                  |
| --------------------- | -------------------- |
| Header (all items)    | `header:{item.id}`   |
| Loading row           | `loading:{item.id}`  |
| Each host sub-row     | `host:{host.id}`     |
| Overflow "X more" row | `overflow:{item.id}` |

Logic:

```text
idx = 0
for item in items:
  indices["header:{item.id}"] = idx++
  if !isSingleHostItem(item):
    if itemDetailLoadingIds.has(item.id):
      indices["loading:{item.id}"] = idx++
    else if not collapsedGroupIds.has(item.id) and detailHosts(item).length > 0:
      for host in visibleHosts(item):   // visibleHosts returns all hosts when overflow expanded
        indices["host:{host.id}"] = idx++
      if hiddenHostCount(item) > 0:    // returns 0 when overflow expanded → no overflow row
        indices["overflow:{item.id}"] = idx++
```

When `expandedOverflowGroupIds.has(item.id)` is true, `visibleHosts` returns all hosts and
`hiddenHostCount` returns 0 — all hosts emit `host:` entries and no overflow row is emitted.

`$derived.by` reads `items`, `itemDetailsById`, `collapsedGroupIds`,
`expandedOverflowGroupIds`, and `itemDetailLoadingIds` — all reactive (`SvelteMap`/`SvelteSet`).
The map recomputes automatically on any expand, collapse, or load event.

Pagination resets the flat index to 0 for each new page. This is intentional and matches
`DataTable`'s per-page behaviour.

---

## 2. Zebra and Hover Application (Desktop)

A plain helper function determines the background class:

```text
function zebraClass(idx): idx % 2 !== 0 ? 'bg-[var(--bg-raised)]' : ''
```

Odd JS indices (1, 3, 5…) get `--bg-raised`; even JS indices (0, 2, 4…) are transparent.
This is equivalent to `DataTable`'s CSS `nth-child(even)` (1-based even = 0-based odd). The
first visible row is index 0 and renders transparent; the second is index 1 and renders raised.

Apply via `{@const}` inside the `{#each items as item}` block, before the header `<div>`. Since `$derived.by` and the template iterate the same
`items` array synchronously in Svelte 5, the key will always be present — but a fallback of
`-1` is used instead of `0` to surface any key mismatch as wrong-stripe (raised) rather than
silently passing as transparent. This works because JS `%` returns `-1` for `-1 % 2`, which
satisfies `!== 0`. Do not "fix" this to `Math.abs(idx) % 2 !== 0` — the current form is intentional:

```svelte
{@const rowIdx = flatRowIndices.get(`header:${item.id}`) ?? -1}
<div class="{zebraClass(rowIdx)} hover:bg-[var(--bg-hover)]
            transition-[background,border-color,color] duration-fast ...">
```

For host sub-rows, `{@const rowIdx = flatRowIndices.get('host:' + host.id) ?? -1}` must
appear inside the `{#each visibleHosts(item) as host}` block, not outside it.

**Performance note:** `$derived.by` traverses all items and their visible hosts on each
reactive change. This is O(n×h) where n = page size and h = visible host count. Current
pagination keeps this trivially fast. If page size grows significantly, revisit.

`visibleHosts(item)` is a pure function (slice of a reactive map read). It is called once
inside `$derived.by` and once in the template's `{#each visibleHosts(item) as host}`. Both
calls are cheap; no memoisation is needed.

**Header rows (both single-host and multi-host):**  
Remove the current always-on `bg-[var(--bg-raised)]`. Replace with zebra class from
`flatRowIndices.get('header:{id}')`.

**Loading row:**  
Apply zebra class from `flatRowIndices.get('loading:{id}')`.

**Sub-rows (host rows):**  
Replace current `bg-transparent hover:bg-[var(--bg-raised)]` with the zebra class from
`flatRowIndices.get('host:{host.id}')`, `hover:bg-[var(--bg-hover)]`, and
`transition-[background,border-color,color] duration-fast` (already present on sub-rows; verify).

**Overflow "X more" row:**  
Apply zebra class from `flatRowIndices.get('overflow:{id}')`. Add hover and transition
consistent with other rows.

No new design tokens. No new colour values. No changes to layout, spacing, or borders.

---

## 3. Mobile Treatment

Mobile cards (`sm:hidden` layout) use a simpler structure: one `<div class="px-4 py-3">` per
software item. Sub-host content is indented _inside_ the card (behind a border-l), not as sibling
rows. Expanding a mobile card does not shift peer cards.

Mobile zebra uses `{#each items as item, i (item.id)}` index directly — no flat map needed.
The existing template already keys by `(item.id)`; the `i` index is added alongside it:

```svelte
{#each items as item, i (item.id)}
  <div class="{i % 2 !== 0 ? 'bg-[var(--bg-raised)]' : ''}
              hover:bg-[var(--bg-hover)]
              transition-[background,border-color,color] duration-fast ..."
```

Mobile cards currently have no explicit hover or transition. Both are added here.

---

## 4. Unchanged

- DOM structure: outer item wrapper divs are kept. No flattening.
- `aria-controls`, `role="listitem"`, `data-testid` attributes: unchanged.
- Borders (`border-b last:border-b-0`, `border-t` on sub-rows): unchanged.
- Spacing, typography, badge rendering: unchanged.
- DataTable component: unchanged (reference only).

---

## 5. Files Changed

| File                                                      | Change                                                                                                                 |
| --------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `frontend/src/lib/components/ui/SoftwareGroupList.svelte` | Add `flatRowIndices` derived; apply zebra + hover to all desktop rows; apply index-based zebra + hover to mobile cards |

No other files require changes.

---

## 6. Testing

- Expand a multi-host item: stripe of all rows below it must shift.
- Collapse it: stripe reverts.
- Expand overflow ("X more"): overflow row disappears, new host rows appear, stripes re-number.
- Single-host items: one row each, no expand state, stripe based on position in list.
- Both dark and light themes: `--bg-raised` and `--bg-hover` tokens differ per theme; visual
  test both.
- Run existing frontend test suite: no behaviour changes expected, only visual class changes.
