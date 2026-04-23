# Token Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`
**Audit source:** `docs/superpowers/specs/2026-04-23-design-alignment-gaps.md` §Category A + §Category B

Eliminates all remaining Skeleton Labs preset classes (`preset-filled-*`, `preset-tonal-*`,
`btn`, `btn-sm`, `badge preset-*`) and Skeleton color tokens (`text-surface-*`,
`bg-surface-*`, `border-surface-*`, `divide-surface-*`, `text-error-500`,
`text-success-500`, `bg-primary-*`, `text-primary-*`, `border-primary-*`) from
`lib/components/` and all route files, replacing them with design-token equivalents
from `frontend/src/theme/tokens.ts`.

---

## Overview

The eight-wave design-language rollout covered all Button/primitive migrations in route
files. It did not cover shared modals, surface components, or every scattered inline
color reference. This spec completes the sweep.

No behaviour changes. Pure class-substitution.

---

## Design decisions

**Q1 — Token mapping authority.**

- (chosen) `frontend/src/theme/tokens.ts` is the canonical token registry. Every
  Skeleton class must map to a token from that file.
- Rejected: leaving any Skeleton class in place. They break in dark mode differently
  than design tokens and couple components to Skeleton's theme engine.

**Q2 — `preset-filled-error-500` / `preset-tonal-surface` on `<aside>` blocks.**

- (chosen) Replace with `<Callout>` component (`tone="danger"`, `tone="info"`, etc.)
  from `$lib/components/ui`. `Callout` already uses design tokens internally.
- Rejected: inline token classes on `<aside>`. `Callout` encapsulates the visual
  contract; raw aside recreates it inconsistently.

**Q3 — `badge preset-tonal-*` on status indicators.**

- (chosen) Replace with `<StatusBadge>` from `$lib/components/ui` using `tone`
  prop (`tone="danger"`, `tone="warning"`, `tone="success"`, `tone="info"`).
- Rejected: inline badge token classes. `StatusBadge` is the canonical badge primitive.

**Q4 — Raw `<a class="btn btn-sm preset-tonal">` and raw `<button>` styled as links.**

- (chosen) Replace with `<Button variant="ghost">` (or `<Button variant="ghost" href=...>`
  for anchor semantics).
- Rejected: keep raw elements. Bypasses `Button`'s loading/disabled/aria contract.

**Q5 — `card` Skeleton utility class.**

- (chosen) Replace with `<SectionCard>` where a titled section card is needed, or with
  `bg-[var(--bg-surface)] rounded-[3px] border border-[var(--border-subtle)]` for inline
  use. Do not use the Skeleton `card` utility.
- Rejected: keep `card`. Skeleton-era utility; not part of the design token system.

---

## Token substitution table

