# Settings Shell + Auth/Registration/Danger Zone Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` (§3 Layout, §4.3 Buttons, §4.10 Form
Validation)

**Sub-spec #3c of the UI design-language rollout.** Depends on sub-spec #2 (Button primitive) merged. Form-input sites
defer to sub-spec #2b + a future #3c2 pass; this sub-spec migrates buttons only.

## Overview

Migrate the settings shell tab scaffold (`frontend/src/routes/settings/+page.svelte`, 334 lines) and four tab-body
components — `GlobalSettingsTab.svelte` (582), `AuthenticationSettings.svelte` (57), `RegistrationSettings.svelte`
(103), `DangerZone.svelte` (157) — from Skeleton preset-\* button markup to the `<Button>` primitive. `+page.svelte`'s
tab navigation is already handled by `<TabStrip>`; only its 5× "Retry All" error-state buttons are in scope. The four
components render inside it per active tab.

## Design decisions

**Q1 — Tab pill migration: scope clarification.**

`+page.svelte` already uses `<TabStrip>` from `$lib/components/ui` (imported at line 25, rendered at lines 237–243).
`<TabStrip>` is a dedicated accessible tab primitive — no raw `<button>` pills exist in this file. The migration scope
for `+page.svelte` is therefore limited to the non-TabStrip buttons: the 5× "Retry All" buttons inside error Callout
blocks (lines 258, 266, 279, 287, 295). No tab-pill migration work is required here.

> **Design note (superseded):** an earlier draft of Q1 proposed migrating raw `<button>` tab pills to
> `<Button variant="ghost" size="sm">`. That option is moot — no such raw buttons exist in the current source.
> `<TabStrip>` is already the tab primitive and is out of scope for this PR.

**Q2 — Danger Zone destructive actions.**

- Options:
  - (chosen) `<Button variant="danger">` for every destructive confirm button; per parent §4.3 this renders the error
    gradient + red ring.
  - Keep existing `preset-filled-error` class. Rejected — baseline migration.
