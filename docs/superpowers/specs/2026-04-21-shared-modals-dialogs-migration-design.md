# Shared Modals + Dialogs Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`
(§4.3 Buttons, §4.8 Modals, §4.5 Cards)

**Sub-spec #3k of the UI design-language rollout.** Depends on sub-spec #2
(Button primitive) merged. Form-input sites inside modals defer to a
future #3k2 pass after sub-spec #2b primitives land.

## Overview

Migrate seven shared UI components referenced by every route: `ConfirmDialog.svelte`
(64 lines), `BatchResultDialog.svelte` (47), `BatchActionBar.svelte` (173),
`Pagination.svelte` (83), `ToastNotifications.svelte` (406),
`AssignToHostModal.svelte` (550), `EditHostAssignmentModal.svelte`
(1365). These are the cross-cutting primitives — a regression here hits
every consumer at once.

## Design decisions

**Q1 — ConfirmDialog confirm/cancel variants.**

- Options:
  - (chosen) Configurable per-call: props `confirmVariant?: 'primary' |
    'danger'` (default `primary`); Cancel always `secondary`.
    Consumer passes `confirmVariant="danger"` for destructive confirms.
  - Always `primary` + `secondary`. Rejected — callers need danger
    semantics; hardcoding `primary` forces every caller to override
    inline.
- Reasoning: ConfirmDialog is the most-used primitive for destructive
  confirms across the app (Delete rule, Delete provider, Revoke token,
  etc.). A single `confirmVariant` prop is the correct generalization.

**Q2 — BatchActionBar action buttons.**

- Options:
  - (chosen) Accept `actions: Array<{label, variant, onclick, loading}>`
    prop; render each as `<Button variant={action.variant} size="sm"
    loading={action.loading}>`. Component is data-driven.
  - Hardcode specific action variants. Rejected — different bulk flows
    (software, hosts, services) need different action sets.
- Reasoning: data-driven shape matches how consumers already compose
  the bar; passes variant through rather than guessing.

**Q3 — Pagination prev/next buttons.**

- Options:
  - (chosen) `<Button variant="ghost" size="sm" leadingIcon={...}>Prev</Button>`
    / `<Button variant="ghost" size="sm" trailingIcon={...}>Next</Button>`.
    Page number buttons also ghost with active-state override class.
  - Use `<Link>` primitive from #2b. Rejected — pagination is not
    navigation (no URL change); it mutates filter state.
- Reasoning: Pagination is action-shaped but not state-transitioning;
  ghost variant matches. Requires Button primitive to support
  `trailingIcon` snippet — flag as primitive follow-up if not already
  supported.

**Q4 — ToastNotifications dismiss buttons.**

- Options:
  - (chosen) `<Button variant="ghost" size="sm" class="p-0 w-5 h-5"
    leadingIcon={CloseIcon} aria-label="Dismiss">` per toast. Size
    override matches the compact toast close-button shape.
  - Retain raw `<button>` with inline class. Rejected — migration goal
    is to delete preset-* classes uniformly.
- Reasoning: toast close is a legitimate ghost icon-only button;
  requires the Button primitive `ariaLabel` update to land or `sr-only`
  tactical fallback.

**Q5 — AssignToHostModal / EditHostAssignmentModal scope.**

- Options:
  - (chosen) Migrate every button in both modals. Form-input sites
    inside (search box, select) defer to #3k2. Modal *shell* (backdrop,
    focus trap) stays untouched — it's not a button concern.
  - Defer modal migration entirely. Rejected — these two components
    render across every host/service management flow; leaving them on
    preset-* classes creates visual inconsistency.
- Reasoning: button-level migration is mechanical and essential for
  cross-surface consistency; inputs are a separate concern tied to #2b.

**Q6 — `trailingIcon` on Button primitive.**

- Options:
  - (chosen) Flag Button primitive update: add `trailingIcon?: Snippet`
    alongside existing `leadingIcon`. Used by Pagination "Next" button.
    Matches existing `leadingIcon` shape — one-line addition.
  - Pass icon inline as children. Rejected — primitive owns the spacing
    contract between icon and text; inline children re-creates that
    per site.
- Reasoning: API gap in Button primitive. Include in primitive-update
  PR bundle alongside `ariaLabel` prop add.

## Goals

1. Every interactive button in the seven files renders through
   `<Button>`.
2. `ConfirmDialog` exposes `confirmVariant` prop; consumers pass
   `"danger"` for destructive confirms.
3. `BatchActionBar` renders actions data-driven from a prop array.
4. `Pagination` adopts ghost + icon shape; uses new `trailingIcon`
   prop when available.
5. `ToastNotifications` dismiss adopts ghost icon-only shape with
   `ariaLabel`.

## Non-goals

