# Settings Shell + Auth/Registration/Danger Zone Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`
(§3 Layout, §4.3 Buttons, §4.10 Form Validation)

**Sub-spec #3c of the UI design-language rollout.** Depends on sub-spec #2
(Button primitive) merged. Form-input sites defer to sub-spec #2b + a
future #3c2 pass; this sub-spec migrates buttons only.

## Overview

Migrate the settings shell tab scaffold (`frontend/src/routes/settings/+page.svelte`,
334 lines) and four tab-body components — `GlobalSettingsTab.svelte` (582),
`AuthenticationSettings.svelte` (57), `RegistrationSettings.svelte` (103),
`DangerZone.svelte` (157) — from Skeleton preset-* button markup to the
`<Button>` primitive. `+page.svelte` is the tab scaffold itself (tab pills,
header actions); the four components render inside it per active tab.

## Design decisions

**Q1 — Tab pill migration: `<Button>` or bespoke link primitive?**

- Options:
  - (chosen) `<Button variant="ghost" size="sm">` for inactive pills and
    `<Button variant="ghost" size="sm" class="text-[var(--accent)]">` for
    the active pill. Matches the #3b navbar-pill pattern.
  - Introduce `<TabPill>` primitive. Rejected — same YAGNI argument as
    the navbar-pill case in #3b; one consumer shape today.
- Reasoning: cross-surface consistency (navbar and tab pills share the
  same ghost + accent-override pattern) reduces cognitive load.

**Q2 — Danger Zone destructive actions.**

- Options:
  - (chosen) `<Button variant="danger">` for every destructive confirm
    button; per parent §4.3 this renders the error gradient + red ring.
  - Keep existing `preset-filled-error` class. Rejected — baseline migration.
- Reasoning: Danger Zone is the canonical destructive surface; landing
  variant="danger" here anchors the visual definition for every later
  destructive confirm (#3k modals reuse it).

**Q3 — OIDC provider list buttons inside GlobalSettingsTab.**

- Options:
  - (chosen) Defer OIDC-specific buttons to sub-spec #3e (which owns
    the `OidcProviders` component). `GlobalSettingsTab` will still own
    the "Add OIDC provider" launcher button — migrate that here.
  - Migrate everything OIDC-related here. Rejected — scope bleed across
    sub-spec boundaries.
- Reasoning: #3e owns the provider list component outright; #3c touches
  only what's in the GlobalSettingsTab wrapper itself.

**Q4 — Loading-state wiring on settings save actions.**

- Options:
  - (chosen) Use `<Button loading={...}>` everywhere an async save is in
    flight. Disable manual text swaps ("Saving...") — parent §4.6 spec's
    spinner already expresses loading state.
  - Keep existing text swaps. Rejected — sub-spec #2 Button primitive
    owns spinner + text-preservation contract.
- Reasoning: consumers converge on a single loading UI; avoids per-site
  "Saving..." strings diverging over time.

## Goals

1. Every interactive button in the five files renders through `<Button>`.
2. Destructive buttons adopt `variant="danger"`; tab pills adopt
   `variant="ghost"`; primary save actions adopt `variant="primary"`.
3. Delete `preset-filled-*` / `preset-tonal-*` / `btn-variant-*` attributes
   from the five files.
4. All async save actions use `<Button loading>`; no text swaps.

## Non-goals

- Form-input migration — deferred until sub-spec #2b primitives land
  (tracked as #3c2).
- OIDC provider list/editor buttons — sub-spec #3e.
- Tab routing refactor — SvelteKit routing stays unchanged.
- `SettingKey` backend work — out of frontend scope.

## Scope

Files migrated:

- `frontend/src/routes/settings/+page.svelte` — tab scaffold, tab pills,
  optional header-level actions.
- `frontend/src/routes/settings/GlobalSettingsTab.svelte` — global
  SMTP/network settings save actions, reset buttons, "Add OIDC provider"
  launcher.
- `frontend/src/routes/settings/AuthenticationSettings.svelte` — save /
  cancel / reset for auth config.
- `frontend/src/routes/settings/RegistrationSettings.svelte` — registration-
  mode toggle save; "Generate new token" action if present.
- `frontend/src/routes/settings/DangerZone.svelte` — every destructive
  confirm button.

## Migration pattern

Per-button translation rules:

- `preset-filled-primary-*` → `<Button variant="primary">`.
- `preset-tonal-*` with secondary intent → `<Button variant="secondary">`.
- `preset-filled-error-*` → `<Button variant="danger">`.
- Ghost tab pills → `<Button variant="ghost" size="sm">`; active pill gets
  `class="text-[var(--accent)]"` override.
- Async save buttons: `<Button variant="primary" loading={isSaving}
  onclick={save}>Save</Button>` — no text swap; the spinner sits over the
  preserved text per parent §4.6.

## Data flow

Template-level only. No runtime behavior changes. Existing save/cancel/
reset handlers pass through unchanged — only the rendered button element
changes.

## Error handling

- Button primitive's discriminated union catches invalid prop
  combinations at compile time.
- Save error propagation: existing `fieldErrors` / toast notification
  pipelines stay untouched; Button merely renders visual state.

## Testing

### Unit tests

Extend existing / create new spec files:

- `settings/+page.test.ts` — tab pill variant assertions; active pill
  carries accent-text override; tab switch preserves expected aria state.
- `settings/GlobalSettingsTab.test.ts` — save button enters loading when
  save in flight; renders danger variant on no buttons (none expected here).
- `settings/AuthenticationSettings.test.ts` — save button variant;
  loading wiring.
- `settings/RegistrationSettings.test.ts` — save button variant;
  loading wiring; token-generation action (if present).
- `settings/DangerZone.test.ts` — every destructive confirm button
  renders `variant="danger"`; disabled-state wiring unchanged.

### Integration / e2e

- Playwright re-baseline `/settings` default tab + each tab variant
  (global, auth, registration, danger). Delta enumeration: buttons
  shrink to `h-[23px]`; uppercase 9px text; danger-zone red gradient.
- Toast/error pathway smoke tests unchanged — async save failure still
  surfaces to toast, not Button.

## Rollout

Single PR titled
"feat(frontend): migrate settings shell + auth/registration/danger-zone to Button primitive (sub-spec #3c)".

1. `frontend/src/routes/settings/+page.svelte` — migrate tab pills and
   header actions.
2. `frontend/src/routes/settings/GlobalSettingsTab.svelte` — migrate
   every non-OIDC-list button.
3. `frontend/src/routes/settings/AuthenticationSettings.svelte` —
   migrate save/cancel/reset.
4. `frontend/src/routes/settings/RegistrationSettings.svelte` — migrate
   save + optional token action.
5. `frontend/src/routes/settings/DangerZone.svelte` — migrate every
   destructive confirm.
6. Extend unit tests per plan.
7. Re-baseline Playwright snapshots for every settings tab.
8. Full frontend gate.

### Risk + rollback

Revert of one PR restores Skeleton preset classes on settings surfaces.
Highest-sensitivity surface is Danger Zone — mitigated by unit tests on
destructive variant + Playwright regression gates.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged, sub-spec #3b merged (for layout
  chrome baseline).
- **Blocks:** #3c2 form-input migration (after #2b lands).
- **Parallel-safe with:** sub-spec #3d–k, #4.
