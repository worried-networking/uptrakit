# Settings Plugins/Scheduler/System-Services Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`
(§4.3 Buttons, §4.5 Cards, §4.10 Form Validation)

**Sub-spec #3d of the UI design-language rollout.** Depends on sub-spec #2
(Button primitive) merged. Form-input sites defer to a future #3d2 pass
after sub-spec #2b primitives land.

## Overview

Migrate five large settings sub-components to the `<Button>` primitive:
`PluginConfigsTab.svelte` (1468 lines), `SchedulerTab.svelte` (190),
`SystemServicesSettings.svelte` (406), `EnrollmentTokenSettings.svelte`
(450), `AgentCertificateSettings.svelte` (82). PluginConfigsTab is the
largest single file in scope — contains nested plugin-config editor
modals, add/edit/delete plugin-instance actions, secret-masking toggle.

## Design decisions

**Q1 — PluginConfigsTab scope: single PR or split by tab-body section.**

- Options:
  - (chosen) Single PR covering all five files. File size is large but
    the per-button translation is mechanical (preset-* → variant).
    Splitting adds PR overhead without reducing risk.
  - Split PluginConfigsTab into its own PR. Rejected — the file is big
    but the diff is narrow (attribute-level); Playwright baseline covers
    regression risk.
- Reasoning: mechanical migration + visual regression gate + bisect-
  friendly commit per file inside the PR.

**Q2 — Modal action buttons: migrate here or defer to #3k.**

- Options:
  - (chosen) Migrate inline modal actions (e.g. "Add plugin config",
    "Save", "Cancel" inside per-plugin modal). Modal *shell* (ConfirmDialog,
    BatchResultDialog) still deferred to #3k — this sub-spec only touches
    actions rendered by these five files directly.
  - Defer every modal action to #3k. Rejected — would leave buttons
    inside this file half-migrated; bad state for interim review.
- Reasoning: own the buttons rendered in-file; defer only the shared
  modal primitive wrapping logic.

**Q3 — Secret-masking toggle in PluginConfigsTab.**

- Options:
  - (chosen) `<Button variant="ghost" size="sm">` with `leadingIcon`
    snippet for eye/eye-off icon; text-only label.
  - Introduce toggle primitive. Rejected — same YAGNI as #3b theme toggle.
- Reasoning: Button primitive already supports icon + text; one consumer.

**Q4 — Scheduler enable/disable toggle UI.**

- Options:
  - (chosen) Keep existing toggle widget untouched (it's not a button — it's
    a switch). Migrate the save/cancel/"Run now" buttons only.
  - Migrate switch to a new primitive. Rejected — switches are a separate
    primitive concern; scope creep.
- Reasoning: switches are not Button primitive territory.

## Goals

1. Every interactive button in the five files renders through `<Button>`.
2. Destructive actions (delete plugin config, revoke enrollment token)
   adopt `variant="danger"`.
3. Primary save / "Add config" actions adopt `variant="primary"`.
4. Secret-masking toggle + "Run now" scheduler action migrate to
   `<Button variant="ghost" size="sm">` with icons.

## Non-goals

- Form-input migration — deferred to #3d2.
- Switch / toggle widget migration — outside Button scope.
- Modal shell (ConfirmDialog etc.) — sub-spec #3k.
- Plugin-config schema runtime changes — backend scope.
- Scheduler logic — backend scope.

## Scope

Files migrated:

- `frontend/src/routes/settings/PluginConfigsTab.svelte` — plugin
  instance list actions, per-plugin modal buttons, secret-mask toggle.
- `frontend/src/routes/settings/SchedulerTab.svelte` — save, cancel,
  "Run now" per task, reset.
- `frontend/src/routes/settings/SystemServicesSettings.svelte` — add,
  edit, delete, save, cancel buttons.
- `frontend/src/routes/settings/EnrollmentTokenSettings.svelte` —
  generate, revoke, copy-to-clipboard buttons.
- `frontend/src/routes/settings/AgentCertificateSettings.svelte` —
  rotate / regenerate / download actions.

## Migration pattern

Standard translation rules from #3b (preset-filled-primary → primary,
preset-filled-error → danger, preset-tonal-* → secondary/ghost).
Icon-only sites use `leadingIcon` snippet plus `sr-only` label until the
Button `ariaLabel` primitive update lands.

Special:

- Copy-to-clipboard in `EnrollmentTokenSettings` uses
  `<Button variant="ghost" size="sm" leadingIcon={CopyIcon}>Copy</Button>`
  — existing copy-to-clipboard handler unchanged.
- `PluginConfigsTab`'s "Add config" per-plugin-type launcher renders as
  `<Button variant="primary" size="sm">` to match card header-action
  convention.

## Data flow

Template-level only. No runtime behavior changes.

## Error handling

Button discriminated union catches invalid prop combos at compile time.
Toast / error pipelines remain unchanged.

## Testing

### Unit tests

Extend / create spec files:

- `PluginConfigsTab.test.ts` — plugin instance action variants; secret-
  mask toggle renders as ghost + icon; delete action is danger.
- `SchedulerTab.test.ts` — save, cancel, "Run now" variants.
- `SystemServicesSettings.test.ts` — every action variant; delete is
  danger.
- `EnrollmentTokenSettings.test.ts` — generate (primary), revoke
  (danger), copy (ghost + icon).
- `AgentCertificateSettings.test.ts` — rotate / regenerate / download
  variants.

### Integration / e2e

- Playwright re-baseline `/settings` with each tab active that surfaces
  one of these components. Delta enumeration: button heights shrink to
  `h-[23px]`; uppercase 9px text; danger variant on destructive actions.
- Test token generation, copy, revoke flows via existing Playwright
  scenarios — button behavior unchanged.

## Rollout

Single PR titled
"feat(frontend): migrate settings plugins/scheduler/system-services to Button primitive (sub-spec #3d)".

1. `PluginConfigsTab.svelte` — migrate list + modal action buttons.
2. `SchedulerTab.svelte` — migrate save/cancel/"Run now".
3. `SystemServicesSettings.svelte` — migrate every action.
4. `EnrollmentTokenSettings.svelte` — migrate generate/revoke/copy.
5. `AgentCertificateSettings.svelte` — migrate rotate/regenerate/download.
6. Extend unit tests per plan.
7. Re-baseline Playwright snapshots.
8. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset classes across settings plugin admin.
Critical workflow path — mitigated by unit tests + Playwright coverage
on token/copy/secret-mask actions.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged, sub-spec #2c merged
  (`variant="secondary"` + base `ariaLabel` for icon-only toggle),
  sub-spec #3b merged, sub-spec #3c merged (settings shell baseline).
- **Blocks:** #3d2 form-input migration (which depends on both
  sub-spec #2b Input/Checkbox/Link and sub-spec #2d Textarea primitive
  merged; PluginConfigsTab alone contains 14 textarea sites).
- **Parallel-safe with:** sub-spec #3e–k, #4.