- Form-input migration inside modals — deferred to #3k2.
- Modal shell (backdrop, focus trap) refactor — outside Button scope.
- New modal designs — feature work.
- ToastNotifications store architecture — untouched.

## Scope

Files migrated:

- `frontend/src/lib/components/ConfirmDialog.svelte` — add
  `confirmVariant` prop; migrate confirm + cancel buttons.
- `frontend/src/lib/components/BatchResultDialog.svelte` — migrate
  Close / View details buttons.
- `frontend/src/lib/components/BatchActionBar.svelte` — accept actions
  prop array; migrate each to Button.
- `frontend/src/lib/components/Pagination.svelte` — migrate prev /
  next / page-number buttons.
- `frontend/src/lib/components/ToastNotifications.svelte` — migrate
  dismiss buttons.
- `frontend/src/lib/components/AssignToHostModal.svelte` — migrate
  save, cancel, add-assignment, remove-assignment buttons.
- `frontend/src/lib/components/EditHostAssignmentModal.svelte` —
  migrate save, cancel, per-row edit / save / delete buttons.

## Migration pattern

Standard translation rules. Special:

- `ConfirmDialog`: add `confirmVariant?: 'primary' | 'danger' = 'primary'`
  to props; bind to `<Button variant={confirmVariant}>`.
- `BatchActionBar`: accept `actions: Array<{label: string; variant?:
  ButtonVariant; loading?: boolean; onclick: () => void}>` prop;
  `{#each actions as a}<Button variant={a.variant ?? 'secondary'} size="sm"
  loading={a.loading} onclick={a.onclick}>{a.label}</Button>{/each}`.
- `Pagination`: Prev/Next use `leadingIcon` / `trailingIcon`; page-
  number pills reuse ghost+active-override pattern.
- Consumer audit: update every `ConfirmDialog` call site to pass
  `confirmVariant="danger"` where the action is destructive. Grep call
  sites, update in same PR.

## Data flow

Template-level + API-additive. `ConfirmDialog` gains one optional prop.
`BatchActionBar` consumers now pass `actions` array (some already do —
others migrate in same PR). No runtime behavior changes.

## Error handling

Button discriminated union catches invalid prop combos at compile time.
Modal focus trap + keyboard handling unchanged.

## Testing

### Unit tests

Extend existing `ConfirmDialog.test.ts`, `BatchActionBar` (new),
`Pagination.test.ts`, `ToastNotifications.test.ts`, `AssignToHostModal.test.ts`,
`EditHostAssignmentModal.test.ts`:

- `ConfirmDialog`: confirm button renders with configured variant
  (primary by default, danger when prop passed); cancel always
  secondary.
- `BatchActionBar`: actions prop renders each action with correct
  variant + loading state.
- `Pagination`: prev/next render ghost + icon; page-number active state
  applies override class.
- `ToastNotifications`: dismiss carries correct `aria-label`.
- Modals: save/cancel variants; destructive row actions carry danger.
- Consumer sites: grep ensures every `<ConfirmDialog>` call with
  destructive intent passes `confirmVariant="danger"`.

### Integration / e2e

- Playwright re-baseline every route that renders one of these
  components (approximately: `/software`, `/hosts`, `/services`,
  `/settings` with danger-zone). Delta enumeration: dialog button
  variants; batch-action button shrink to `h-[23px]`; pagination icons
  visible.
- Smoke test a destructive confirm flow (revoke token, delete rule) —
  danger variant visually confirmed, focus trap unchanged.

## Rollout

Single PR titled
"feat(frontend): migrate shared modals + dialogs to Button primitive (sub-spec #3k)".

Prereq: Button primitive update PR (ariaLabel prop + trailingIcon prop)
lands first or as sibling PR.

1. `ConfirmDialog.svelte` — add `confirmVariant` prop; migrate confirm
   and cancel.
2. Consumer sweep: every `<ConfirmDialog>` call site that triggers a
   destructive action passes `confirmVariant="danger"`.
3. `BatchResultDialog.svelte` — migrate Close / details.
4. `BatchActionBar.svelte` — data-driven actions prop; migrate render
   loop.
5. `Pagination.svelte` — migrate prev/next/page buttons.
6. `ToastNotifications.svelte` — migrate dismiss.
7. `AssignToHostModal.svelte` — migrate every action.
8. `EditHostAssignmentModal.svelte` — migrate every action.
9. Extend unit tests per plan.
10. Re-baseline Playwright snapshots for every authenticated route (these
    primitives cut across the app).
11. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset classes across shared primitives,
reverting their effect app-wide. Highest-blast-radius sub-spec —
mitigated by widest Playwright coverage + focused unit tests on each
primitive.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged; Button primitive update (ariaLabel,
  trailingIcon) landed as sibling work.
- **Blocks:** #3k2 form-input migration inside modals.
- **Parallel-safe with:** sub-spec #3c–j, #4 (surface layer).