- Reasoning: Danger Zone is the canonical destructive surface; landing variant="danger" here anchors the visual
  definition for every later destructive confirm (#3k modals reuse it).

**Q5 — DangerZone migration boundary.**

`DangerZone.svelte` does **not** use `<ConfirmDialog>`. It uses a bespoke `<Modal>` (from `$lib/components/Modal.svelte`)
with inline footer buttons rendered in a `{#snippet footer()}` block. The earlier boundary reasoning referencing
`<ConfirmDialog>` and sub-spec #3k does not apply here.

- Options:
  - (chosen) Migrate all three DangerZone button sites in this sub-spec (#3c):
    - Launcher button (`"Reset Data"`, line 66) → `<Button variant="danger">`.
    - Inline Cancel button inside `{#snippet footer()}` → `<Button variant="secondary">`.
    - Inline "Reset All Data" confirm button inside `{#snippet footer()}` → `<Button variant="danger">`.
  - Defer inline footer buttons to #3k. Rejected — #3k owns the `<ConfirmDialog>` _primitive_; DangerZone's bespoke
    `<Modal>` footer buttons are owned by DangerZone itself, not by a shared primitive. Deferring creates an orphaned
    migration gap.
- Reasoning: DangerZone's `<Modal>` is not a shared component. All button markup in this file is #3c's responsibility.
  No double-migration risk with #3k because #3k's scope is the `<ConfirmDialog>` primitive, which this file does not use.

**Q3 — OIDC content boundary.**

- Options:
  - (chosen) All OIDC buttons defer to sub-spec #3e. Verified against the current source: `GlobalSettingsTab.svelte`
    contains no OIDC launcher; `<OidcProvidersSettings>` is rendered directly from `+page.svelte` and owns every OIDC
    button (Add, Edit, Activate, Deactivate, Delete, Cancel, Save) end-to-end.
  - Migrate anything OIDC-adjacent here. Rejected — scope bleed + false duplication (the launcher this option assumed
    simply does not exist).
- Reasoning: #3e owns the OIDC component outright; #3c has no OIDC surface to migrate.

**Q4 — Loading-state wiring on settings save actions.**

- Options:
  - (chosen) Use `<Button loading={...}>` everywhere an async save is in flight. Disable manual text swaps ("Saving...")
    — parent §4.6 spec's spinner already expresses loading state.
  - Keep existing text swaps. Rejected — sub-spec #2 Button primitive owns spinner + text-preservation contract.
- Reasoning: consumers converge on a single loading UI; avoids per-site "Saving..." strings diverging over time.

## Goals

1. Every interactive button element in the five files renders through `<Button>`.
2. Destructive buttons adopt `variant="danger"`; primary save/action buttons adopt `variant="primary"`;
   secondary/cancel buttons adopt `variant="secondary"`.
3. All `preset-filled-*` / `preset-tonal-*` / `btn-variant-*` classes are removed from `<button>` elements in the
   five files. Non-button Skeleton classes on badge/alert elements (e.g. `preset-tonal-warning` on `<span>` badges,
   `preset-filled-warning` / `preset-filled-surface` on `<aside>` alert containers) are **out of scope** for this PR —
   they are not interactive elements and do not block the button migration goal.
4. All async save actions use `<Button loading>`; no text swaps.

## Non-goals

- Form-input migration — deferred until sub-spec #2b primitives land (tracked as #3c2).
- OIDC provider list/editor buttons — sub-spec #3e.
- Tab routing refactor — SvelteKit routing stays unchanged.
- `SettingKey` backend work — out of frontend scope.

## Scope

Files migrated:

- `frontend/src/routes/settings/+page.svelte` — 5× "Retry All" buttons inside error Callout blocks (lines 258, 266,
  279, 287, 295); each → `<Button variant="primary" size="sm">Retry All</Button>`. Tab navigation is already handled by
  `<TabStrip>` and is not touched.
- `frontend/src/routes/settings/GlobalSettingsTab.svelte` — network settings save + reset, GitHub provider save,
  NATS-URL clear, Zeroconf save, server-certificate renew, CA rotate (destructive launcher). No OIDC launcher exists in
  this file — the OIDC provider list is rendered directly from `+page.svelte` via `<OidcProvidersSettings>`, whose every
  button is migrated entirely by sub-spec #3e. An earlier draft of this spec listed an "Add OIDC provider" launcher and
  SMTP save/reset buttons here; both were stale and have been removed (`GlobalSettingsTab.svelte` contains no SMTP code).
  Non-button Skeleton classes on badge `<span>` elements (lines 437, 513: `preset-tonal-warning`) and `<aside>` alert
  containers (lines 532–533: `preset-filled-warning-500`, `preset-filled-surface-400-600`) are out of scope.
- `frontend/src/routes/settings/AuthenticationSettings.svelte` — save button. **Note:** `isSaving` does not exist in
  this file yet; it must be introduced as `let isSaving = $state(false)` as part of this migration.
- `frontend/src/routes/settings/RegistrationSettings.svelte` — save button; "Generate new token" action if present.
  **Note:** `isSaving` does not exist in this file yet; it must be introduced as `let isSaving = $state(false)` as part
  of this migration.
- `frontend/src/routes/settings/DangerZone.svelte` — three buttons: launcher (`"Reset Data"` →
  `<Button variant="danger">`); inline Cancel inside `{#snippet footer()}` → `<Button variant="secondary">`; inline
  "Reset All Data" confirm inside `{#snippet footer()}` → `<Button variant="danger">`. This file uses a bespoke
  `<Modal>`, not `<ConfirmDialog>` — no #3k boundary applies.

## Migration pattern

Per-button translation rules:

- `preset-filled-primary-*` → `<Button variant="primary">`.
- `preset-tonal-*` with secondary/cancel intent → `<Button variant="secondary">`.
- `preset-tonal-error` → `<Button variant="danger">` (applies to GlobalSettingsTab's NATS Clear button).
- `preset-filled-error-*` → `<Button variant="danger">`.
- Async save buttons: `<Button variant="primary" loading={isSaving} onclick={save}>Save</Button>` — no text swap; the
  spinner sits over the preserved text per parent §4.6.
- Non-button elements carrying `preset-*` classes (badge `<span>`, alert `<aside>`) — **not migrated**; these are
  presentational, not interactive.

## Data flow

Template-level only. No runtime behavior changes. Existing save/cancel/ reset handlers pass through unchanged — only the
rendered button element changes.

## Error handling

- Button primitive's discriminated union catches invalid prop combinations at compile time.
- Save error propagation: existing `fieldErrors` / toast notification pipelines stay untouched; Button merely renders
  visual state.

## Testing

### Unit tests

Extend existing / create new spec files:

- `settings/+page.test.ts` — each of the 5 "Retry All" buttons (inside error Callout blocks) renders as
  `<Button variant="primary" size="sm">`. `<TabStrip>` tab-switching and `aria-selected` state are not re-tested here
  (owned by TabStrip's own test suite).
- `settings/GlobalSettingsTab.test.ts` — network settings save button enters loading when the save handler is in flight.
  GitHub, Zeroconf, and server-certificate renew save buttons each render `variant="primary"` with their own independent
  loading flag. NATS clear button renders `variant="danger"` (destructive state transition) with `loading={natsClearing}`.
  CA rotate launcher renders `variant="danger"` (confirm opens on click). No SMTP save/reset assertions — this file
  contains no SMTP code.
- `settings/AuthenticationSettings.test.ts` — save button renders `variant="primary"` + `loading={isSaving}` wired to
  the component's newly introduced `isSaving` state (`let isSaving = $state(false)` — does not exist in the current
  source and must be added as part of migration).
- `settings/RegistrationSettings.test.ts` — save button variant + loading wiring matching AuthenticationSettings
  (same note: `isSaving` must be introduced); "Generate new token" action (if present in source at migration time)
  renders `variant="primary"` with its own independent loading state.
- `settings/DangerZone.test.ts` — launcher "Reset Data" button renders `variant="danger"`; inline Cancel button in
  `{#snippet footer()}` renders `variant="secondary"`; inline "Reset All Data" confirm button in `{#snippet footer()}`
  renders `variant="danger"`. This file uses a bespoke `<Modal>`, not `<ConfirmDialog>`, so there is no #3k boundary —
  all three buttons are tested here. Disabled-state wiring unchanged.

### Integration / e2e

- Playwright re-baseline `/settings` default tab + each of the four tabs (global, auth, registration, danger), each in
  both dark and light themes. DangerZone is captured with the `<Modal>` closed (idle launcher state) and with it open
  (inline footer buttons visible).
- Delta enumeration per parent §9 (separated by size class, to avoid conflating them in the PR description):
  - "Retry All" buttons (size `sm`): shrink to `h-[19px]`, label `8.5px` uppercase.
  - Save / action / launcher buttons (size `md`): shrink to `h-[23px]`, label `9px` uppercase.
  - DangerZone launcher + inline confirm buttons render the `danger` gradient; inline Cancel renders `secondary` style.
- Toast/error pathway smoke tests unchanged — async save failure still surfaces to toast, not Button.

## Rollout

Single PR titled "feat(frontend): migrate settings shell + auth/registration/danger-zone to Button primitive
(sub-spec #3c)".

1. `frontend/src/routes/settings/+page.svelte` — migrate 5× "Retry All" buttons.
2. `frontend/src/routes/settings/GlobalSettingsTab.svelte` — migrate every non-OIDC-list button.
3. `frontend/src/routes/settings/AuthenticationSettings.svelte` — migrate save/cancel/reset.
4. `frontend/src/routes/settings/RegistrationSettings.svelte` — migrate save + optional token action.
5. `frontend/src/routes/settings/DangerZone.svelte` — migrate every destructive confirm.
6. Extend unit tests per plan.
7. Re-baseline Playwright snapshots for every settings tab.
8. Full frontend gate.

### Risk + rollback

Revert of one PR restores Skeleton preset classes on settings surfaces. Highest-sensitivity surface is Danger Zone —
mitigated by unit tests on destructive variant + Playwright regression gates.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged, sub-spec #2c merged (`variant="secondary"`),
  sub-spec #3b merged (for layout chrome baseline).
- **Blocks:** #3c2 form-input migration (after #2b lands).
- **Parallel-safe with:** sub-spec #3d–k, #4.
