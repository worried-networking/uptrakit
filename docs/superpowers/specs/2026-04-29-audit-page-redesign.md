# Audit Page Redesign

**Date:** 2026-04-29
**Status:** Approved

## Scope

Redesign `frontend/src/routes/audit-logs/+page.svelte` for design-language alignment and visual
quality. Also update `DataTable.svelte` to bake in the approved row style system-wide, which
requires a targeted cleanup sweep across all `{#snippet row(...)}` blocks inside DataTable callers.

## Decisions

### 1. Filter Panel Layout

Replace the current 3-column `FormFieldRow` grid with a compact label-above-input grid.

**Before:** `FormFieldRow` (16rem label col + input col) nested inside `grid-cols-3`. Label
columns truncate inside narrow cells; inputs shrink to unusable widths.

**After:** `grid grid-cols-1 gap-x-4 gap-y-3 sm:grid-cols-2 lg:grid-cols-4` with each cell
containing a stacked `<label>` + `<Input>` or `<select>`. No `FormFieldRow`.

Label style: `text-xs font-medium text-[var(--text-secondary)]` with `mb-1`.

Field order at `lg:grid-cols-4`: Action, Outcome, Actor Type, Target Type, Target ID, From, To.
This gives a 4+3 layout — one empty cell at the end. The empty cell is intentional; Apply and
Clear buttons remain in the `SectionCard` header `actions` snippet, not in the grid.

`SectionCard` title (`Filters`), description, and `actions` snippet (Apply + Clear buttons)
are preserved unchanged.

**Accessibility note:** this pattern does not use `FormFieldRow`, which enforced `htmlFor`/`id`
linkage structurally. The implementer must manually pair each `<label for="...">` with the
correct `<Input id="...">`. Future reuse of this pattern carries the same responsibility.

### 2. Date Field Labels

Rename `"From (RFC 3339)"` → `"From"` and `"To (RFC 3339)"` → `"To"`. RFC 3339 is an
internal implementation detail; the `datetime-local` input handles conversion transparently.

### 3. Log Scope Tab Strip

Remove the `SectionCard` wrapper around `TabStrip`. When `hasBoth` is true, render `TabStrip`
directly inside `PageShell` body with no surrounding card.

`PageShell` uses `space-y-6` between children. The bare `TabStrip` receives 24px separation
from the page header above and from the Filters card below — sufficient rhythm, no orphaning.

When `hasBoth` is false, two sub-cases:

- **Tenant-only** (`canViewTenant && !canViewSystem`): no tab strip, no card. Already the
  current behaviour — no change needed for this branch.
- **System-only** (`canViewSystem && !canViewTenant`): remove the existing `SectionCard`
  containing `"Showing system-level audit logs."`. Render nothing — the `PageShell` description
  already contextualises the view.

### 4. Table Row Style — DataTable-wide

#### 4a. Fixed row style

`DataTable.svelte` always applies zebra striping and hover highlight. No prop, no opt-out.
Add the following class to `<tbody>`:

```html
<tbody class="[&>tr:nth-child(even)]:bg-[var(--bg-raised)] [&>tr:hover]:bg-[var(--bg-hover)]">
```

Token roles:

- Odd rows at rest: transparent — inherits `--bg-surface` from the card container.
- Even rows at rest: `--bg-raised` (dark `#18181b`, light `#f1f5f9`).
- Any row on hover: `--bg-hover` (dark `#1e1e22`, light `#eef1f5`) — one step above `--bg-raised`,
  visually distinct from both odd and even rest states in both themes.

#### 4b. `<tbody>` owns row backgrounds

Remove `even:bg-[var(--bg-raised)]` from the default auto-rendered `<tr>` inside `DataTable`.
**This removal is coupled to the tbody class addition above — do not apply one without the
other.** The `<tr>` removal is safe only because tbody always covers even-row fills.
Applying the removal without the tbody class breaks zebra for all non-custom-snippet callers.

Mobile cards auto-generated path: replace the per-card inline `style={index % 2 === 1 ? ...}`
expression with `[&>div:nth-child(even)]:bg-[var(--bg-raised)] [&>div:hover]:bg-[var(--bg-hover)]`
on the container div. The `mobileRow` custom snippet path is caller-controlled — do not touch it.

#### 4c. Caller sweep

Sweep all snippets passed as the `row` prop to any `<DataTable ...>` call — whether written
inline between `<DataTable>` tags or defined above the call and referenced by name (e.g.
`SurfaceTable`'s `entityLinkRow`). Within those snippets, remove `even:bg-[var(--bg-raised)]`
from `<tr>` elements. Scope guard: any `<tr>` not in a snippet passed as `row` to DataTable
is out of scope regardless of file location.

**Special cases:**

- `hosts/+page.svelte`: the row snippet carries both `even:bg-[var(--bg-raised)]` and
  `hover:bg-[var(--bg-raised)]` on the `<tr>`. Remove both — tbody now owns both.
- `SurfaceTable.svelte` has two `<tr>` sites: (1) the `entityLinkRow` snippet passed as `row`
  to DataTable — sweep this one, remove `even:bg-[var(--bg-raised)]`; (2) any `<tr>` inside
  SurfaceTable's own `<tbody>` management outside a DataTable call — leave that alone.
- `last:border-b-0` stays on caller `<tr>` elements — tbody does not own border suppression.
  Do not remove `last:border-b-0` during the sweep.

### 5. Actor Column

Split the current single-column actor display into two sub-elements within the same `<td>`.

`<td>` layout: `flex items-center gap-2`. The `items-center` aligns badge and text vertically.

Elements:

- `<PillBadge label={entry.actor_type} />` — taxonomy label for the actor type.
- Enriched display name: `entry.actor_display` if set, else `entry.actor_id` if set, else
  nothing. Rendered as `text-table-body text-[var(--text-primary)]`.
- For actors where both `actor_display` and `actor_id` are absent (e.g. `actor_type = "system"`),
  the `PillBadge` alone is sufficient. A lone badge without trailing text is intentional.
- Note: `PillBadge` adds a bordered pill for every row. For the dominant `user` / `api_token`
  cases where a display name exists, the badge is redundant but provides consistent column
  structure and scannability by type. This is an accepted trade-off.

`<td>` `title` attribute: `entry.actor_display ?? entry.actor_id ?? entry.actor_type`.
`actor_type` is always present (required field), so this chain never produces `undefined`.

The existing `actorLabel()` helper is superseded. Remove it if no other call site references it.

Add `PillBadge` to the existing `import { ..., PillBadge } from '$lib/components/ui'` in
the audit page.

## Design Language Doc Updates

`docs/development/ui/primitives.md` — DataTable visual rules updated to document the single
canonical row highlight mode: zebra striping (`--bg-raised` for even rows) plus hover highlight
(`--bg-hover` for any hovered row). Both always applied — not configurable.

## Out of Scope

- Auto-apply filters on change (requires debounce; not requested).
- Expandable row detail view.
- Target column enrichment (no `PillBadge` for target type — target values are IDs, not
  taxonomy labels).
- Any backend or API changes.
