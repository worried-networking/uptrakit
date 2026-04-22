# Software Area Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` (§4.3 Buttons, §4.5 Cards, §4.7
Tables)

**Sub-spec #3f of the UI design-language rollout.** Depends on sub-spec #2 (Button primitive) merged. Form-input sites
defer to a future #3f2 pass after sub-spec #2b primitives land.

## Overview

Migrate the software administration area: `/software/+page.svelte` (1515 lines — software list + filters + bulk
actions), `/software/[id]/+page.svelte` (1247 — software detail, version list, update-trigger button, plugin links),
`IgnoreRulesTab.svelte` (271), `SoftwareMergeWizard.svelte` (466), `AddSoftwareModal.svelte` (109). Total: ~3600 lines
of migration surface, making this the largest single #3 sub-spec.

## Design decisions

**Q1 — "Trigger update" button sites on software area.**

- Options:
  - (chosen) `<UpdateAllButton>` primitive from sub-spec #2 is the canonical aggregate-header trigger on
    `software/+page.svelte` (header-row "Update all" affordance — parent §5.1). The primitive's locked contract is
    `{ state, count?, onclick, ariaLabel?, children?, class? }` — the caller closes over software/host context in the
    `onclick` handler; the primitive itself has no software/hostIds props. Per-row single "Update" on the software list
    (non-header rows) and the software detail page's per-host "Trigger update" site each render as
    `<Button variant="primary" size="sm" loading={isTriggeringHostId === host.host_id}>` with a local guard flag; they
    do NOT use `<UpdateAllButton>` because they lack the count/aggregate semantics.
  - Use `<UpdateAllButton>` for every trigger-update call site. Rejected — the primitive is aggregate-shaped; applying
    it to a single-version action would mis-signal count and dim semantics.
- Reasoning: parent §5.1 distinguishes "Update all" (aggregate) from "Update" (single row). `<UpdateAllButton>` matches
  the first; a plain `<Button variant="primary">` with loading wiring matches the second.

**Q2 — SoftwareMergeWizard navigation buttons.**

- Options:
  - (chosen) `<Button variant="secondary">` for Back, `<Button variant="primary" loading={loading}>` for
    Next/Merge, `<Button variant="ghost">` for Cancel. All three default to `size="md"` (no override) — matches
    modal-dialog density. The step-2 action button label is "Merge" (not "Finish"). The state variable is `loading`
    (not `isSubmitting`). The `secondary` variant class contract is defined by #2c (base Button primitive was #2;
    secondary landed in #2c).
  - Use Button primitive's `leadingIcon` for arrow icons. Deferred — icons optional; shipping without them keeps the
    diff narrow.
- Reasoning: wizard buttons have a conventional variant shape; matching that shape avoids surprise.

**Q3 — Table row-level action buttons (software list).**

- Options:
  - (chosen) `<Button variant="ghost" size="sm">` with `leadingIcon` plus visible text label for every row action (view,
    merge, ignore). Destructive delete uses `variant="danger" size="sm"` with the same icon + text pattern. Text label
    chosen for readability at dense row heights, not because `ariaLabel` is unavailable — `ariaLabel` already shipped in
    #2c and is available as a fallback when a design later drops to icon-only.
  - Icon-only with `ariaLabel` per #2c. Rejected here — these are non-destructive row actions whose intent (merge /
    ignore vs. view) benefits from visible labeling; icon-only compresses row scannability.
- Reasoning: #2c's `ariaLabel` prop is the correct mechanism for icon-only sites, but #3f's row actions are not
  icon-only — they are icon-plus-text by design. No `sr-only` fallback anywhere in this sub-spec.

**Q4 — Bulk-action bar on software list.**

- Options:
  - (chosen) The floating `BatchActionBar` panel that surfaces after selection defers to sub-spec #3k end-to-end (panel
    chrome + its internal buttons). #3f owns only the row-level selection affordances that feed it: individual row
    checkboxes and the select-all header checkbox. Those checkboxes are NOT Button consumers — they render via the
    `<Checkbox>` primitive from sub-spec #2b, not via `<Button>`. If the software list has any separate "launch bulk
    action" button (distinct from a checkbox), it migrates here as `<Button variant="primary" size="sm">`.
  - Migrate the `BatchActionBar` panel inline here. Rejected — duplicates #3k effort.
