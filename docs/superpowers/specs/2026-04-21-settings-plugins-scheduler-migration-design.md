# Settings Plugins/Scheduler/System-Services Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` (§4.3 Buttons, §4.5 Cards, §4.10 Form
Validation)

**Sub-spec #3d of the UI design-language rollout.** Depends on sub-spec #2 (Button primitive) merged. Form-input sites
defer to a future #3d2 pass after sub-spec #2b primitives land.

## Overview

Migrate five large settings sub-components to the `<Button>` primitive: `PluginConfigsTab.svelte` (1468 lines),
`SchedulerTab.svelte` (190), `SystemServicesSettings.svelte` (406), `EnrollmentTokenSettings.svelte` (450),
`AgentCertificateSettings.svelte` (82). PluginConfigsTab is the largest single file in scope — contains nested
plugin-config editor modals, add/edit/delete plugin-instance actions.

Note: `SystemServicesSettings.svelte` is the **system-scope enrollment token** component (parallel to
`EnrollmentTokenSettings.svelte` which handles tenant-scope tokens). It is mounted inside `GlobalSettingsTab.svelte`.
Its button inventory mirrors `EnrollmentTokenSettings`: load/refresh, create, copy-to-clipboard, cancel modal, create
modal, revoke — not "add/edit/delete services" as the title might suggest.

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

**Q3 — Scheduler enable/disable toggle UI.**

- Options:
  - (chosen) Keep existing toggle widget untouched (it's not a button — it's a switch). Migrate the save/cancel/"Run
    now" buttons only.
  - Migrate switch to a new primitive. Rejected — switches are a separate primitive concern; scope creep.
- Reasoning: switches are not Button primitive territory.

## Goals

1. Every interactive button in the five files renders through `<Button>`.
2. Destructive actions (delete plugin config, revoke enrollment token) adopt `variant="danger"`.
3. Primary save / "Add config" actions adopt `variant="primary"`.
4. "Run now" scheduler action migrates to `<Button variant="ghost" size="sm">` with icon.

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

- `frontend/src/routes/settings/PluginConfigsTab.svelte` — plugin instance list actions, per-plugin modal buttons.
- `frontend/src/routes/settings/SchedulerTab.svelte` — save, cancel, "Run now" per task, retry on load error. Uses
  raw `Modal` (not `ModalShell`) for the edit dialog. `saving: boolean` already exists; `triggeringId: string | null`
  tracks per-row in-flight run.
- `frontend/src/routes/settings/SystemServicesSettings.svelte` — load/refresh, create token, copy-to-clipboard,
  cancel modal, create (modal footer), revoke buttons. System-scope enrollment token component; mirrors
  `EnrollmentTokenSettings.svelte`. Revoke button **not yet migrated** (unlike tenant-scope). `creating: boolean`
  and `copied: boolean` flags already exist; no dedicated copy-loading flag.
- `frontend/src/routes/settings/EnrollmentTokenSettings.svelte` — load/refresh, create token, copy-to-clipboard,
  cancel modal, create (modal footer) buttons. Revoke button **already migrated** to `<Button variant="danger">` in a
  prior pass. `creating: boolean` and `copied: boolean` flags already exist; no `isGenerating`/`isCopying` — those
  names do not appear in the file.
- `frontend/src/routes/settings/AgentCertificateSettings.svelte` — one Save button (`saveCertificates`). Introduce
  `saving: boolean = $state(false)` flag (does not currently exist).

## Migration pattern

Standard translation rules from #3b (preset-filled-primary → primary, preset-filled-error → danger, preset-tonal-\* →
secondary/ghost). Icon-only sites (any unlabeled list-row icon action) use the `leadingIcon` snippet + empty children +
`ariaLabel` prop from sub-spec #2c. No `sr-only` fallback — `ariaLabel` is already available because #2c is a hard
blocker on this sub-spec (see Dependencies).

Special:

