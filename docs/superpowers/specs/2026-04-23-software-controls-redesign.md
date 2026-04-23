# Software Page Controls Redesign

**Date:** 2026-04-23
**Status:** Approved

## Problem

The Software page has two free-floating control divs that sit outside the card system used by every other page:

1. `div.mb-4.flex.items-center.justify-end.gap-2.flex-wrap` — sits **outside** `{#if isItemsTab}`;
   each child (`Updates available`, plugin select, Add Software button) has its own inner
   `{#if isItemsTab}` or `{#if isItemsTab && canManage}` guard.
2. `div.flex.justify-end` — Select all checkbox, inside `{#if isItemsTab}` → `{:else}` (only
   rendered when `items.length > 0`).

These look visually disconnected. Other pages (History, Hosts) ground similar controls inside
`SectionCard` headers or table `<thead>` rows.

## Solution

Wrap the list section — from the start of the `{#if isItemsTab}` block through `TableFooterBar`
— in a single card div matching `SectionCard` visual styles. Consolidate all controls into the
card header. Eliminate both floating divs. `BatchActionBar`, dialogs, and modals that follow the
list stay outside the card but remain inside `{#if isItemsTab}`.

## File Changed

`frontend/src/routes/software/+page.svelte` only. No new components, no logic changes.

## Card Structure

```text
{#if isItemsTab}                          ← existing guard, unchanged
  div.rounded-2xl.border.bg-[var(--bg-surface)].shadow-sm  [data-ui="software-route-groups"]
  ├── header.flex.flex-col.gap-3.border-b.px-5.py-3.md:flex-row.md:items-center.md:justify-between
  │   ├── left div.flex.flex-wrap.items-center.gap-3
  │   │   ├── [if canManage] select-all checkbox + "Select all" label
  │   │   ├── [if canManage] vertical divider (w-px h-4 bg-[var(--border-subtle)])
  │   │   ├── Updates available checkbox + label  (no isItemsTab guard — already inside it)
  │   │   └── [if pluginTypeOptions.length > 0] plugin type select
  │   └── [if canManage] div.shrink-0: Add Software button (variant="primary" size="sm")
  └── body (no wrapper padding — list fills edge-to-edge)
      ├── error state:   div.p-5 wrapping Callout + retry button
      ├── loading state: p.px-5.py-8.text-center
      ├── empty state:   div.px-4.py-8.text-center (remove border/bg — card provides that)
      └── list rows div [role="list"] [data-ui="software-group-list"] + TableFooterBar
{/if}
← BatchActionBar, ConfirmDialogs, ContextMenuShell, AssignToHostModal,
  SoftwareMergeWizard, AddSoftwareModal, BatchResultDialog stay here
  (inside {#if isItemsTab}, outside the card div)
← updateModalItem ModalShell, editItem ModalShell are after </PageShell>
  (outside {#if isItemsTab} entirely) — untouched
```

## Specific Changes

### Remove

- The outer `<div class="mb-4 flex items-center justify-end gap-2 flex-wrap">` block and all
  children (lines 886–920). Note: this div sits **outside** `{#if isItemsTab}`; remove the entire
  div including its inner `{#if isItemsTab}` guards.
- The `<div class="space-y-4" data-ui="software-route-groups">` wrapper — `data-ui` attribute
  **moves to the new card wrapper div**.
- The standalone select-all `<div class="flex justify-end">` (lines 938–948).
- The inner list container's `overflow-hidden rounded-[4px] border border-[var(--border-subtle)]
  bg-[var(--bg-surface)]` classes — the outer card provides that. Keep `role="list"`,
  `aria-label`, and `data-ui="software-group-list"` attributes.
- The empty state's inner `rounded-[4px] border … bg-[var(--bg-surface)]` — card provides it.

### Add

- Card wrapper div as first child inside `{#if isItemsTab}`, wrapping the
  error/loading/list content through `TableFooterBar`.
- Card header with left-side controls and right-side action button.
- Select-all checkbox with visible "Select all" label (previously label-less) and three visually
  distinct states: empty (nothing selected), indeterminate dash (some selected), checked (all selected).
- Vertical divider between select-all and filters (when `canManage`).
- Individual padding wrappers on error/loading/empty states (since card body has no global padding).

### Keep Unchanged

- All `onchange` / `onclick` handlers — identical to current.
- All list row markup inside the items list.
- `TableFooterBar` placement.
- `BatchActionBar`, `ContextMenuShell`, `ConfirmDialog` (batch + delete), `BatchResultDialog`,
  `AssignToHostModal`, `SoftwareMergeWizard`, `AddSoftwareModal` — remain inside
  `{#if isItemsTab}` but outside the card div. Untouched.
- `updateModalItem` ModalShell, `editItem` ModalShell — already outside `{#if isItemsTab}`
  (after `</PageShell>`). Untouched.
- Ignore Rules tab, surface tabs — untouched.

## Select-all Behaviour Change

Previously: only rendered inside `{:else}` (when `items.length > 0`).

After: rendered in card header when `canManage`, always visible. With zero items loaded it is
visually present but functionally inert (no items to select).

This matches the Hosts DataTable pattern where the select-all `<th>` is always present in the header row.

### Three Visual States

The checkbox has three visually distinct states driven by existing derived values:

| State | Condition | `checked` | `indeterminate` | Visual |
| ----- | --------- | --------- | --------------- | ------ |
| Nothing selected | `batchSelectedIds.size === 0` | `false` | `false` | empty box |
| Some selected | `batchSelectedIds.size > 0 && !allBatchPageSelected` | `false` | `true` | dash/minus |
| All selected | `allBatchPageSelected` | `true` | `false` | filled/checked |

No new logic required — `allBatchPageSelected` (`items.every(i => batchSelectedIds.has(i.id))`)
and `batchSelectedIds.size` already provide the necessary signals. The `Checkbox` primitive
renders the indeterminate dash natively when `indeterminate={true}` and `checked={false}`.

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
