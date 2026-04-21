# Layout Shell + Home Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`
(§3 Layout, §4.3 Buttons, §4.5 Stat Cards)

**Sub-spec #3b of the UI design-language rollout.** Depends on sub-spec #2
(Button primitive) merged. Independent of sub-spec #2b — this sub-spec does
not touch form inputs.

## Overview

Migrate the root application chrome (`frontend/src/routes/+layout.svelte`,
636 lines) and the dashboard home route
(`frontend/src/routes/+page.svelte`, 309 lines) from Skeleton
`preset-filled-*` / `preset-tonal-*` button markup and ad-hoc inline
classes to sub-spec #2's `<Button>` primitive plus parent-spec §3 /
§4.5 layout conventions. This is the first authenticated-app sub-spec —
it sets the reference shape for every subsequent #3 migration.

## Design decisions

**Q1 — Navbar / topbar button migration scope.**

- Options:
  - (chosen) Migrate every user-interactive button in the layout
    (theme toggle, user menu trigger, nav pills, sign-out action) to
    `<Button>` with appropriate variant. Nav pills use `ghost`; primary
    CTAs (e.g. sign-out inside menu) use matching semantic variant.
  - Leave navbar/topbar as a separate sub-spec. Rejected — the whole
    point of #3b is to establish the reference chrome; bifurcating
    navbar from home page creates two PRs that need to land together
    to avoid visual inconsistency on dashboard.
- Reasoning: layout shell is one cohesive unit; one PR, one re-baseline.

**Q2 — Stat cards on the home dashboard: migrate inline or leave for a
dedicated primitive sub-spec.**

