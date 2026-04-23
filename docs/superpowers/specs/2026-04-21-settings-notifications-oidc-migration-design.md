# Settings Notifications + OIDC Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` (§4.3 Buttons, §4.6 Loading State)

**Sub-spec #3e of the UI design-language rollout.** Depends on sub-spec #2 (Button primitive) merged. Form-input sites
defer to a future #3e2 pass after sub-spec #2b primitives land.

## Overview

Migrate three settings sub-components that own notification channel rules, delivery log, and OIDC provider list:
`NotificationRulesSettings.svelte` (306 lines), `NotificationLogView.svelte` (189), `OidcProvidersSettings.svelte`
(368). No "test channel" / "test connection" buttons exist in the current source — adding them is out of scope for this
migration. The components today render: rules CRUD + pagination (rules), error-state retry only (log view), provider
CRUD + activate/deactivate (OIDC).

## Design decisions

**Q1 — Pagination buttons in NotificationRulesSettings.**

- Options:
  - (chosen) `<Button variant="secondary" size="sm">` for both Previous and Next. Disabled state passes through to the
    primitive's `disabled` prop (primitive renders opacity-40 per #2c).
  - Use `ghost`. Rejected — ghost reads as navigation/link-shape; pager actions are semantic list-state transitions.
- Reasoning: parent §4.3 secondary = reversible side actions; pagination is a side action relative to the main rules
  table.

**Q2 — NotificationLogView error-state Retry.**

- Options:
  - (chosen) `<Button variant="primary">` (no `size` override → default md) with `loading={isRetrying}` bound to a local
    flag flipped true during the `void loadData()` await window. The Retry button is the only recovery path from an
    error boundary — primary matches §4.3 "primary = main action in this context."
  - Secondary. Rejected — secondary implies a reversible side action, but here Retry is the sole action on screen.
- Reasoning: context, not content, drives variant selection. An error boundary's recovery button is the primary action
  of that view.

**Q3 — OIDC provider row Activate/Deactivate.**

- Options:
  - (chosen) Both render as `<Button variant="secondary" size="sm" loading={togglingProviderId === provider.id}>` —
    state-flip actions, reversible. The label switches between "Activate" and "Deactivate" based on `provider.active`;
    the variant does not change. A single per-provider `togglingProviderId` id guard prevents concurrent toggles across
    rows.
  - Use `primary` for Activate / `danger` for Deactivate. Rejected — deactivating an OIDC provider is not destructive
    (no data loss, trivially reversible), so `danger` would over-signal.
- Reasoning: parity with parent §4.3 semantics; activation is not destructive and not the page's primary CTA.

**Q4 — Saving-text swap in rule modal.**

- Options:
  - (chosen) Remove the `{saving ? 'Saving...' : editingRule ? 'Update' : 'Create'}` expression; render
    `<Button variant="primary" loading={saving}>{editingRule ? 'Update' : 'Create'}</Button>`. The Button primitive's
    spinner (parent §4.6) replaces the text-swap. Same contract as #3c Q4.
  - Keep text swap. Rejected — sub-spec #2 + #3c own the loading contract.
- Reasoning: consumers converge on a single loading UI.

## Goals

1. Every interactive button in the three files renders through `<Button>`.
2. Destructive actions (delete rule, delete provider) adopt `variant="danger"`.
3. Rule/provider create + edit launcher and modal save actions adopt `variant="primary"`. Modal save binds
   `loading={saving}`; text-swap "Saving..." expressions removed (Q4).
4. Pagination (Previous/Next) and Edit row actions adopt `variant="secondary" size="sm"`.
5. OIDC Activate/Deactivate toggles adopt `variant="secondary" size="sm"` with per-provider loading flag (Q3).
6. NotificationLogView error-state Retry adopts `variant="primary"` with `loading={isRetrying}` (Q2).

## Non-goals

- Form-input migration — deferred to #3e2.
- Payload JSON viewer refactor — outside Button scope.
- Notification delivery backend — out of scope.

## Scope

Files migrated (button sites enumerated exhaustively against current source; adding buttons not listed here is out of
scope):

