# Settings Plugins/Scheduler/System-Services Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` (§4.3 Buttons, §4.5 Cards, §4.10 Form
Validation)

**Sub-spec #3d of the UI design-language rollout.** Depends on sub-spec #2 (Button primitive) merged. Form-input sites
defer to a future #3d2 pass after sub-spec #2b primitives land.

## Overview

Migrate five large settings sub-components to the `<Button>` primitive: `PluginConfigsTab.svelte` (1468 lines),
`SchedulerTab.svelte` (190), `SystemServicesSettings.svelte` (406), `EnrollmentTokenSettings.svelte` (450),
`AgentCertificateSettings.svelte` (82). PluginConfigsTab is the largest single file in scope — contains nested
plugin-config editor modals, add/edit/delete plugin-instance actions, secret-masking toggle.

## Design decisions

**Q1 — PluginConfigsTab scope: single PR or split by tab-body section.**

- Options:
  - (chosen) Single PR covering all five files. File size is large but the per-button translation is mechanical
    (preset-\* → variant). Splitting adds PR overhead without reducing risk.
  - Split PluginConfigsTab into its own PR. Rejected — the file is big but the diff is narrow (attribute-level);
    Playwright baseline covers regression risk.
- Reasoning: mechanical migration + visual regression gate + bisect- friendly commit per file inside the PR.

**Q2 — Modal action buttons: migrate here or defer to #3k.**

- Options:
  - (chosen) Migrate inline modal actions (e.g. "Add plugin config", "Save", "Cancel" inside per-plugin modal). Modal
    _shell_ (ConfirmDialog, BatchResultDialog) still deferred to #3k — this sub-spec only touches actions rendered by
    these five files directly.
  - Defer every modal action to #3k. Rejected — would leave buttons inside this file half-migrated; bad state for
    interim review.
- Reasoning: own the buttons rendered in-file; defer only the shared modal primitive wrapping logic.

**Q3 — Secret-masking toggle in PluginConfigsTab.**

- Options:
  - (chosen) `<Button variant="ghost" size="sm">` with `leadingIcon` snippet for eye/eye-off icon; text-only label.
  - Introduce toggle primitive. Rejected — same YAGNI as #3b theme toggle.
- Reasoning: Button primitive already supports icon + text; one consumer.

**Q4 — Scheduler enable/disable toggle UI.**

- Options:
  - (chosen) Keep existing toggle widget untouched (it's not a button — it's a switch). Migrate the save/cancel/"Run
    now" buttons only.
  - Migrate switch to a new primitive. Rejected — switches are a separate primitive concern; scope creep.
- Reasoning: switches are not Button primitive territory.

## Goals

1. Every interactive button in the five files renders through `<Button>`.
2. Destructive actions (delete plugin config, revoke enrollment token) adopt `variant="danger"`.
3. Primary save / "Add config" actions adopt `variant="primary"`.
4. Secret-masking toggle + "Run now" scheduler action migrate to `<Button variant="ghost" size="sm">` with icons.

## Non-goals

- Form-input migration — deferred to #3d2. Explicitly out of scope: the 14 `<textarea>` sites in
  `PluginConfigsTab.svelte`, any `<input>` / `<Checkbox>` / inline `<a>` sites across the five files. Implementers must
  not touch those elements in this PR; they belong to #3d2 (which blocks on #2b Input/Checkbox/Link and #2d Textarea).
- Switch / toggle widget migration — outside Button scope.
- Modal shell (ConfirmDialog etc.) — sub-spec #3k.
- Plugin-config schema runtime changes — backend scope.
- Scheduler logic — backend scope.

## Scope

Files migrated:

- `frontend/src/routes/settings/PluginConfigsTab.svelte` — plugin instance list actions, per-plugin modal buttons,
  secret-mask toggle.
- `frontend/src/routes/settings/SchedulerTab.svelte` — save, cancel, "Run now" per task, reset.
- `frontend/src/routes/settings/SystemServicesSettings.svelte` — add, edit, delete, save, cancel buttons.
- `frontend/src/routes/settings/EnrollmentTokenSettings.svelte` — generate, revoke, copy-to-clipboard buttons.
- `frontend/src/routes/settings/AgentCertificateSettings.svelte` — rotate / regenerate / download actions.

## Migration pattern

Standard translation rules from #3b (preset-filled-primary → primary, preset-filled-error → danger, preset-tonal-\* →
secondary/ghost). Icon-only sites (secret-mask toggle, any unlabeled list-row icon action) use the `leadingIcon`
snippet + empty children + `ariaLabel` prop from sub-spec #2c. No `sr-only` fallback — `ariaLabel` is already available
because #2c is a hard blocker on this sub-spec (see Dependencies).

Special:

- Copy-to-clipboard in `EnrollmentTokenSettings` uses
  `<Button variant="ghost" size="sm" leadingIcon={CopyIcon}>Copy</Button>` — visible text `Copy` provides the accessible
  name; `ariaLabel` omitted. Existing copy-to-clipboard handler unchanged. `loading` prop bound to an ephemeral
  `isCopying` state (true for the brief window between click and clipboard-API resolve) so the spinner renders even
  though the operation is fast; avoids double-click thrash.
- `PluginConfigsTab`'s "Add config" per-plugin-type launcher renders as `<Button variant="primary" size="sm">` to match
  card header-action convention.
