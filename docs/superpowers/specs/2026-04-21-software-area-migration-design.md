# Software Area Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`
(§4.3 Buttons, §4.5 Cards, §4.7 Tables)

**Sub-spec #3f of the UI design-language rollout.** Depends on sub-spec #2
(Button primitive) merged. Form-input sites defer to a future #3f2 pass
after sub-spec #2b primitives land.

## Overview

Migrate the software administration area: `/software/+page.svelte` (1515
lines — software list + filters + bulk actions), `/software/[id]/+page.svelte`
(1247 — software detail, version list, update-trigger button, plugin links),
`IgnoreRulesTab.svelte` (271), `SoftwareMergeWizard.svelte` (466),
`AddSoftwareModal.svelte` (109). Total: ~3600 lines of migration
surface, making this the largest single #3 sub-spec.

## Design decisions

**Q1 — "Trigger update" button variant on software detail.**

- Options:
  - (chosen) `<UpdateAllButton>` wrapper from sub-spec #2 — already an
    established primitive for this exact workflow. Call site passes
    software + host context.
  - Raw `<Button variant="primary">`. Rejected — UpdateAllButton owns
    the polling/status-aware state machine per parent §4.6; re-using
    that primitive here eliminates duplication.
- Reasoning: parent spec defines UpdateAllButton as the canonical
  trigger-update surface; every trigger-update call site should adopt
  it as sub-spec #2 rolls out.

**Q2 — SoftwareMergeWizard navigation buttons.**

- Options:
  - (chosen) `<Button variant="secondary">` for Back, `<Button
    variant="primary">` for Next/Finish, `<Button variant="ghost">` for
    Cancel. Standard wizard layout.
  - Use Button primitive's `leadingIcon` for arrow icons. Deferred —
    icons optional; shipping without them keeps the diff narrow.
- Reasoning: wizard buttons have a conventional variant shape; matching
  that shape avoids surprise.

**Q3 — Table row-level action buttons (software list).**

- Options:
  - (chosen) `<Button variant="ghost" size="sm">` with `leadingIcon`
    for every row action (view, merge, ignore, delete). Destructive
    delete uses `variant="danger" size="sm"` instead.
  - Use icon-only with sr-only label. Deferred — Button primitive
    ariaLabel prop (sub-spec #2 follow-up) lands first; for now, include
    a short text label alongside the icon to avoid sr-only proliferation.
- Reasoning: table rows benefit from label clarity until ariaLabel lands.

**Q4 — Bulk-action bar on software list.**

- Options:
  - (chosen) Defer to sub-spec #3k (`BatchActionBar` shared component).
    This sub-spec only migrates the trigger that opens it; the bar
    itself is shared.
  - Migrate inline here. Rejected — duplicates #3k effort.
- Reasoning: shared primitive belongs in #3k; #3f owns only local
  consumers of it.

## Goals

1. Every interactive button in the five files renders through `<Button>`
   or `<UpdateAllButton>`.
2. Row-level destructive actions adopt `variant="danger" size="sm"`.
3. Wizard navigation adopts `Back=secondary, Next/Finish=primary, Cancel=ghost`.
4. Every "trigger update" call site on `/software/[id]` uses `<UpdateAllButton>`.

## Non-goals

- Form-input migration — deferred to #3f2.
- `BatchActionBar` component migration — sub-spec #3k.
- Table column / filter refactor — outside Button scope.
- Software detail tab refactor — existing structure unchanged.
- Backend software merge endpoint — out of scope.

## Scope

Files migrated:

- `frontend/src/routes/software/+page.svelte` — filters, bulk actions,
  row-level actions, pagination triggers.
- `frontend/src/routes/software/[id]/+page.svelte` — trigger update,
  plugin links, version actions, delete / merge launch triggers.
- `frontend/src/routes/software/IgnoreRulesTab.svelte` — add rule,
  delete rule, save.
- `frontend/src/lib/components/SoftwareMergeWizard.svelte` — wizard
  navigation (Back / Next / Cancel / Finish), per-step actions.
- `frontend/src/lib/components/AddSoftwareModal.svelte` — Save, Cancel.

## Migration pattern

Standard translation rules. Special:

- "Trigger update" on `[id]/+page.svelte` → `<UpdateAllButton software=...
  hostIds=.../>` per sub-spec #2.
- Row-level actions → `<Button variant="ghost" size="sm" leadingIcon={...}>Text</Button>`;
  row delete uses `variant="danger"`.
- Wizard nav → `<Button variant="secondary">Back</Button>` +
  `<Button variant="primary" loading={isSubmitting}>Next</Button>` +
  `<Button variant="ghost">Cancel</Button>`.

## Data flow

Template-level only. Trigger-update path delegates to UpdateAllButton's
internal state machine (already covered in sub-spec #2). No new stores.

## Error handling

Button discriminated union catches invalid prop combos at compile time.
Merge wizard error states propagate through existing step-level error
stores.

## Testing

### Unit tests

- Extend existing `software/+page.test.ts` / `[id]/+page.test.ts` /
  `IgnoreRulesTab.test.ts` / `SoftwareMergeWizard.test.ts` /
  `AddSoftwareModal.test.ts` with per-button variant + loading
  assertions.
- Row-level action variant checks (ghost + danger).
- Wizard button shape: Back is secondary, Next carries loading during
  submit, Cancel is ghost.
- Trigger-update button renders UpdateAllButton, not raw Button.

### Integration / e2e

- Playwright re-baseline `/software` (list + filters open) and
  `/software/[id]` (detail + merge wizard + add modal). Delta
  enumeration: row-action buttons shrink to `h-[23px]`; uppercase 9px
  text; wizard button gradient fills.
- Smoke test merge wizard flow end-to-end — button state transitions
  visible.

## Rollout

Single PR titled
"feat(frontend): migrate software area to Button primitive (sub-spec #3f)".

1. `software/+page.svelte` — migrate filters + row + bulk-launcher buttons.
2. `software/[id]/+page.svelte` — migrate trigger-update + version
   actions; swap raw Button for UpdateAllButton on trigger sites.
3. `IgnoreRulesTab.svelte` — migrate rule CRUD.
4. `SoftwareMergeWizard.svelte` — migrate wizard nav + per-step actions.
5. `AddSoftwareModal.svelte` — migrate save/cancel.
6. Extend unit tests per plan.
7. Re-baseline Playwright snapshots.
8. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset classes across software admin. Largest
sub-spec by LOC across the #3 series — mitigated by per-file commit within
the PR (easy bisect on regression) plus Playwright coverage on list,
detail, and merge.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged (UpdateAllButton primitive).
- **Blocks:** #3f2 form-input migration; #3k depends on this only for
  bulk-action-bar consumer migration clarity.
- **Parallel-safe with:** sub-spec #3c–e, #3g–j, #4.
