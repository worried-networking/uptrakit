# Audit Page Redesign

**Date:** 2026-04-29
**Status:** Approved

## Scope

Redesign `frontend/src/routes/audit-logs/+page.svelte` for design-language alignment and visual
quality. Also update `DataTable.svelte` to bake in the approved row style system-wide.

## Decisions

### 1. Filter Panel Layout

Replace the current 3-column `FormFieldRow` grid with a compact label-above-input grid.

**Before:** `FormFieldRow` (16rem label col + input col) nested inside `grid-cols-3`. Label
columns truncate inside narrow cells; inputs shrink to unusable widths.

**After:** `grid grid-cols-1 gap-x-4 gap-y-3 sm:grid-cols-2 lg:grid-cols-4` with each cell
containing a stacked `<label>` + `<Input>` or `<select>`. No `FormFieldRow`.

Label style: `text-xs font-medium text-[var(--text-secondary)]` with `mb-1`.

`SectionCard` title (`Filters`), description, and `actions` snippet (Apply + Clear buttons)
are preserved unchanged.

### 2. Date Field Labels

Rename `"From (RFC 3339)"` → `"From"` and `"To (RFC 3339)"` → `"To"`. RFC 3339 is an
internal implementation detail; the `datetime-local` input handles conversion transparently.

### 3. Log Scope Tab Strip

Remove the `SectionCard` wrapper around `TabStrip`. When `hasBoth` is true, render `TabStrip`
directly inside `PageShell` body with no surrounding card.

When `hasBoth` is false (user has only one permission), render nothing — no tab strip, no
explanatory card. The page `PageShell` description already contextualises the view.

Remove the single-permission `SectionCard` containing `"Showing system-level audit logs."`.

### 4. Table Row Style — DataTable-wide

**`DataTable.svelte`:** Move row background responsibility to `<tbody>` via Tailwind child
selectors:

```svelte
<tbody class="[&>tr:nth-child(even)]:bg-[var(--bg-raised)] [&>tr:hover]:bg-[var(--bg-raised)]">
```

- Even rows: `--bg-raised` at rest.
- All rows: `--bg-raised` on hover (odd rows go transparent → raised; even rows already at
  raised, hover adds no additional change — acceptable).
- Remove `even:bg-[var(--bg-raised)]` from the default `<tr>` inside `DataTable` (tbody covers
  it).
- Mobile cards container: replace per-card inline `style` with class-based equivalents using
  `[&>div:nth-child(even)]` and `[&>div:hover]`.

**Callers with custom `row` snippets** (including the audit page): remove any `even:bg-...`
from their `<tr>` elements — tbody now owns it.

### 5. Actor Column

Split the current single-column actor display into two sub-elements within the same `<td>`:

- `<PillBadge label={entry.actor_type} />` — taxonomy label for the actor type.
- Enriched display name: `entry.actor_display` if set, else `entry.actor_id` if set, else
  nothing. Rendered as `text-table-body text-[var(--text-primary)]` alongside the badge.

The existing `actorLabel()` helper is no longer needed for this column and can be removed if
unused elsewhere.

## Design Language Doc Updates

`docs/development/ui/primitives.md` — DataTable visual rules already updated in this session
to document two supported row highlight modes:

- **Hover-only** — rows transparent at rest, highlighted on hover.
- **Zebra + hover** — alternating fill plus hover on all rows. Preferred for high-density
  read-only log tables.

## Out of Scope

- Auto-apply filters on change (requires debounce; not requested).
- Expandable row detail view.
- Target column enrichment (no `PillBadge` for target type — target values are IDs, not
  taxonomy labels).
- Any backend or API changes.