- `PluginConfigsTab`'s secret-mask toggle binds its `leadingIcon` and `children` to the current `masked` state: when
  `masked=true`, render the eye icon with children `Show`; when `masked=false`, render the eye-off icon with children
  `Hide`. `ariaLabel` is not required — children text carries the accessible name and state transition.
- `AgentCertificateSettings` `rotate` and `regenerate` actions bind `loading={isRotating}` and
  `loading={isRegenerating}` respectively; `download` is synchronous and does not need a loading state.
- `EnrollmentTokenSettings` `generate` and `revoke` actions each bind their own component-local `isGenerating` /
  `isRevoking` flag to `loading`.

## Data flow

Template-level only. No runtime behavior changes.

## Error handling

Button discriminated union catches invalid prop combos at compile time. Toast / error pipelines remain unchanged.

## Testing

### Unit tests

Extend / create spec files:

- `PluginConfigsTab.test.ts` — plugin instance action variants; delete action is `variant="danger"`. Secret-mask toggle:
  when `masked=true` renders eye icon + children text `Show`; when `masked=false` renders eye-off icon + children text
  `Hide`; toggling the bound state flips both fragments in a single re-render (no `ariaLabel` asserted — accessible name
  comes from children). "Add config" launcher renders `variant="primary"` + `size="sm"`. No `loading` prop wired on the
  secret-mask toggle or "Add config" launcher (both synchronous).
- `SchedulerTab.test.ts` — save renders `variant="primary"` + `loading={isSaving}`; cancel renders `variant="secondary"`
  (no loading); each "Run now" row button renders `variant="ghost"` + `size="sm"` with its own per-row `loading` flag
  that flips true during dispatch and back to false on resolution (assert both transitions).
- `SystemServicesSettings.test.ts` — every action variant: add/edit `variant="primary"`, delete `variant="danger"`, save
  `variant="primary"` with `loading={isSaving}`, cancel `variant="secondary"`.
- `EnrollmentTokenSettings.test.ts` — generate renders `variant="primary"`
  - `loading={isGenerating}` (assert flag flips true on click then false on resolution); revoke renders
    `variant="danger"` + `loading={isRevoking}` with the same transition assertion; copy renders `variant="ghost"` +
    `size="sm"` + `loading={isCopying}` and the ephemeral `isCopying` flag is true for the awaited window of the
    clipboard write and false once the promise resolves (assert both states via fake clipboard shim).
- `AgentCertificateSettings.test.ts` — rotate renders `variant="primary"`
  - `loading={isRotating}`; regenerate renders `variant="primary"` + `loading={isRegenerating}`; download renders
    `variant="secondary"` without a `loading` prop (synchronous browser download).

### Integration / e2e

- Playwright re-baseline `/settings` with each tab active that surfaces one of these components (Plugins, Scheduler,
  System Services, Enrollment Tokens, Agent Certificates), captured in both dark and light themes. Delta enumeration per
  parent §9: button heights shrink to `h-[23px]`; uppercase 9px text; `variant="danger"` renders the error gradient +
  red ring on destructive actions; ghost icon-only actions (secret-mask, copy) render with the `--bg-hover` token on
  hover.
- Snapshot masking (to keep the visual gate stable despite stochastic UI state):
  - Mask in-flight spinner rotation inside any `loading` button — fixture takes the snapshot with every `loading` flag
    forced `false`, or masks the spinner element outright if forcing is not available from the tab fixture.
  - Mask transient toast/notification banners surfaced by generate/revoke/rotate/regenerate handlers — the button state
    is the gate, not the toast body.
  - Mask dynamic token strings (enrollment token value) — only surrounding button chrome is asserted.
- Test token generation, copy, revoke flows via existing Playwright scenarios — button behavior unchanged; assert that
  the button's `aria-busy="true"` is observable during the awaited handler window.

## Rollout

Single PR titled "feat(frontend): migrate settings plugins/scheduler/system-services to Button primitive
(sub-spec #3d)".

Commit granularity: one commit per file so a bisect can isolate a visual regression to the specific migrated surface.
The 1468-line `PluginConfigsTab.svelte` commit is the largest in the PR but still a single commit — the diff is
attribute-level (preset-\* → variant) across independent button sites, so intra-file subdivision would not produce
meaningfully smaller bisect windows. Commit 6 bundles unit- test extensions. Commit 7 re-baselines Playwright snapshots.

1. `PluginConfigsTab.svelte` — migrate list + modal action buttons.
2. `SchedulerTab.svelte` — migrate save/cancel/"Run now".
3. `SystemServicesSettings.svelte` — migrate every action.
4. `EnrollmentTokenSettings.svelte` — migrate generate/revoke/copy.
5. `AgentCertificateSettings.svelte` — migrate rotate/regenerate/download.
6. Extend unit tests per plan.
7. Re-baseline Playwright snapshots.
8. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset classes across settings plugin admin. Critical workflow path — mitigated by unit
tests + Playwright coverage on token/copy/secret-mask actions.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged, sub-spec #2c merged (`variant="secondary"` + base `ariaLabel` for icon-only
  toggle), sub-spec #3b merged, sub-spec #3c merged (settings shell baseline).
- **Blocks:** #3d2 form-input migration (which depends on both sub-spec #2b Input/Checkbox/Link and sub-spec #2d
  Textarea primitive merged; PluginConfigsTab alone contains 14 textarea sites).
- **Parallel-safe with:** sub-spec #3e–k, #4.
