# Host-Tags + Audit-Logs + Profile Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`
(§4.3 Buttons, §4.5 Cards, §4.7 Tables)

**Sub-spec #3j of the UI design-language rollout.** Depends on sub-spec #2
(Button primitive) merged. Form-input sites defer to a future #3j2 pass
after sub-spec #2b primitives land.

## Overview

Migrate three lower-frequency admin routes: `/host-tags/+page.svelte`
(537 lines — tag CRUD + color picker launcher), `/audit-logs/+page.svelte`
(322 — audit log list with filters + export trigger), `/profile/+page.svelte`
(219 — current user profile, password change launcher, API token list).

## Design decisions

**Q1 — Host-tag color picker launcher.**

- Options:
  - (chosen) `<Button variant="ghost" size="sm" class="p-0 w-5 h-5
    rounded-[3px]" style="background-color: {tagColor}">` — a colored
    square swatch that opens the picker on click. Size override is
    acceptable because swatches are a known shape.
  - Introduce `<ColorSwatch>` primitive. Rejected — one consumer today.
  - Keep existing `<div role="button">`. Rejected — accessibility:
    button role without button semantics.
- Reasoning: use Button with size override + inline background style; a11y
  comes for free (native button keyboard + focus). The dimension override
  is local to this use — documented in call site via comment.

**Q2 — Audit log "Export" trigger shape.**

- Options:
  - (chosen) `<Button variant="secondary" size="sm" leadingIcon={DownloadIcon}
    loading={isExporting}>Export CSV</Button>`. Export is a
    side-effect-free action worth showing but not the primary page
    action.
  - `<Button variant="primary">`. Rejected — export is auxiliary.
- Reasoning: secondary expresses "useful but not main" per parent §4.3.

**Q3 — Profile API token list row actions.**

- Options:
  - (chosen) Generate: `<Button variant="primary" size="sm">`;
    Copy: `<Button variant="ghost" size="sm" leadingIcon={CopyIcon}>`;
    Revoke: `<Button variant="danger" size="sm">`.
  - Same variants, collapse copy into revoke row. Rejected — copy and
    revoke are distinct intents.
- Reasoning: token-row actions map cleanly to primary (create),
  ghost (side action), danger (destructive).

**Q4 — Profile "Change password" launcher.**

- Options:
  - (chosen) `<Button variant="secondary" size="sm">Change password</Button>`
    — opens a modal. Modal shell handled by #3k.
  - `<Button variant="primary">`. Rejected — password change is not the
    primary action on the profile page; account viewing is.
- Reasoning: primary variant should remain reserved for the main page
  workflow; secondary for side-action launchers.

## Goals

1. Every interactive button in the three files renders through
   `<Button>`.
2. Color swatches on host-tags render as `<Button>` with size override
   plus inline background style.
3. Export / password-change launchers adopt `variant="secondary"`.
4. API token revoke adopts `variant="danger"`.

## Non-goals

- Form-input migration — deferred to #3j2.
- Password-change modal itself — sub-spec #3k.
- Audit log detail panel refactor — outside Button scope.
- Profile-screen avatar or settings link refactor — outside scope.

## Scope

Files migrated:

- `frontend/src/routes/host-tags/+page.svelte` — add tag, edit, delete,
  color-swatch launchers.
- `frontend/src/routes/audit-logs/+page.svelte` — filter chips, export
  CSV, row-level view-details actions, pagination.
- `frontend/src/routes/profile/+page.svelte` — change password
  launcher, generate token, copy token, revoke token.

## Migration pattern

Standard translation rules. Special:

- Color swatch: `<Button variant="ghost" size="sm" class="p-0 w-5 h-5
  rounded-[3px]" style="background-color: {tag.color}" aria-label="Change
  color for {tag.name}" onclick={openPicker} />`. Uses `ariaLabel` prop
  if Button primitive update has landed; otherwise `sr-only` child as
  per #3b tactical fallback.
- Export: `<Button variant="secondary" size="sm" leadingIcon={DownloadIcon}
  loading={isExporting}>Export CSV</Button>`.
- Revoke token: `<Button variant="danger" size="sm" loading={isRevoking}>
  Revoke</Button>`.

## Data flow

Template-level only. Existing tag CRUD, export, token CRUD handlers
unchanged.

## Error handling

Button discriminated union catches invalid prop combos at compile time.
Toast pipelines unchanged.

## Testing

### Unit tests

Extend `host-tags/+page.test.ts`, `audit-logs/+page.test.ts`,
`profile/+page.test.ts`:

- Color swatch renders as Button with background-color style.
- Export button: secondary + loading.
- Revoke token: danger + loading.
- Change password launcher: secondary variant.
- Generate token: primary variant.
- Filter chip active-state override (audit-logs).

### Integration / e2e

- Playwright re-baseline `/host-tags`, `/audit-logs`, `/profile`. Delta
  enumeration: action buttons shrink to `h-[23px]`; uppercase 9px text;
  swatch dimensions preserved (w-5 h-5).
- Smoke test revoke token flow — loading state during API call.

## Rollout

Single PR titled
"feat(frontend): migrate host-tags + audit-logs + profile to Button primitive (sub-spec #3j)".

1. `host-tags/+page.svelte` — migrate tag CRUD + color swatches.
2. `audit-logs/+page.svelte` — migrate filters + export + pagination.
3. `profile/+page.svelte` — migrate password launcher + token CRUD.
4. Extend unit tests per plan.
5. Re-baseline Playwright snapshots.
6. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset classes across three lower-traffic
routes. Moderate sensitivity — token revoke is the highest-value
regression guard; mitigated by dedicated unit test.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged, sub-spec #3b merged.
- **Blocks:** #3j2 form-input migration.
- **Parallel-safe with:** sub-spec #3c–i, #3k, #4.
