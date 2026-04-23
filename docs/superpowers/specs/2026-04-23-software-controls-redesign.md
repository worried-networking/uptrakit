# Software Page Controls Redesign

**Date:** 2026-04-23
**Status:** Approved

## Problem

The Software page has two free-floating control divs that sit outside the card system used by every other page:

1. `div.mb-4.flex.items-center.justify-end.gap-2.flex-wrap` — Updates available checkbox + plugin filter select + Add Software button (lines 886–920)
2. `div.flex.justify-end` — Select all checkbox (lines 938–948)

These look visually disconnected. Other pages (History, Hosts) ground similar controls inside
`SectionCard` headers or table `<thead>` rows.

## Solution

Wrap the entire `{#if isItemsTab}` content in a single card div matching `SectionCard` visual
styles. Consolidate all controls into the card header. Eliminate both floating divs.

## File Changed

`frontend/src/routes/software/+page.svelte` only. No new components, no logic changes.

## Card Structure

```text
div.rounded-2xl.border.border-[var(--border-subtle)].bg-[var(--bg-surface)].shadow-sm
├── header.flex.items-center.justify-between.gap-4.border-b.px-5.py-3.flex-wrap
│   ├── left div.flex.items-center.gap-3.flex-wrap
│   │   ├── [if canManage] select-all checkbox + "Select all" label
│   │   ├── [if canManage] vertical divider (w-px h-4 bg-[var(--border-subtle)])
│   │   ├── [if isItemsTab] Updates available checkbox + label
│   │   └── [if pluginTypeOptions.length > 0] plugin type select
│   └── [if canManage] right: Add Software button (variant="primary" size="sm")
└── body (no wrapper padding — list fills edge-to-edge)
    ├── error state: div.p-5 wrapping Callout + retry button
    ├── loading state: p.py-8.text-center
    ├── empty state: div.px-4.py-8.text-center
    └── [else] list rows div (role="list") + TableFooterBar
```

## Specific Changes

### Remove

- The outer `<div class="mb-4 flex items-center justify-end gap-2 flex-wrap">` block and all children (lines 886–920).
- The `<div class="space-y-4" data-ui="software-route-groups">` wrapper.
- The standalone select-all `<div class="flex justify-end">` (lines 938–948).
- The inner list container's border/rounding/bg classes — the outer card provides that now.
  Keep `role="list"`, `aria-label`, and `data-ui` attributes.

### Add

- Card wrapper div around entire `{#if isItemsTab}` block.
- Card header with left-side controls and right-side action button.
- Select-all checkbox with visible "Select all" label (previously label-less).
- Vertical divider between select-all and filters (when `canManage`).
- Individual padding wrappers on error/loading/empty states (since card body has no global padding).

### Keep Unchanged

- All `onchange` / `onclick` handlers — identical to current.
- All list row markup inside the items list.
- `TableFooterBar` placement.
- `BatchActionBar`, modals, context menu, confirm dialogs — untouched.
- Ignore Rules tab, surface tabs — untouched.

## Select-all Behaviour Change

Previously: only rendered inside `{:else}` (when `items.length > 0`).

After: rendered in card header when `canManage`, always visible. Checkbox state (`checked`,
`indeterminate`) still driven by `allBatchPageSelected` and `batchSelectedIds.size`. With zero
items loaded, it is visually present but functionally inert (no items to select).

This matches the Hosts DataTable pattern where the select-all `<th>` is always present in the header row.

## Alignment with Existing Patterns

| Control                    | Before                                         | After                               |
| -------------------------- | ---------------------------------------------- | ----------------------------------- |
| Add Software button        | floating div, right-aligned                    | card header right side              |
| Updates available checkbox | floating div, right-aligned                    | card header left side               |
| Plugin filter select       | floating div, right-aligned                    | card header left side               |
| Select all checkbox        | separate floating div, right-aligned, no label | card header left side, "Select all" |

- History page: `SectionCard` with `{#snippet actions()}` for "Trigger Update", filters in card body.
- Hosts page: select-all in `<th>` header column, always visible.
- Software page (after): single card div with unified header row — same intent, adapted to the
  custom list layout.

## Out of Scope

- No changes to list row structure, batch actions, or modals.
- No changes to `SectionCard` component itself.
- No changes to other page routes.
- No responsive/mobile behaviour changes beyond what flex-wrap already provides.