| Skeleton class | Design token replacement |
| --- | --- |
| `text-surface-400`, `text-surface-500` | `text-[var(--text-muted)]` |
| `text-surface-600`, `text-surface-700` | `text-[var(--text-secondary)]` |
| `text-surface-600 dark:text-surface-400` | `text-[var(--text-secondary)]` (dark variant dropped — `--text-secondary` is theme-aware) |
| `text-surface-900 dark:text-surface-100` | `text-[var(--text-primary)]` |
| `bg-surface-50 dark:bg-surface-900` | `bg-[var(--bg-surface)]` |
| `bg-surface-100 dark:bg-surface-800` | `bg-[var(--bg-raised)]` |
| `bg-surface-100/800` (slash notation) | `bg-[var(--bg-raised)]` |
| `border-surface-200`, `border-surface-300` | `border-[var(--border-default)]` |
| `dark:border-surface-600`, `dark:border-surface-700` | (drop — `--border-default` handles both themes) |
| `divide-surface-200 dark:divide-surface-700` | `divide-[var(--border-subtle)]` |
| `hover:bg-surface-100-800-token` | `hover:bg-[var(--bg-hover)]` |
| `rounded-container-token` | `rounded-[3px]` |
| `border-surface-300-600-token` | `border-[var(--border-default)]` |
| `text-error-500` | `text-[var(--color-error)]` |
| `text-success-500` | `text-[var(--color-success)]` |
| `border-t-primary-500` | `border-t-[var(--accent)]` |
| `border-primary-500` | `border-[var(--accent)]` |
| `bg-primary-100 dark:bg-primary-900/40` | `bg-[rgba(var(--accent-rgb),0.12)]` |
| `text-primary-700 dark:text-primary-200` | `text-[var(--accent-bright)]` |
| `preset-filled-error-500` on `<aside>` | `<Callout tone="danger">` |
| `preset-tonal-surface` on `<aside>` | `<Callout tone="info">` (or omit Callout and use `bg-[var(--bg-raised)]`) |
| `preset-filled-warning-500` | `<Callout tone="warning">` |
| `preset-filled-surface-400-600` | `bg-[var(--bg-raised)]` |
| `badge preset-tonal` | `<StatusBadge tone="info">` |
| `badge preset-tonal-warning` | `<StatusBadge tone="warning">` |
| `badge preset-tonal-error` | `<StatusBadge tone="danger">` |
| `badge preset-tonal-surface` | `<StatusBadge tone="info">` |
| `badge preset-filled-primary-500` | `<StatusBadge tone="info">` |
| `card preset-tonal-primary` | `bg-[rgba(var(--accent-rgb),0.08)] rounded-[3px] border border-[rgba(var(--accent-rgb),0.15)] p-4` |
| `card preset-tonal-surface` | `bg-[var(--bg-raised)] rounded-[3px] border border-[var(--border-subtle)]` |
| `card` (Skeleton utility) | `<SectionCard>` or inline token classes |
| `btn btn-sm preset-tonal` on `<a>` | `<Button variant="ghost" size="sm" href=...>` |
| `hover:text-surface-700 dark:hover:text-surface-300` on raw `<button>` | migrate element to `<Button variant="ghost">` |

---

## Scope

### `lib/components/` files

| File | Key violations |
| --- | --- |
| `ToastNotifications.svelte:388` | `<a class="btn btn-sm preset-tonal">` → `<Button variant="ghost" size="sm" href=...>` |
| `BatchActionBar.svelte:105,110,112,119,155` | `bg-surface-*`, `text-surface-500`, `border-t-primary-500`; raw `<button>` as link → `<Button variant="ghost">` |
| `BatchResultDialog.svelte:21,29,37,38,39` | `text-success-500`, `text-error-500`, `bg-surface-100/800`, `text-surface-500` |
| `Modal.svelte:22` | `bg-surface-50 dark:bg-surface-900` |
| `CheckboxList.svelte:34,38,52,59` | `rounded-container-token`, `border-surface-*-token`, `hover:bg-surface-*-token`, `text-surface-500` |
| `AddSoftwareModal.svelte:59` | `text-surface-500` |
| `AssignToHostModal.svelte:261,270,274,375,501` | `text-surface-*`; `preset-filled-error-500` → `<Callout tone="danger">`; `preset-tonal-surface` → `<Callout tone="info">` |
| `EditHostAssignmentModal.svelte:685,699,838,883,965,1015,1157,1205` | `text-surface-*`, `bg-surface-*`, `border-surface-*`, `preset-filled-error-500` (×3 at 685,838,965), `badge preset-tonal` (×2 at 699,1015), `badge preset-tonal-warning` (×2 at 883,1205); error paragraphs 838,965,1157 |
| `SoftwareMergeWizard.svelte` | `bg-primary-*`, `text-primary-*`, `bg-surface-*`, `text-surface-*`; `badge preset-*` (×4); `card preset-tonal-primary` |
| `surfaces/SurfaceKeyValue.svelte:16,20` | `text-surface-500`, `divide-surface-200/700` |
| `surfaces/SurfaceWorkflow.svelte` | `bg-primary-*`, `text-primary-*`, `border-primary-500` (incl. :471), `text-surface-*`, `card preset-tonal-surface` at :420 → `bg-[var(--bg-raised)] rounded-[3px] border border-[var(--border-subtle)]` (no padding — this card has its own internal layout) |