- Copy-to-clipboard in `EnrollmentTokenSettings` and `SystemServicesSettings` uses
  `<Button variant="ghost" size="sm">Copy</Button>` — visible text `Copy` provides the accessible name; `ariaLabel`
  omitted. No `loading` prop — clipboard write is synchronous from the perspective of the button (timeout resets
  `copied` flag after 2 s, but the button reverts to "Copy" text, not a spinner).
- `PluginConfigsTab`'s "Add config" per-plugin-type launcher renders as `<Button variant="primary" size="sm">` to match
  card header-action convention.
- `AgentCertificateSettings` has one Save button. Introduce `let saving = $state(false)` in the script block; set
  `true` before `await updateAgentCertificateSettings(...)` and `false` in `finally`. Bind `loading={saving}` on the
  migrated `<Button variant="primary">`.
- `EnrollmentTokenSettings` create action binds `loading={creating}` (flag name already exists). Revoke is already
  migrated. Copy button has no loading state (see above).
- `SystemServicesSettings` create action binds `loading={creating}` (flag name already exists). Revoke button is NOT
  yet migrated — migrate it here with `variant="danger"`.

## Data flow

Template-level only. No runtime behavior changes.

## Error handling

Button discriminated union catches invalid prop combos at compile time. Toast / error pipelines remain unchanged.

## Testing

### Unit tests

Extend / create spec files:

- `PluginConfigsTab.test.ts` — plugin instance action variants; delete action is `variant="danger"`. "Add config"
  launcher renders `variant="primary"` + `size="sm"`. No `loading` prop on "Add config" (synchronous).
- `SchedulerTab.test.ts` — save renders `variant="primary"` + `loading={saving}` (`saving` is the existing flag name);
  cancel renders `variant="secondary"` (no loading); retry on load error renders `variant="primary"`; each "Run now" row
  button renders `variant="ghost"` + `size="sm"` with per-row `loading` tied to `triggeringId === task.id`.
- `SystemServicesSettings.test.ts` — load/refresh renders `variant="secondary"` (or primary for load-tokens initial);
  create token (modal footer) renders `variant="primary"` + `loading={creating}`; revoke renders `variant="danger"`;
  copy renders `variant="ghost"` + `size="sm"` (no loading prop).
- `EnrollmentTokenSettings.test.ts` — create (modal footer) renders `variant="primary"` + `loading={creating}`;
  revoke (already migrated) renders `variant="danger"` (no loading prop — ConfirmDialog intercepts click before
  `handleRevoke` is called); copy renders `variant="ghost"` + `size="sm"` (no loading prop).
- `AgentCertificateSettings.test.ts` — Save renders `variant="primary"` + `loading={saving}`; assert `saving` flag
  introduced by this migration flips `aria-busy="true"` while `updateAgentCertificateSettings` is in-flight and clears
  on resolution.

### Integration / e2e

- Playwright re-baseline `/settings` with each tab active that surfaces one of these components (Plugins, Scheduler,
  System Services, Enrollment Tokens, Agent Certificates), captured in both dark and light themes. Delta enumeration per
  parent §9: button heights shrink to `h-[23px]`; uppercase 9px text; `variant="danger"` renders the error gradient +
  red ring on destructive actions; ghost copy action renders with the `--bg-hover` token on hover.
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
4. `EnrollmentTokenSettings.svelte` — migrate unmigrated buttons (load/refresh, create token, copy, cancel, create modal footer); revoke already done.
5. `AgentCertificateSettings.svelte` — introduce `saving` flag + migrate Save button.
6. Extend unit tests per plan.
7. Re-baseline Playwright snapshots.
8. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset classes across settings plugin admin. Critical workflow path — mitigated by unit
tests + Playwright coverage on token/copy actions.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged, sub-spec #2c merged (`variant="secondary"` + base `ariaLabel` for icon-only
  toggle), sub-spec #3b merged, sub-spec #3c merged (settings shell baseline).
- **Blocks:** #3d2 form-input migration (which depends on both sub-spec #2b Input/Checkbox/Link and sub-spec #2d
  Textarea primitive merged; PluginConfigsTab alone contains 14 textarea sites).
- **Parallel-safe with:** sub-spec #3e–k, #4.