- Reasoning: shared primitives belong to their own sub-specs (#3k owns the bar; #2b owns Checkbox); #3f owns only the
  local consumers.

## Goals

1. Every interactive button in the five files renders through `<Button>` or `<UpdateAllButton>`.
2. Row-level destructive actions adopt `variant="danger" size="sm"`.
3. Wizard navigation adopts `Back=secondary, Next/Merge=primary, Cancel=ghost`.
4. The aggregate header-row trigger on `/software/+page.svelte` uses `<UpdateAllButton>`; per-host row trigger
   sites on `/software/[id]/+page.svelte` use `<Button variant="primary" size="sm">`.

## Non-goals

- Form-input migration — deferred to #3f2.
- `BatchActionBar` component migration — sub-spec #3k.
- Table column / filter refactor — outside Button scope.
- Software detail tab refactor — existing structure unchanged.
- Backend software merge endpoint — out of scope.

## Scope

Files migrated:

- `frontend/src/routes/software/+page.svelte` — filters, bulk actions, row-level actions.
- `frontend/src/lib/components/Pagination.svelte` — **out of scope for this spec**; owned exclusively by #3k
  (`shared-modals-dialogs`) to avoid parallel merge conflicts. No `Pagination.svelte` changes in this PR.
- `frontend/src/routes/software/[id]/+page.svelte` — trigger update, plugin links, version actions, delete / merge
  launch triggers.
- `frontend/src/routes/software/IgnoreRulesTab.svelte` — add rule launcher, "Create" button in modal, delete rule.
- `frontend/src/lib/components/SoftwareMergeWizard.svelte` — wizard navigation (Back / Next / Merge / Cancel), per-step
  actions.
- `frontend/src/lib/components/AddSoftwareModal.svelte` — "Register Software" submit button, Cancel.

## Migration pattern

Standard translation rules (preset-filled-primary → primary, preset-filled-error → danger, preset-tonal-\* →
secondary/ghost).

Special:

- Aggregate "Update all" header trigger on `software/+page.svelte` →
  `<UpdateAllButton state={...} count={...} onclick={handleTriggerAll} ariaLabel="Update all N packages">` per the
  locked #2 contract. The primitive has no `software` / `hostIds` props; the call site closes over that context in the
  `onclick` handler.
- Per-host "Trigger update" on `software/[id]/+page.svelte` →
  `<Button variant="primary" size="sm" loading={isTriggeringHostId === host.host_id}>Trigger update</Button>` with a
  local `isTriggeringHostId` guard flipping to `host.host_id` during the handler's awaited window and back to `null` in
  both success and catch paths. The detail page has a per-host table (not a per-version table); there is no
  `isTriggeringVersionId` state variable.
- Row-level actions → `<Button variant="ghost" size="sm" leadingIcon={...}>Text</Button>`; row delete uses
  `variant="danger" size="sm"`. Row actions keep visible text labels (Q3); no `sr-only` and no `ariaLabel` needed at
  these sites.
- Pagination (Previous / Next) on `/software` — `Pagination.svelte` is a shared component; its migration is deferred
  to sub-spec #3k (`shared-modals-dialogs`) to prevent parallel merge conflicts. This spec does **not** touch
  `Pagination.svelte`. The software list pagination will appear migrated once #3k lands.
- Wizard nav → `<Button variant="secondary">Back</Button>` +
  `<Button variant="primary" loading={loading}>Next</Button>` (step 1) /
  `<Button variant="primary" loading={loading}>Merge</Button>` (step 2) +
  `<Button variant="ghost">Cancel</Button>`. `loading` is reset to `false` in the catch path of each step handler so a
  failed step returns the button to idle; Cancel stays enabled during submit so the user can always back out.
- `IgnoreRulesTab` modal "Create" button → `<Button variant="primary" disabled={!ignoreForm.name.trim()}>Create</Button>`.
  There is no `isSaving` state and no "Saving…" text swap; the button is disabled while the name field is empty and
  has no loading state. Add rule launcher renders `variant="primary" size="sm"`; row delete renders
  `variant="danger" size="sm"`.
- `AddSoftwareModal` submit → `<Button variant="primary" loading={submitting}>Register Software</Button>` with
  in-flight text swap to "Registering…" removed — spinner + static label per #2 §4.6. The state variable is
  `submitting` (not `isSubmitting`). Cancel → `<Button variant="secondary">Cancel</Button>`.

## Data flow

Template-level only. Trigger-update path delegates to UpdateAllButton's internal state machine (already covered in
sub-spec #2). No new stores.

## Error handling

Button discriminated union catches invalid prop combos at compile time. Merge wizard error states propagate through
existing step-level error stores.

## Testing

### Unit tests

- `software/+page.test.ts` — filters, row actions (ghost + sm) + delete (danger + sm), pagination Previous/Next
  (secondary + sm with disabled passthrough). Header-row aggregate trigger renders `<UpdateAllButton>` with `state` +
  `count` props from the #2 contract (not raw `<Button>`). Checkbox row + header select-all render via `<Checkbox>`
  (from #2b), not via `<Button>`.
- `software/[id]/+page.test.ts` — per-host "Trigger update" renders
  `<Button variant="primary" size="sm" loading={isTriggeringHostId === host.host_id}>` and the guard flag flips to the
  matching host id during the awaited window and back to null in both success and catch paths (render two host rows,
  trigger one, assert only that host's button has `aria-busy="true"`). Merge / delete launchers carry expected
  variants. Plugin-link buttons render `variant="ghost" size="sm"`.
- `IgnoreRulesTab.test.ts` — add launcher (primary + sm), row delete (danger + sm), modal "Create" button renders
  `variant="primary"` + `disabled={!ignoreForm.name.trim()}` with no loading state (regression guard: no `isSaving`
  state exists, no "Saving…" text swap to remove).
- `SoftwareMergeWizard.test.ts` — Back `variant="secondary"`, Next (step 1) / Merge (step 2) `variant="primary"` with
  `loading={loading}` and `aria-busy="true"` during submit, Cancel `variant="ghost"` remains enabled throughout the
  submit window. On simulated step-submit error, `loading` resets to `false` and the error surfaces via the
  existing step-error store without leaving Next/Merge stuck in `loading`.
- `AddSoftwareModal.test.ts` — "Register Software" button renders `variant="primary"` + `loading={submitting}` with
  static children "Register Software" (regression guard: no "Registering…" text swap); Cancel renders
  `variant="secondary"`.

### Integration / e2e

- Playwright re-baseline `/software` (list + filters open), software detail `/software/[id]`, merge wizard (each step),
  and `AddSoftwareModal` in both dark + light themes.
- Delta enumeration per parent §9 (split by size):
  - Row actions + pagination (`size="sm"`): `h-[19px]`, label `8.5px`.
  - Header "Update all" (via `<UpdateAllButton>`) + modal / wizard submits (`size="md"`): `h-[23px]`, label `9px`.
  - `variant="danger"` renders error gradient on row Delete.
  - `variant="secondary"` renders `--bg-hover` token on hover per #2c.
- Snapshot masking per parent §3 approved dynamic categories:
  - Mask in-flight spinner rotation on every `<Button loading>` site (force `loading=false` or mask spinner element).
    `<UpdateAllButton>` has no loading/triggering state (`UpdateAllState = 'idle' | 'dim'` only) — no spinner masking
    needed for it.
  - Mask version digest strings (e.g., `sha256:…`) and relative timestamps — only surrounding button chrome asserts.
  - Mask transient toast banners surfaced by trigger / merge / delete flows.
  - Per parent §3, total masked area stays under 15% per snapshot.
- Wizard smoke test exercises the full transition matrix:
  - valid step advance (Next enables, `loading` flips true then back to false on success);
  - invalid step advance blocked (Next disabled / stays idle);
  - step submit error (`loading` returns to false, error surfaces via existing error store, Cancel remains enabled
    throughout);
  - Back navigation to prior step preserves entered values;
  - Merge (step 2) posts the merge payload and routes away.
- Trigger-update smoke test asserts `aria-busy="true"` on the per-host `<Button>` during the awaited dispatch window.
  `<UpdateAllButton>` has no `triggering` state and emits no `aria-busy` attribute — do not assert `aria-busy` on it.

## Rollout

Single PR titled "feat(frontend): migrate software area to Button primitive (sub-spec #3f)".

Commit granularity: each of steps 1–5 lands as a distinct commit with the full frontend gate (type-check + unit tests
passing) on each commit so a bisect can isolate a regression to the specific migrated file. Step 6 bundles the test-plan
extension; step 7 is the snapshot re-baseline commit. Same pattern established in #3b / #3d.

1. `software/+page.svelte` — migrate filters + row-level actions + `<UpdateAllButton>` header trigger. (`Pagination.svelte`
   deferred to #3k — no changes here.)
2. `software/[id]/+page.svelte` — migrate per-host `<Button variant="primary">` trigger-update sites + delete / merge
   launchers + plugin-link buttons.
3. `IgnoreRulesTab.svelte` — migrate rule CRUD (add launcher + per-row delete + modal "Create" button).
4. `SoftwareMergeWizard.svelte` — migrate wizard nav + per-step actions.
5. `AddSoftwareModal.svelte` — migrate "Register Software" / Cancel buttons (no text-swap to remove; the modal already
   uses inline ternary "Registering…" which is replaced by the spinner + static label pattern from #2 §4.6).
6. Extend unit tests per plan.
7. Re-baseline Playwright snapshots.
8. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset classes across software admin. Largest sub-spec by LOC across the #3 series — mitigated
by per-file commit within the PR (easy bisect on regression) plus Playwright coverage on list, detail, and merge.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged (Button + UpdateAllButton primitives), sub-spec #2c merged (`variant="secondary"` +
  `ariaLabel`
  - `--bg-hover` token), sub-spec #2b merged only for the Checkbox consumer on row + header selection (Q4).
- **Blocks:** #3f2 form-input migration; #3k depends on this only for bulk-action-bar consumer migration clarity.
- **Parallel-safe with:** sub-spec #3c–e, #3g–j, #4.
