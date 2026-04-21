# History Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`
(§4.3 Buttons, §4.5 Cards, §4.7 Tables, §4.6 Loading)

**Sub-spec #3g of the UI design-language rollout.** Depends on sub-spec #2
(Button primitive) merged. Form-input sites (date pickers, search) defer
to a future #3g2 pass after sub-spec #2b primitives land.

## Overview

Migrate `/history/+page.svelte` (722 lines) — the update history list
view, its filter chips, per-row actions ("View details", "View logs",
"Rerun" if applicable), and the "Input Required" badge interaction for
in-progress interactive updates (per memory note on `update_history.interactive`).

## Design decisions

**Q1 — "View logs" action shape.**

- Options:
  - (chosen) `<Button variant="ghost" size="sm" leadingIcon={LogsIcon}>View logs</Button>`.
    Match row-action ghost pattern from #3f.
  - Use a link primitive. Rejected — this is not a navigation link;
    it's an action that opens an in-view panel.
- Reasoning: consistent with #3f's row-action shape; "ghost + icon +
  text label" is the emerging row-level action standard.

**Q2 — Filter chip buttons.**

- Options:
  - (chosen) `<Button variant="ghost" size="sm">` for inactive chips,
    active chip gets `class="bg-[var(--bg-hover)] text-[var(--accent)]"`.
    Same active-state override pattern as navbar pills (#3b) and
    settings tab pills (#3c).
  - Introduce dedicated `<FilterChip>` primitive. Rejected — third
    consumer of the same ghost+override pattern; still YAGNI until a
    fourth consumer emerges.
- Reasoning: three consumers is still below the "extract primitive"
  threshold established in #3b and #3c.

**Q3 — "Input Required" badge interaction.**

- Options:
  - (chosen) Leave the existing badge untouched (it's a status
    indicator, not a button). The row's "View logs" button is where the
    interactive terminal session attaches — rename label to
    "Attach terminal" for rows where `interactive && in_progress`, no
    variant change.
  - Promote badge to a button itself. Rejected — semantic mismatch;
    status vs. action.
- Reasoning: status badges remain badges; the button adjacent to them
  owns the attach action.

**Q4 — "Rerun" action scope (if present on history rows).**

- Options:
  - (chosen) Migrate if the action exists today. If not rendered in
    current markup, skip — scope limited to existing buttons.
  - Add rerun action as part of migration. Rejected — feature work, not
    style work.
- Reasoning: sub-spec is migration-only; functional changes belong
  elsewhere.

## Goals

1. Every interactive button in `/history/+page.svelte` renders through
   `<Button>`.
2. Filter chips adopt ghost + active-override pattern.
3. Row-level "View logs" / "Attach terminal" adopt
   `variant="ghost" size="sm" leadingIcon={...}`.
4. "Input Required" badge remains a status indicator (no button).

## Non-goals

- Form-input migration (date pickers, search input) — deferred to #3g2.
- History backend / SSE schema changes — out of scope.
- Log viewer panel refactor — outside Button scope.
- Rerun functionality — feature work, separate spec.

## Scope

Files migrated:

- `frontend/src/routes/history/+page.svelte` — filters, row actions,
  pagination triggers, any modal action buttons rendered inline.

## Migration pattern

Standard translation rules. Special:

- Filter chips: `<Button variant="ghost" size="sm" onclick={() => toggleFilter(...)}
  class={activeFilters.has(filter) ? 'bg-[var(--bg-hover)] text-[var(--accent)]' : ''}>`.
- Row-action "View logs": `<Button variant="ghost" size="sm" leadingIcon={LogsIcon}>
  {interactive && inProgress ? 'Attach terminal' : 'View logs'}</Button>`.

## Data flow

Template-level only. No runtime changes to SSE subscription, filter
state, or pagination.

## Error handling

Button discriminated union catches invalid prop combos at compile time.
Existing error pipelines (failed row, failed SSE reconnect) unchanged.

## Testing

### Unit tests

Extend `history/+page.test.ts`:

- Filter chip active state renders override class.
- Row action variant: ghost + sm + icon.
- Label toggles to "Attach terminal" when `interactive && in_progress`.
- Row with `status = Failed` does not render trigger-update action
  (existing test stays green).

### Integration / e2e

- Playwright re-baseline `/history` default + filter-active +
  in-progress-interactive row permutations. Delta enumeration: filter
  chips shrink to `h-[23px]`; uppercase 9px text; active-chip accent.
- Smoke test SSE-driven row status transition: pending → in_progress →
  completed — button state transitions visible.

## Rollout

Single PR titled
"feat(frontend): migrate history to Button primitive (sub-spec #3g)".

1. `history/+page.svelte` — migrate filters + row actions + pagination
   triggers.
2. Extend unit tests per plan.
3. Re-baseline Playwright snapshots.
4. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset classes across history. Moderate
sensitivity — SSE-driven live updates are the primary regression
concern, mitigated by existing in-progress-row unit tests and Playwright
live-update smoke.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged, sub-spec #2c merged
  (`--bg-hover` for active-filter override), sub-spec #3b merged
  (navbar baseline).
- **Blocks:** #3g2 form-input migration.
- **Parallel-safe with:** sub-spec #3c–f, #3h–k, #4.
