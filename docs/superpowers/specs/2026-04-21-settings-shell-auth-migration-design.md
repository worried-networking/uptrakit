# Settings Shell + Auth/Registration/Danger Zone Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` (§3 Layout, §4.3 Buttons, §4.10 Form
Validation)

**Sub-spec #3c of the UI design-language rollout.** Depends on sub-spec #2 (Button primitive) merged. Form-input sites
defer to sub-spec #2b + a future #3c2 pass; this sub-spec migrates buttons only.

## Overview

Migrate the settings shell tab scaffold (`frontend/src/routes/settings/+page.svelte`, 334 lines) and four tab-body
components — `GlobalSettingsTab.svelte` (582), `AuthenticationSettings.svelte` (57), `RegistrationSettings.svelte`
(103), `DangerZone.svelte` (157) — from Skeleton preset-\* button markup to the `<Button>` primitive. `+page.svelte` is
the tab scaffold itself (tab pills, header actions); the four components render inside it per active tab.

## Design decisions

**Q1 — Tab pill migration: `<Button>` or bespoke link primitive?**

- Options:
  - (chosen) `<Button variant="ghost" size="sm">` for inactive pills and
    `<Button variant="ghost" size="sm" class="text-[var(--accent)] bg-[var(--bg-hover)]">` for the active pill —
    identical active-state contract to the #3b navbar-pill pattern (accent text
    - raised background via the #2c `--bg-hover` token).
  - Introduce `<TabPill>` primitive. Rejected — same YAGNI argument as the navbar-pill case in #3b; one consumer shape
    today.
- Reasoning: cross-surface consistency (navbar and tab pills share the same ghost + accent-override pattern) reduces
  cognitive load.

**Q2 — Danger Zone destructive actions.**

- Options:
  - (chosen) `<Button variant="danger">` for every destructive confirm button; per parent §4.3 this renders the error
    gradient + red ring.
  - Keep existing `preset-filled-error` class. Rejected — baseline migration.