> **Cross-spec note:** `SurfaceSlot.svelte:38,40` uses Skeleton `card` utility and `h3`
> typography class. Those are geometry/typography deviations handled by
> `2026-04-23-primitive-conformance-design.md`. Do not re-migrate here.

### Route files

| File | Key violations |
| --- | --- |
| `routes/+page.svelte:161` | `text-surface-500` |
| `routes/surfaces/[id]/+page.svelte:54,62` | `text-surface-500` |
| `routes/history/+page.svelte:682,711` | `bg-surface-50/900`, `text-error-500` |
| `routes/audit-logs/+page.svelte:216` | `text-surface-500` |
| `routes/profile/+page.svelte:182` | `bg-surface-100/800` |
| `routes/hosts/+page.svelte:437,563` | `text-surface-400`, `text-error-500` |
| `routes/hosts/[id]/+page.svelte:584,624,643` | `<a class="btn btn-sm preset-tonal">` → `<Button variant="ghost" size="sm" href=...>`; `preset-tonal-surface`; `preset-tonal` badge → `<StatusBadge>` |
| `routes/host-tags/+page.svelte:480` | `text-surface-400` |
| `routes/software/[id]/+page.svelte:848` | `badge preset-tonal` → `<StatusBadge>` |
| `routes/settings/GlobalSettingsTab.svelte` | `text-surface-*`, `bg-surface-100-900`, `preset-filled-warning-500` → `<Callout tone="warning">`, `preset-filled-surface-400-600` |
| `routes/settings/PluginConfigsTab.svelte:1276,1288` | `text-surface-500`, `text-error-500` |
| `routes/settings/SchedulerTab.svelte:124,141` | `text-surface-500`, `text-error-500` |
| `routes/settings/SystemServicesSettings.svelte:200` | `text-surface-600 dark:text-surface-400` |
| `routes/settings/+page.svelte:250` | `text-surface-600` |

---

## Migration pattern

Per file:

1. Grep for Skeleton classes:
   `preset-\|text-surface-\|bg-surface-\|border-surface-\|divide-surface-\|bg-primary-\|text-primary-\|`
   `border-primary-\|text-error-500\|text-success-500\|btn btn-sm\|badge preset`
2. Apply substitution table above.
3. For `preset-filled-*` on `<aside>`: import `Callout` from `$lib/components/ui`;
   replace `<aside class="... preset-filled-error-500">` with `<Callout tone="danger">`.
4. For `badge preset-tonal-*`: import `StatusBadge` from `$lib/components/ui`;
   replace `<span class="badge preset-tonal-...">text</span>` with `<StatusBadge tone="...">text</StatusBadge>`.
5. For raw `<a class="btn btn-sm preset-tonal">`: import `Button`;
   replace with `<Button variant="ghost" size="sm" href=...>`.
6. Run `cd frontend && npm run check && npm run test`.

Work files independently — no ordering constraints within this spec.

**Commit strategy:** one commit per logical file group (e.g. one commit for lib/components,
one for route files) to keep diffs reviewable.

---

## Testing

- `npm run check` — zero new type errors.
- `npm run test` — all 811 Vitest tests pass.
- Dark-mode smoke: open at least one page containing a migrated `text-surface-*` class
  in both light and dark mode; confirm text color changes correctly (muted vs. readable).
- Playwright parity suite: pure class swaps (`text-surface-*`, `bg-surface-*` → token
  equivalents) produce no visual diff — baselines should stay unchanged. Component
  substitutions (`<aside>` → `<Callout>`, `<span class="badge">` → `<StatusBadge>`,
  `<a class="btn">` → `<Button>`) **will** produce visual diffs — delete and regenerate
  affected snapshots after verifying the new output matches the spec intent.

## Rollout

Two PRs (or one large PR) titled:

- `"fix(frontend): replace Skeleton preset/surface classes in lib/components with design tokens"`
- `"fix(frontend): replace residual Skeleton color tokens in routes with design tokens"`

No dependency on `2026-04-23-primitive-conformance-design.md` or
`2026-04-23-form-primitive-adoption-design.md`. Can land in any order.
