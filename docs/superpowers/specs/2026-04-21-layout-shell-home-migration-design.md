# Layout Shell + Home Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` (§3 Layout, §4.3 Buttons, §4.5 Stat
Cards)

**Sub-spec #3b of the UI design-language rollout.** Depends on sub-spec #2 (Button primitive) merged. Independent of
sub-spec #2b — this sub-spec does not touch form inputs.

## Overview

Migrate the root application chrome (`frontend/src/routes/+layout.svelte`, 636 lines) and the dashboard home route
(`frontend/src/routes/+page.svelte`, 309 lines) from Skeleton `preset-filled-*` / `preset-tonal-*` button markup and
ad-hoc inline classes to sub-spec #2's `<Button>` primitive plus parent-spec §3 / §4.5 layout conventions. This is the
first authenticated-app sub-spec — it sets the reference shape for every subsequent #3 migration.

## Design decisions

**Q1 — Navbar / topbar button migration scope.**

- Options:
  - (chosen) Migrate every user-interactive button in the layout (theme toggle, user menu trigger, nav pills, sign-out
    action) to `<Button>` with appropriate variant. Nav pills use `ghost`; primary CTAs (e.g. sign-out inside menu) use
    matching semantic variant.
  - Leave navbar/topbar as a separate sub-spec. Rejected — the whole point of #3b is to establish the reference chrome;
    bifurcating navbar from home page creates two PRs that need to land together to avoid visual inconsistency on
    dashboard.
- Reasoning: layout shell is one cohesive unit; one PR, one re-baseline.

**Q2 — Stat cards on the home dashboard: migrate inline or leave for a dedicated primitive sub-spec.**