- Options:
  - (chosen) Leave stat cards as inline markup for now. #3b touches
    them only to swap embedded buttons (if any) to `<Button>`. Parent
    §4.5 defines stat-card shape but does not pin it to a primitive;
    extracting a `<StatCard>` primitive would require its own
    brainstorming + spec.
  - Extract `<StatCard>` primitive here. Rejected — scope creep;
    primitives live in their own sub-specs per the established pattern
    (#2, #2b).
- Reasoning: primitive extraction has its own design round; don't
  graft it into a migration sub-spec.

**Q3 — Theme toggle button: `<Button>` or standalone primitive?**

- Options:
  - (chosen) `<Button variant="ghost" size="sm">` with icon-only
    children (just the `leadingIcon` snippet, empty text children).
    Matches existing §4.3 ghost shape.
  - Introduce a `<IconButton>` primitive. Rejected — one consumer today;
    YAGNI.
- Reasoning: `<Button>` already supports icon-only via leadingIcon
  slot plus empty children string.

**Q4 — Empty children in Button for icon-only buttons.**

- Options:
  - (chosen) Pass empty `{#snippet children()}{/snippet}` or an
    accessible `<span class="sr-only">` label. Consumers pass
    `aria-label` through Button's `class` prop? No — Button primitive
    needs an `ariaLabel` prop addition for icon-only use.
  - Pass visually-hidden text children (`<span class="sr-only">Toggle
    theme</span>`). Acceptable but verbose at every call site.
- Reasoning: this is an API gap in Button. Flagging it as a follow-up
  in #3b's rollout; if impl notes the gap during implementation, file
  a minor Button primitive update (ariaLabel?: string prop on Button —
  matches the UpdateAllButton pattern already established in #2).
  Implementation can ship via `<span class="sr-only">` as a tactical
  solution until the primitive update lands; no blocker.

## Goals

1. Every interactive button in `+layout.svelte` and `+page.svelte`
   renders through `<Button>`.
2. Navbar pills adopt `ghost` variant; primary CTAs adopt `primary`;
   destructive actions adopt `danger`.
3. Delete `preset-filled-*` / `preset-tonal-*` class attributes from
   both files.
4. Stat cards on the home dashboard retain current markup; only
   embedded action buttons migrate.

## Non-goals

- `<StatCard>` primitive extraction — deferred.
- `<IconButton>` primitive — deferred; today's call sites use
  `<Button>` with sr-only text children.
- Stat-card color token verification — handled by sub-spec #1
  conformance.
- Form-input migration — not relevant to these two files.
- Nav-link structural refactor — existing SvelteKit routing stays
  unchanged.

## Scope

Files migrated:

- `frontend/src/routes/+layout.svelte` — global chrome, topbar, nav,
  theme toggle, user menu, sign-out.
- `frontend/src/routes/+page.svelte` — dashboard home page, stat cards,
  any embedded action buttons (e.g. "Enroll Host" empty-state action).

For each file: every element currently using `preset-filled-*` or
`preset-tonal-*` or equivalent inline class contracts for button
styling migrates to `<Button>` with the semantic variant from §4.3.

## Migration pattern

Per-button translation rules:

- `preset-filled-primary-500` → `<Button variant="primary">`.
- `preset-tonal-primary` / `preset-tonal-surface` → `<Button variant="ghost">`.
- `preset-filled-error-*` → `<Button variant="danger">`.
- `variant-ghost-surface` (nav pills in active state) →
  `<Button variant="ghost">` plus consumer-level `class="text-[var(--accent)]"`
  override to express active-nav accent coloring (acceptable because
  active-nav is a route-aware state, not a base Button variant).

For link-styled nav items that navigate via `href`, use the polymorphic
`<Button href="...">` branch.

For icon-only buttons (theme toggle, menu trigger), pass the icon as
`leadingIcon` and use a visually-hidden `<span class="sr-only">` child
for screen readers until a Button primitive `ariaLabel?: string` update
lands.

## Data flow

No runtime behavior changes. Template-level migrations only. Theme
toggle's existing `onclick` handler, user-menu open/close state, and
sign-out action all pass through unchanged — only the button element's
rendered class contract changes.

## Error handling

- Button primitive's discriminated union catches invalid prop
  combinations at compile time.
- Focus-visible rings inherit from sub-spec #1's global `app.css`
  rule — navbar keyboard navigation stays consistent.

## Testing

### Unit tests

Extend existing `+layout.svelte` / `+page.svelte` spec files (or create
if absent) with:

- Each migrated button renders with expected variant class fragment
  (`h-[23px]`, gradient for primary, border for ghost).
- Theme toggle button carries appropriate `aria-label` (via sr-only
  child or primitive-update prop).
- Active nav pill receives accent-text override class.
- Sign-out action carries `variant="danger"` class fragment.

### Integration / e2e

- Playwright re-baseline for every route — `+layout` chrome touches
  every authenticated page. PR description enumerates the nav / user-
  menu / theme-toggle deltas per parent §9 waiver schema.
- Deliberate visual-delta enumeration: navbar button heights shrink to
  `h-[23px]` §4.3 compact; uppercase 9px text; primary gradient fill on
  sign-out CTAs.
- All non-chrome route content (list tables, forms) must stay within
  0.5 % threshold — #3b does not touch page content.

## Rollout

Single PR titled
"feat(frontend): migrate layout shell + home dashboard to Button primitive (sub-spec #3b)".

1. `frontend/src/routes/+layout.svelte` — migrate every button site,
   including theme toggle, user menu, nav pills, sign-out.
2. `frontend/src/routes/+page.svelte` — migrate every button site on
   the dashboard home page.
3. Extend unit tests per plan.
4. Re-baseline Playwright snapshots for authenticated app chrome.
5. Full frontend gate.

### Risk + rollback

Revert of one PR restores Skeleton preset classes app-wide on chrome.
Highest-visibility surface — mitigated by per-route Playwright
regression gates.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 PR1 merged.
- **Blocks:** none directly, but subsequent #3c–k sub-specs share this
  layout chrome — landing #3b first stabilises the cross-route visual
  baseline before each #3 subsequent sub-spec adds its own snapshot
  re-baselines.
- **Parallel-safe with:** sub-spec #2b, sub-spec #4, sub-spec #3a.