- `frontend/src/routes/settings/NotificationRulesSettings.svelte` — "Add Rule" launcher (`variant="primary"`,
  `size="sm"`), per-row "Edit" (`variant="secondary"`, `size="sm"`), per-row "Delete" (`variant="danger"`, `size="sm"`),
  pagination "Previous" / "Next" (`variant="secondary"`, `size="sm"`, passthrough `disabled`), modal "Create" / "Update"
  submit (`variant="primary"`, `loading={saving}`).
- `frontend/src/routes/settings/NotificationLogView.svelte` — single error-state "Retry" (`variant="primary"`,
  `loading={isRetrying}`). The Retry button lives inside `{#snippet errorActions()}` passed to `DataTable` — migrate
  it there. No other interactive buttons exist in the source; row-level retry / view payload / filter / clear are NOT
  added by this sub-spec.
- `frontend/src/routes/settings/OidcProvidersSettings.svelte` — "Add Provider" launcher (`variant="primary"`), per-row
  "Edit" (`variant="secondary"`, `size="sm"`), per-row "Activate"/"Deactivate" toggle (`variant="secondary"`,
  `size="sm"`, `loading` guarded by `togglingProviderId`), per-row "Delete" (`variant="danger"`, `size="sm"`), modal
  "Cancel" (`variant="secondary"`), modal submit (`variant="primary"`, `loading={saving}`, `disabled={!getIsOnline()}`
  passthrough preserved). Modal submit text is `{editingProvider ? 'Update' : 'Create'}` — **not** literal "Save";
  unit tests must use `getByRole('button', { name: 'Create' })` or `{ name: 'Update' }` accordingly.

Scope boundary with #3c: #3c's scope line referencing an "Add OIDC provider" launcher inside `GlobalSettingsTab.svelte`
is factually wrong against the current source — `GlobalSettingsTab.svelte` contains no OIDC launcher; the only "Add
Provider" button lives inside `OidcProvidersSettings.svelte` itself. #3e owns the OIDC component end-to-end with no
cross-component split. #3c's spec should be amended (separate PR) to drop the OIDC launcher claim; until then,
implementers reading #3c should treat that line as stale and defer to this scope block.

## Migration pattern

Per-attribute translation: `preset-filled-primary-500` → `variant="primary"`; `preset-tonal` → `variant="secondary"`;
`preset-tonal-warning` (Deactivate) + `preset-tonal-success` (Activate) → `variant="secondary"` with label switch (Q3);
`preset-filled-error-500` / `preset-tonal-error` → `variant="danger"`; `preset-tonal-surface` (Cancel) →
`variant="secondary"`.

Async wiring:

- `NotificationLogView` Retry: introduce local `let isRetrying = $state(false)`. Handler becomes
  `async () => { isRetrying = true; try { await loadData(); } finally { isRetrying = false; } }`.
- `NotificationRulesSettings` modal save: reuse existing `saving` flag; drop the `{saving ? 'Saving...' : …}` text swap
  per Q4; children render `{editingRule ? 'Update' : 'Create'}` only.
- `OidcProvidersSettings` modal save: introduce `let saving = $state(false)` (flag does not currently exist). Set
  `true` before API call, `false` in `finally`. Bind `loading={saving}` on the Save button.
- `OidcProvidersSettings` toggle: introduce `let togglingProviderId = $state<string | null>(null)`.
  `toggleOidcActive(provider)` wraps its API call with
  `togglingProviderId = provider.id; try { … } finally { togglingProviderId = null; }`. Per-row
  `loading={togglingProviderId === provider.id}`.

Form-input parity note: OidcProvidersSettings renders 3 `<textarea>` sites (scope names / claim mappings / role
mappings) and several `<input>` sites; none migrate in this sub-spec. The re-baselined Playwright snapshot captures them
in their current Skeleton-styled form. Visible mixing of Button-primitive row actions with Skeleton- styled form fields
inside the provider modal is expected and NOT a regression — #3e2 resolves the parity.

## Data flow

