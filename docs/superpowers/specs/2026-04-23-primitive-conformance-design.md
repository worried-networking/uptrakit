# Primitive Conformance — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`
**Audit source:** `docs/superpowers/specs/2026-04-23-design-alignment-gaps.md` §Category D

Fixes spec-geometry and spec-typography deviations in the `ui/` primitive layer and
surface component layer. These primitives are consumed everywhere — fixing them here
propagates correct values across all consumers automatically.

---

## Overview

The `lib/components/ui/` primitives and several surface/shared components were authored
with border-radius and typography values that do not match the parent spec. Because the
spec is authoritative, every deviation must be corrected at the primitive level.

Category: no new features, no behaviour changes — geometry and typography corrections
only.

---

## Design decisions

**Q1 — Border radius authority.**

- Options:
  - (chosen) Parent spec §2.3 is canonical: cards/table wrappers/buttons = `rounded-[3px]`;
    page panels/modals/drawers = `rounded-[4px]`. Apply uniformly.
  - Leave primitives at current values. Rejected — spec is authoritative per user
    instruction; current values (`rounded-2xl`, `rounded-xl`, `rounded-lg`) deviate
    significantly and produce visual inconsistency.
- Reasoning: small radius is the deliberate terminal-UI aesthetic specified in §2.3;
  16px/12px/8px rounding undermines that contract.
- Classification decisions:
  - `Callout` = **panel** (4px): it is a bordered status/alert block, semantically an
    `<aside>`, not a data container. Callouts span horizontal width and act as notices
    similar to info panels, not like discrete data cards. §2.3 panel category applies.
  - `EmptyState`, `SectionCard` = **card** (3px): discrete content containers with
    optional header/body separation. §2.3 card category applies.
  - `ProviderSelector` select = **button/card** (3px): form control radius is governed by
    §2.3 button/card rule (same 3px). The gaps audit cited §4.10 (Form Validation) as the
    authority but §4.10 contains no radius specification — §2.3 is the correct citation.

**Q2 — TabStrip active state.**

- Options:
  - (chosen) Spec §4.11: active tab = `bg-[rgba(var(--accent-rgb),0.12)]` background +
    `text-[var(--accent-bright)]` text. No solid fill.
  - Keep current solid `bg-[var(--accent)]` fill. Rejected — spec is explicit; solid fill
    is high-contrast and diverges from the tint treatment used elsewhere for accent
    selections.
- Reasoning: tint treatment is consistent with nav active states and host-tag selections
  throughout the app.

**Q3 — Typography (PageShell h1, Skeleton `h3`/`h4` utilities).**

- Options:
  - (chosen) Parent spec §2.4 is canonical. Replace Skeleton utility classes
    (`h3`, `h4`) and oversized `text-3xl` with explicit size/weight classes per §2.4.
  - Leave current values. Rejected — Skeleton typography utilities are theme-coupled
    and not part of the design token system.
- Reasoning: explicit `text-[Npx] font-weight` classes are portable and theme-agnostic.
- Exact values from §2.4: h1 = `text-[20px] font-bold text-[var(--text-primary)]`;
  h2 = `text-[16px] font-bold text-[var(--text-primary)]`;
  h3 = `text-[13px] font-bold text-[var(--text-primary)]`.

**Q5 — h4 typography (Skeleton `h4` utility — not defined in §2.4).**

- (chosen) §2.4 defines h1/h2/h3 only. No h4 level exists in the spec. Replace
  Skeleton `h4` utility class with h3-equivalent values:
  `text-[13px] font-bold text-[var(--text-primary)]`.
- Rejected: leaving `h4` Skeleton utility or inventing a fourth level. The spec does
  not define h4; mapping to h3 is the minimal correct substitution.
- Reasoning: `SoftwareMergeWizard` uses `h4` for sub-section headings; h3 values
  produce the correct compact terminal aesthetic without adding a non-spec level.

**Q4 — DataTable cell padding.**

- Options:
  - (chosen) Spec §4.12 mandates 10px horizontal cell padding. Replace `px-4` (16px)
    with `px-[10px]`.
  - Keep `px-4`. Rejected — spec is authoritative; the 6px discrepancy affects all
    table layouts.
- Reasoning: tighter 10px padding is consistent with the compact terminal-UI aesthetic.

---

## Goals

1. Correct border radius on `Callout`, `EmptyState`, `SectionCard`, `TabStrip`,
   `ProviderSelector` to spec §2.3 values.