- Options:
  - (chosen) Leave stat cards as inline markup for now. #3b touches them only to swap embedded buttons (if any) to
    `<Button>`. Parent §4.5 defines stat-card shape but does not pin it to a primitive; extracting a `<StatCard>`
    primitive would require its own brainstorming + spec.
  - Extract `<StatCard>` primitive here. Rejected — scope creep; primitives live in their own sub-specs per the
    established pattern (#2, #2b).
- Reasoning: primitive extraction has its own design round; don't graft it into a migration sub-spec.

**Q3 — Theme toggle button: `<Button>` or standalone primitive?**

- Options:
  - (chosen) `<Button variant="ghost" size="sm">` with icon-only children (just the `leadingIcon` snippet, empty text
    children). Matches existing §4.3 ghost shape.
  - Introduce a `<IconButton>` primitive. Rejected — one consumer today; YAGNI.
- Reasoning: `<Button>` already supports icon-only via leadingIcon slot plus empty children string.

**Q4 — Icon-only accessible naming: `ariaLabel` prop from #2c.**

- Options:
  - (chosen) Use the `ariaLabel?: string` prop added to the base `<Button>` primitive by sub-spec #2c. Icon-only sites
    (`leadingIcon` + empty children) pass `ariaLabel="Toggle theme"` and similar; primitive renders `aria-label` on the
    underlying element. No `sr-only` fallback at any call site.
  - Ship with `<span class="sr-only">` children as a tactical fallback ahead of #2c. Rejected — #3b hard-blocks on #2c
    (see Dependencies below) specifically so `ariaLabel` is available at migration time; running two accessible-naming
    patterns in parallel creates inconsistency across the navbar.
- Reasoning: #2c exists precisely to unblock this sub-spec (and the later #3j / #3k icon-only consumers). Depending on
  #2c is the correct contract; the sr-only fallback is no longer needed.

## Goals

1. Every interactive button in `+layout.svelte` and `+page.svelte` renders through `<Button>`.
2. Navbar pills adopt `ghost` variant; primary CTAs adopt `primary`; destructive actions adopt `danger`.
3. Delete `preset-filled-*` / `preset-tonal-*` class attributes from both files.
4. Stat cards on the home dashboard retain current markup; only embedded action buttons migrate.

## Non-goals

- `<StatCard>` primitive extraction — deferred.
- `<IconButton>` primitive — deferred; today's call sites use `<Button>` with sr-only text children.
- Stat-card color token verification — handled by sub-spec #1 conformance.
- Form-input migration — not relevant to these two files.
- Nav-link structural refactor — existing SvelteKit routing stays unchanged.

## Scope

Files migrated:

- `frontend/src/routes/+layout.svelte` — global chrome, topbar, nav, theme toggle, user menu, sign-out, plus any other
  interactive button-shaped elements discovered during migration (e.g. sidebar collapse toggle, search trigger,
  notification bell, environment badge-as-button). Goal 1 is "every interactive button" — the enumeration here is
  representative, not exhaustive. Implementers grep the file for `<button`, `preset-filled-`, `preset-tonal-`,
  `variant-ghost-`, and any ad-hoc inline button class strings, and migrate each one.
- `frontend/src/routes/+page.svelte` — dashboard home page, stat cards, any embedded action buttons (e.g. "Enroll Host"
  empty-state action).

For each file: every element currently using `preset-filled-*` or `preset-tonal-*` or equivalent inline class contracts
for button styling migrates to `<Button>` with the semantic variant from §4.3.

## Migration pattern

Per-button translation rules:

- `preset-filled-primary-500` → `<Button variant="primary">`.
- `preset-tonal-primary` / `preset-tonal-surface` → `<Button variant="ghost">`.
- `preset-filled-error-*` → `<Button variant="danger">`.
- `variant-ghost-surface` (nav pills in active state) → `<Button variant="ghost">` plus a consumer-level active-state
  override class string that expresses both the accent text color and the raised background:
  `class="text-[var(--accent)] bg-[var(--bg-hover)]"`. `--bg-hover` lands in sub-spec #2c; #3b consumes it here for the
  first time. The override is applied by the consumer when its own `$derived` active- route check fires, not baked into
  the Button primitive (active-nav is a route-aware state, not a base Button variant).

For link-styled nav items that navigate via `href`, use the polymorphic `<Button href="...">` branch. Nav pills that
represent route links use this branch; nav pills that trigger client-side state (menu open, filter reset) stay on the
`onclick` branch.

For icon-only buttons (theme toggle, menu trigger, any discovered topbar icons), pass the icon as `leadingIcon`, leave
`children` empty, and pass `ariaLabel="<accessible label>"` per Q4 / sub-spec #2c.

## Data flow

No runtime behavior changes. Template-level migrations only. Theme toggle's existing `onclick` handler, user-menu
open/close state, and sign-out action all pass through unchanged — only the button element's rendered class contract
changes.

## Error handling

- Button primitive's discriminated union catches invalid prop combinations at compile time.
- Focus-visible rings inherit from sub-spec #1's global `app.css` rule — navbar keyboard navigation stays consistent.

## Testing

### Unit tests

Extend existing `+layout.svelte` / `+page.svelte` spec files (or create if absent) with:

- Each migrated button renders with expected variant class fragment (`h-[23px]`, gradient for primary, border for
  ghost).
- Theme toggle button (and every other icon-only site) renders the underlying `aria-label` attribute with the string
  passed to the `ariaLabel` prop; when children are empty, the accessible name comes from `ariaLabel` alone (regression
  guard for the #2c wiring).
- Active nav pill receives both `text-[var(--accent)]` and `bg-[var(--bg-hover)]` fragments when the active-route
  condition holds; inactive pills carry neither.
- Nav pills with `href` render as `<a>` elements (polymorphic branch) with the ghost class fragment plus `role="button"`
  inherited from the Button primitive; `onclick`-only pills render as `<button>`.
- Sign-out action carries `variant="danger"` class fragment.

### Integration / e2e

- Playwright re-baseline scope (bounded — `+layout` chrome touches every authenticated page but the snapshot set is
  specifically):
  - `/` (dashboard home — every chrome element plus stat cards).
  - Three representative deep routes picked to exercise the three nav-pill states (active, inactive-same-group,
    inactive-other- group): `/hosts`, `/services`, `/settings`.
  - `/` in both themes (dark + light) to cover the theme toggle's effect on chrome tokens.
  - One route rendered at both collapsed and expanded sidebar widths (if the layout supports that collapse), to catch a
    regression in the width transition.
- Deliberate visual-delta enumeration per parent §9: navbar button heights shrink to `h-[23px]` §4.3 compact; uppercase
  9px text; primary gradient fill on sign-out CTAs; nav pills active-state background now `--bg-hover` (#2c token)
  instead of Skeleton's `variant-ghost-surface` tonal.
- All non-chrome route content (list tables, forms) must stay within 0.5 % threshold — #3b does not touch page content.
  Any drift on deep-route content snapshots is a spec bug and blocks merge.

## Rollout

Single PR titled "feat(frontend): migrate layout shell + home dashboard to Button primitive (sub-spec #3b)".

1. `frontend/src/routes/+layout.svelte` — migrate every button site, including theme toggle, user menu, nav pills,
   sign-out.
2. `frontend/src/routes/+page.svelte` — migrate every button site on the dashboard home page.
3. Extend unit tests per plan.
4. Re-baseline Playwright snapshots for authenticated app chrome.
5. Full frontend gate.

### Risk + rollback

Revert of one PR restores Skeleton preset classes app-wide on chrome. Highest-visibility surface — mitigated by
per-route Playwright regression gates.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 PR1 merged; sub-spec #2c merged (base `ariaLabel` prop for icon-only theme toggle + user
  menu trigger; `--bg-hover` token for active-nav override).
- **Blocks:** none directly, but subsequent #3c–k sub-specs share this layout chrome — landing #3b first stabilises the
  cross-route visual baseline before each #3 subsequent sub-spec adds its own snapshot re-baselines.
- **Parallel-safe with:** sub-spec #2b, sub-spec #4, sub-spec #3a.