Template-level only. Existing test-channel / test-connection handlers call API as before; only the rendered button
element changes.

## Error handling

Button discriminated union catches invalid prop combos at compile time. Test failures still surface to toast; Button
only renders loading state.

## Testing

### Unit tests

- `NotificationRulesSettings.test.ts` — "Add Rule" launcher renders `variant="primary"` + `size="sm"`; per-row Edit
  `variant="secondary"`; per-row Delete `variant="danger"`; pagination Previous/Next render
  `variant="secondary" size="sm"` and reflect the upstream `disabled` prop (both enabled + both disabled +
  only-Next-disabled rendered permutations); modal submit renders `variant="primary"` + `loading={saving}`, flips to
  `aria-busy="true"` while the save handler's promise is unresolved, and children text stays `Create` / `Update`
  throughout (regression guard that the text swap is gone).
- `NotificationLogView.test.ts` — in the error state, the single Retry button renders `variant="primary"`, receives
  `loading={isRetrying}`, and flips between `isRetrying=false → true → false` across the handler's awaited window
  (assert both transitions + `aria-busy` presence during the loading window). No assertions about row-level retry / view
  payload / filter / clear — those buttons do not exist.
- `OidcProvidersSettings.test.ts` — "Add Provider" renders `variant="primary"`; per-row Edit renders
  `variant="secondary" size="sm"`; Activate/Deactivate toggle renders `variant="secondary" size="sm"` with label
  switching on `provider.active`; `loading` is true only on the row whose id matches `togglingProviderId` (render two
  rows, flip one, assert only that row has `aria-busy="true"`); Delete renders `variant="danger" size="sm"`; modal
  Cancel renders `variant="secondary"`; modal submit renders `variant="primary"` + `loading={saving}` + preserves
  `disabled={!getIsOnline()}` when offline; query button by `name: 'Create'` or `name: 'Update'` (not "Save").

### Integration / e2e

- Playwright re-baseline `/settings` on the notifications rules tab, the notifications log tab (idle list + induced
  error state), and the OIDC providers section, each in dark + light themes.
- Delta enumeration per parent §9 (split by size class):
  - `size="sm"` buttons (row actions, pagination, Add Rule launcher): `h-[19px]`, label `8.5px` uppercase.
  - `size="md"` buttons (modal submits, Retry): `h-[23px]`, label `9px` uppercase.
  - `variant="danger"` renders the error gradient on per-row Delete.
  - `variant="secondary"` renders the `--bg-hover` token on hover per #2c.
- Snapshot masking (required to stabilise the visual gate):
  - Mask in-flight spinner rotation on every `<Button loading>` site — forcing `loading=false` in the fixture or masking
    the spinner element directly.
  - Mask transient toast banners raised by save / activate / deactivate / delete flows — button state is the gate, not
    the toast.
  - `NotificationLogView` timestamp cells (`created_at`, `delivered_at`) and row ids are dynamic; mask them in the
    snapshot so only chrome asserts.
- Un-migrated form inputs (OIDC textareas / text fields) appear in the OIDC modal snapshot in their current Skeleton
  style. This is expected per Migration pattern; only row-level button chrome is asserted.

## Rollout

Single PR titled "feat(frontend): migrate settings notifications/oidc to Button primitive (sub-spec #3e)".

1. `NotificationRulesSettings.svelte` — migrate every action.
2. `NotificationLogView.svelte` — migrate row + filter actions.
3. `OidcProvidersSettings.svelte` — migrate provider CRUD + test.
4. Extend unit tests per plan.
5. Re-baseline Playwright snapshots.
6. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset classes across notifications + OIDC. Moderate-sensitivity surface — test-channel action
is the highest-value regression to guard, mitigated by loading-state unit test.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged, sub-spec #2c merged (`variant="secondary"` for test-channel / test-connection),
  sub-spec #3c merged (settings shell baseline).
- **Blocks:** #3e2 form-input migration (depends on #2b + #2d Textarea primitive — OidcProvidersSettings has 3 textarea
  sites).
- **Parallel-safe with:** sub-spec #3d, #3f–k, #4.