- Reasoning: Danger Zone is the canonical destructive surface; landing variant="danger" here anchors the visual
  definition for every later destructive confirm (#3k modals reuse it).

**Q5 — DangerZone vs ConfirmDialog migration boundary.**

- Options:
  - (chosen) `DangerZone.svelte` owns the _launcher_ buttons that open a destructive confirmation (e.g. "Delete all
    hosts", "Rotate CA"). Those launchers migrate to `<Button variant="danger">` here. The `<ConfirmDialog>` primitive
    itself — its internal confirm + cancel buttons, and any caller that uses it — is migrated by sub-spec #3k (shared
    modals + dialogs). DangerZone's template still invokes `<ConfirmDialog ...>`; the dialog's internals are #3k's job.
  - Migrate ConfirmDialog internals here. Rejected — #3k already owns that primitive. Double-migrating the confirm
    button would force two PRs to touch the same code.
- Reasoning: each surface migrates its own buttons; primitive components are owned by their own sub-specs. No
  double-migration risk, but spec clarity prevents an implementer from reaching into ConfirmDialog.

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

1. Every interactive button in the five files renders through `<Button>`.
2. Destructive buttons adopt `variant="danger"`; tab pills adopt `variant="ghost"`; primary save actions adopt
   `variant="primary"`.
3. Delete `preset-filled-*` / `preset-tonal-*` / `btn-variant-*` attributes from the five files.
4. All async save actions use `<Button loading>`; no text swaps.

## Non-goals

- Form-input migration — deferred until sub-spec #2b primitives land (tracked as #3c2).
- OIDC provider list/editor buttons — sub-spec #3e.
- Tab routing refactor — SvelteKit routing stays unchanged.
- `SettingKey` backend work — out of frontend scope.

## Scope

Files migrated:

- `frontend/src/routes/settings/+page.svelte` — tab scaffold, tab pills, optional header-level actions.
- `frontend/src/routes/settings/GlobalSettingsTab.svelte` — global SMTP settings save + reset, network settings save +
  reset, GitHub provider save, NATS-URL clear, Zeroconf save, server-certificate renew, CA rotate (destructive
  launcher). No OIDC launcher exists in this file against the current source — the OIDC provider list is rendered
  directly from `+page.svelte` via `<OidcProvidersSettings>`, whose every button (including "Add Provider") is migrated
  entirely by sub-spec #3e. An earlier draft of this spec listed an "Add OIDC provider" launcher here; that was stale
  and has been removed.
- `frontend/src/routes/settings/AuthenticationSettings.svelte` — save / cancel / reset for auth config.
- `frontend/src/routes/settings/RegistrationSettings.svelte` — registration- mode toggle save; "Generate new token"
  action if present.
- `frontend/src/routes/settings/DangerZone.svelte` — every destructive confirm button.

## Migration pattern

Per-button translation rules:

- `preset-filled-primary-*` → `<Button variant="primary">`.
- `preset-tonal-*` with secondary intent → `<Button variant="secondary">`.
- `preset-filled-error-*` → `<Button variant="danger">`.
- Ghost tab pills → `<Button variant="ghost" size="sm">`; active pill gets
  `class="text-[var(--accent)] bg-[var(--bg-hover)]"` override (matches #3b navbar pattern).
- Async save buttons: `<Button variant="primary" loading={isSaving} onclick={save}>Save</Button>` — no text swap; the
  spinner sits over the preserved text per parent §4.6.

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

- `settings/+page.test.ts` — tab pill variant assertions (ghost + sm); active pill carries both `text-[var(--accent)]`
  and `bg-[var(--bg-hover)]` class fragments, inactive pills carry neither; tab switch preserves the expected
  `aria-selected` state; href-branch assertion for tabs that route via URL vs onclick-only.
- `settings/GlobalSettingsTab.test.ts` — SMTP save button and network settings save button each enter loading when their
  individual save handler is in flight (each save has its own local `isSaving` state — reset independently). Reset
  buttons render `variant="secondary"`. GitHub, Zeroconf, and server-certificate renew save buttons each render
  `variant="primary"` with their own independent loading flag. NATS clear button renders `variant="danger"` (destructive
  state transition) with `loading={natsClearing}`. CA rotate launcher renders `variant="danger"` (confirms open on
  click).
- `settings/AuthenticationSettings.test.ts` — save button renders `variant="primary"` + `loading={isSaving}` wired to
  the component's `isSaving` state; cancel button renders `variant="secondary"`.
- `settings/RegistrationSettings.test.ts` — save button variant + loading wiring matching AuthenticationSettings;
  "Generate new token" action (if present in source at migration time) renders `variant="primary"` with its own
  independent loading state.
- `settings/DangerZone.test.ts` — every _launcher_ button that opens a `<ConfirmDialog>` renders `variant="danger"`; the
  confirm + cancel buttons rendered inside `<ConfirmDialog>` itself are deliberately untested here (covered by #3k's own
  test plan). Disabled-state wiring unchanged.

### Integration / e2e

- Playwright re-baseline `/settings` default tab + each of the four tabs (global, auth, registration, danger), each in
  both dark and light themes. DangerZone is captured in its idle state only — the `<ConfirmDialog>` is not opened during
  the snapshot, because the dialog itself is owned by sub-spec #3k's re-baseline pass.
- Delta enumeration per parent §9 (separated by size class, to avoid conflating them in the PR description):
  - Tab pills (size `sm`): shrink to `h-[19px]`, label `8.5px` uppercase, active state adds `--bg-hover` background.
  - Save / action / launcher buttons (size `md`): shrink to `h-[23px]`, label `9px` uppercase.
  - DangerZone launcher buttons render the `danger` gradient; no change inside the dialog body (owned by #3k).
- Toast/error pathway smoke tests unchanged — async save failure still surfaces to toast, not Button.

## Rollout

Single PR titled "feat(frontend): migrate settings shell + auth/registration/danger-zone to Button primitive
(sub-spec #3c)".

1. `frontend/src/routes/settings/+page.svelte` — migrate tab pills and header actions.
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

- **Blocks on:** sub-spec #2 merged, sub-spec #2c merged (`variant="secondary"` + `--bg-hover` for active-tab override),
  sub-spec #3b merged (for layout chrome baseline).
- **Blocks:** #3c2 form-input migration (after #2b lands).
- **Parallel-safe with:** sub-spec #3d–k, #4.
