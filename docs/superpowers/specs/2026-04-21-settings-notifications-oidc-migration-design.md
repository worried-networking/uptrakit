# Settings Notifications + OIDC Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`
(§4.3 Buttons, §4.5 Cards)

**Sub-spec #3e of the UI design-language rollout.** Depends on sub-spec #2
(Button primitive) merged. Form-input sites defer to a future #3e2 pass
after sub-spec #2b primitives land.

## Overview

Migrate three settings sub-components that own notification channel rules,
delivery log, and OIDC provider list: `NotificationRulesSettings.svelte`
(306 lines), `NotificationLogView.svelte` (189), `OidcProvidersSettings.svelte`
(368). Notification rules list contains nested "test channel" actions.

## Design decisions

**Q1 — "Test channel" button variant in NotificationRulesSettings.**

- Options:
  - (chosen) `<Button variant="secondary" size="sm" loading={isTesting}>`.
    Test is a reversible read-only action, not a primary save — secondary
    matches the "preview" intent.
  - Use `ghost` variant. Rejected — ghost is for navigation / link-shaped
    actions; test is a semantic action that produces a visible side
    effect (toast).
- Reasoning: parent §4.3 distinguishes primary (main action), secondary
  (reversible side actions), ghost (link/nav).

**Q2 — NotificationLogView row-level buttons.**

- Options:
  - (chosen) Per-row "Retry delivery" action as `<Button variant="secondary"
    size="sm">`; "View payload" as `<Button variant="ghost" size="sm">`.
  - Use icon-only buttons for both. Rejected — log actions benefit from
    text labels (not compact enough to warrant icon-only).
- Reasoning: log-row actions need clarity over compactness; text labels
  reduce user error.

**Q3 — OIDC provider list "Test connection" button.**

- Options:
  - (chosen) `<Button variant="secondary" size="sm" loading={isTesting}>`
    per Q1 pattern.
  - Leave unchanged until a dedicated OIDC sub-spec. Rejected — #3e owns
    this component end-to-end.
- Reasoning: same rationale as notification channel test.

## Goals

1. Every interactive button in the three files renders through `<Button>`.
2. Destructive actions (delete rule, delete provider) adopt
   `variant="danger"`.
3. Test-connection / test-channel actions adopt `variant="secondary"`
   with `loading` prop.
4. Log-view "retry" actions adopt `variant="secondary" size="sm"`.

## Non-goals

- Form-input migration — deferred to #3e2.
- Payload JSON viewer refactor — outside Button scope.
- Notification delivery backend — out of scope.

## Scope

Files migrated:

- `frontend/src/routes/settings/NotificationRulesSettings.svelte` —
  add rule, delete rule, test channel, save.
- `frontend/src/routes/settings/NotificationLogView.svelte` — retry
  delivery, view payload, filter/clear actions.
- `frontend/src/routes/settings/OidcProvidersSettings.svelte` — add
  provider, edit, delete, test connection, save, cancel.

## Migration pattern

Standard translation rules. Test-action variant: `variant="secondary"
size="sm" loading={isTesting}` with existing `onclick` handler.

## Data flow

Template-level only. Existing test-channel / test-connection handlers
call API as before; only the rendered button element changes.

## Error handling

Button discriminated union catches invalid prop combos at compile time.
Test failures still surface to toast; Button only renders loading state.

## Testing

### Unit tests

- `NotificationRulesSettings.test.ts` — add (primary), delete (danger),
  test (secondary + loading), save (primary) variants.
- `NotificationLogView.test.ts` — retry (secondary), view payload
  (ghost), filter / clear variants.
- `OidcProvidersSettings.test.ts` — add (primary), edit (secondary),
  delete (danger), test connection (secondary + loading) variants.

### Integration / e2e

- Playwright re-baseline `/settings` with notifications and OIDC tabs
  active. Delta enumeration: button heights shrink to `h-[23px]`;
  uppercase 9px text; danger variant on destructive row actions.
- Smoke test "Test channel" flow — loading spinner appears, toast fires
  on completion.

## Rollout

Single PR titled
"feat(frontend): migrate settings notifications/oidc to Button primitive (sub-spec #3e)".

1. `NotificationRulesSettings.svelte` — migrate every action.
2. `NotificationLogView.svelte` — migrate row + filter actions.
3. `OidcProvidersSettings.svelte` — migrate provider CRUD + test.
4. Extend unit tests per plan.
5. Re-baseline Playwright snapshots.
6. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset classes across notifications + OIDC.
Moderate-sensitivity surface — test-channel action is the highest-value
regression to guard, mitigated by loading-state unit test.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged, sub-spec #3c merged (settings
  shell baseline).
- **Blocks:** #3e2 form-input migration.
- **Parallel-safe with:** sub-spec #3d, #3f–k, #4.
