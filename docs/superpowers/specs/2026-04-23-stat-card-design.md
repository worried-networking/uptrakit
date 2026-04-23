# StatCard Component Design

## Goal

Extract the repeated stat-card pattern from the dashboard home page into a shared `StatCard`
primitive, migrate all four existing call-sites to use it, and document it in `primitives.md`.

## Background

The dashboard (`frontend/src/routes/+page.svelte`) renders a 4-column summary grid. Each cell
is a navigable `<a>` card with three stacked lines: a small uppercase label, a large bold value,
and a short sub-label. The markup is duplicated verbatim four times. The current inline pattern
also omits the standard transition and focus ring required by the design token spec.

## Component Interface

### File

`frontend/src/lib/components/ui/StatCard.svelte`

### Tone type

```typescript
export type StatCardTone = 'muted' | 'success' | 'info' | 'warning' | 'danger';
```

Tone-to-token mapping:

| Tone | CSS token |
| --- | --- |
| `muted` | `--text-muted` |
| `success` | `--color-success` |
| `info` | `--color-info` |
| `warning` | `--color-warning` |
| `danger` | `--color-error` |

### Props

```typescript
let {
  href,
  label,
  value,
  valueTone = 'muted',
  subLabel,
}: {
  href: string;
  label: string;
  value: string | number;
  valueTone?: StatCardTone;
  subLabel: string;
} = $props();
```

All props except `valueTone` are required. `href` is always present — StatCard is always
navigable. Non-link stat display is out of scope (YAGNI).

### Rendered structure

A `toneTokens` map in the script block resolves the tone to the CSS custom property name:

```typescript
const toneTokens: Record<StatCardTone, string> = {
  muted:   '--text-muted',
  success: '--color-success',
  info:    '--color-info',
  warning: '--color-warning',
  danger:  '--color-error',
};
const toneToken = toneTokens[valueTone];
```

Template:

```svelte
<a
  {href}
  class="block rounded-[3px] border border-[var(--border-subtle)] bg-[var(--bg-surface)]
         px-4 py-4 transition-[background,border-color,color] duration-[120ms]
         hover:border-[var(--accent)]
         focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]"
  data-ui="stat-card"
  data-tone={valueTone}
>
  <p class="text-[7.5px] font-semibold uppercase tracking-[0.08em] text-[var(--text-secondary)]">
    {label}
  </p>
  <p class="mt-1 text-[14px] font-bold" style="color: var({toneToken})">
    {value}
  </p>
  <p class="mt-1 text-[10px] text-[var(--text-secondary)]">
    {subLabel}
  </p>
</a>
```

The value color is applied via `style` using the resolved `toneToken` string (e.g.
`--color-success`). Tailwind arbitrary value classes referencing a runtime variable are not
suitable here because the tone is dynamic.

`block` on the `<a>` ensures full-width fill in grid cells and correct top-to-bottom stacking of
the three child paragraphs regardless of container context. Without it an `<a>` defaults to
`inline` and would collapse to content width outside a grid or flex parent.

### Accessibility

The `<a>` element's text content (label + value + subLabel) is fully visible and meaningful to
screen readers. No additional `aria-label` is required — announced content reads naturally, e.g.
"Hosts 42 registered hosts, link." Do not add an `aria-label` that duplicates the visible text.

### Transitions and focus ring

The inline pattern that existed before this component was missing:

- `transition-[background,border-color,color] duration-[120ms]` — required by `tokens.md` for
  all interactive controls.
- `focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]` —
  required focus ring.

`StatCard` adds both. This is a corrective fix, not a new behaviour.

## Barrel Export

Add to `frontend/src/lib/components/ui/index.ts` after the `DataTable` export line (before
`ContextMenuItem`):

```typescript
export { default as StatCard } from './StatCard.svelte';
export type { StatCardTone } from './StatCard.svelte';
```

## Home Page Migration

File: `frontend/src/routes/+page.svelte`

**Script block:** add `StatCard` to the existing barrel import:

```typescript
import { Callout, DataTable, PageShell, SectionCard, StatCard, StatusBadge } from '$lib/components/ui';
```

**Template:** replace the four inline `<a>` blocks with `StatCard` components. Conditional
sub-label strings are pre-computed as inline ternaries so `subLabel` always receives a plain
`string`. The surrounding grid wrapper and permission guards remain unchanged.

```svelte
{#if canViewHosts}
  <StatCard
    href="/hosts"
    label="Hosts"
    value={totalHosts}
    valueTone="success"
    subLabel="registered hosts"
  />
{/if}

{#if canViewAgents}
  <StatCard
    href="/services"
    label="Services"
    value={totalServices}
    subLabel={pendingServices > 0 ? `${pendingServices} pending approval` : 'No pending approvals'}
  />
{/if}

{#if canViewSoftware}
  <StatCard
    href="/software"
    label="Updates pending"
    value={pendingUpdateCount}
    valueTone="info"
    subLabel={`${totalSoftwareItems + unfeaturedSoftwareCount} tracked software items`}
  />

  <StatCard
    href="/history?status=failed"
    label="Errors"
    value={failedUpdates}
    valueTone="danger"
    subLabel={failedUpdates > 0 ? `${failedUpdates} failed updates in recent activity` : 'No recent update failures'}
  />
{/if}
```

`valueTone` defaults to `'muted'` so the Services card (which renders its count in
`--text-muted`) requires no explicit tone prop.

## Documentation Update

### primitives.md

Replace the existing `## Stat Card` section (currently at the bottom of the file before
`## Notes`) with a full component section. Keep the `## Stat Card` heading in place — do not
move it into another category. The replacement must cover:

- Import path and barrel export name
- Props table with types and defaults
- Tone-to-token table
- Usage example showing a single `<StatCard>` call with all props

Use the same format as other primitives in the file. Concretely, the replacement section must
contain, in order:

1. A file-path comment and type export block:

```typescript
// frontend/src/lib/components/ui/StatCard.svelte
export type StatCardTone = 'muted' | 'success' | 'info' | 'warning' | 'danger';

let {
  href,
  label,
  value,
  valueTone = 'muted',
  subLabel,
}: {
  href: string;
  label: string;
  value: string | number;
  valueTone?: StatCardTone;
  subLabel: string;
} = $props();
```

1. A tone table (same format as the tone tables in `StatusBadge` and `ActionBadge` sections).

1. A Svelte usage example showing a single call with all props.

1. A rules list covering: `3px` radius, surface background, subtle border, hover border promotes
   to `--accent`, value color follows `valueTone`, always an `<a>` with required `href`, grid
   wrapper context (`grid-cols-1 sm:grid-cols-2 lg:grid-cols-4` inside a `SectionCard`).

### README.md

Add `StatCard` to the quick reference table in `docs/development/ui/README.md`. Insert the row
after the `Data listing with pagination` row:

```markdown
| Navigable summary stat card | `StatCard` |
```

## Out of Scope

- Non-link (static `<div>`) variant — YAGNI.
- Token rename (`--color-error` → `--color-danger`) — separate spec.
- Visual regression parity fixture — the component is used only on the dashboard, which has no
  existing parity capture. No new fixture is required for this change.