2. Fix `TabStrip` active state to tint + accent-bright text per spec §4.11.
3. Fix `TabStrip` outer container radius.
4. Fix `PageShell` h1 typography to spec §2.4.
5. Replace Skeleton `h3`/`h4` typography utilities in `SurfaceSlot`, `SurfaceRenderer`,
   `Modal`, `SoftwareMergeWizard` with explicit spec classes.
6. Fix `DataTable` header cell padding to `px-[10px]` per spec §4.12.
7. Fix `ProviderSelector` select field radius to `rounded-[3px]` per spec §2.3.

## Non-goals

- Behaviour changes, new props, or new features on any primitive.
- Fixing token violations in consumers (covered by `2026-04-23-token-migration-design.md`).
- Adding new primitives.

---

## Scope

| File | Change |
| --- | --- |
| `frontend/src/lib/components/ui/Callout.svelte` | `rounded-xl` → `rounded-[4px]` (panel) |
| `frontend/src/lib/components/ui/EmptyState.svelte` | `rounded-2xl` → `rounded-[3px]` (card) |
| `frontend/src/lib/components/ui/SectionCard.svelte` | `rounded-2xl` → `rounded-[3px]` (card) |
| `frontend/src/lib/components/ui/PageShell.svelte` | `text-3xl` → `text-[20px] font-bold text-[var(--text-primary)]` |
| `frontend/src/lib/components/ui/TabStrip.svelte` | Outer `rounded-xl` → `rounded-[4px]`; tab buttons `rounded-lg` → `rounded-[3px]`; active state → tint |
| `frontend/src/lib/components/ui/DataTable.svelte` | Header `px-4` → `px-[10px]` |
| `frontend/src/lib/components/ui/ProviderSelector.svelte` | `rounded-xl` → `rounded-[3px]` per §2.3 (form field = button/card radius) |
| `frontend/src/lib/components/surfaces/SurfaceSlot.svelte` | `card` utility → `bg-[var(--bg-surface)] rounded-[3px] border border-[var(--border-subtle)] p-4`; `h3` class → `text-[13px] font-bold text-[var(--text-primary)]` |
| `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte` | `h3` class → `text-[13px] font-bold text-[var(--text-primary)]` |
| `frontend/src/lib/components/Modal.svelte` | `h3` class → `text-[13px] font-bold text-[var(--text-primary)]` |
| `frontend/src/lib/components/SoftwareMergeWizard.svelte` | `h4` classes → `text-[13px] font-bold text-[var(--text-primary)]` (h4 maps to h3 values per Q5) |

> **Cross-spec note:** `SurfaceWorkflow.svelte:471 border-primary-500` is a Skeleton color
> token violation, not a geometry/typography deviation. It is handled by
> `2026-04-23-token-migration-design.md` (§Category B). Do not migrate it here.

---

## Migration pattern

Authoritative values from parent spec (no need to re-read for these):

- §2.3 radius: buttons/cards/table wrappers = `rounded-[3px]`; modals/panels/drawers = `rounded-[4px]`
- §2.4 typography: h1 = `text-[20px] font-bold text-[var(--text-primary)]`;
  h2 = `text-[16px] font-bold text-[var(--text-primary)]`;
  h3 = `text-[13px] font-bold text-[var(--text-primary)]`; h4 (non-spec) → h3 values
- §4.11 tab active: `bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]`
- §4.12 data table padding: `px-[10px]`

Steps per file:

1. Identify deviating class(es) from the gaps table above.
2. Substitute with spec value.
3. Run `cd frontend && npm run check` — no type errors expected (CSS-only changes).
4. Run `cd frontend && npm run test` — no unit test failures expected.
5. Run Playwright parity suite if TabStrip active state was changed (visual regression
   check); update baselines if diff matches intended spec change.

**TabStrip baseline impact:** The active-state change affects `ui-parity` Playwright
snapshots. After fixing `TabStrip`, delete and regenerate:

```bash
rm -rf frontend/tests/e2e/ui-parity.test.ts-snapshots/
rm -rf frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots/
cd frontend && npx playwright test ui-parity ui-parity-responsive --update-snapshots
```

Must run on macOS + Chromium (platform guard). Commit new baselines with the
conformance fix.

---

## Testing

- `npm run check` — zero type errors.
- `npm run test` — all Vitest unit tests pass.
- Playwright parity suite passes after baseline regen (TabStrip change only).
- Visual spot-check: open at least one page with a TabStrip and verify active tab
  shows tint background + accent text (not solid fill).
- Visual spot-check: SectionCard and EmptyState render with tight 3px corners.

## Rollout

Single PR titled `"fix(frontend): align primitive components with design spec geometry and typography"`.

No dependency on other gap sub-specs; can land in any order.
