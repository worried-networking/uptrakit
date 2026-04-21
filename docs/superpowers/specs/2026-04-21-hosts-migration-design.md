# Hosts Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`
(§4.3 Buttons, §4.5 Cards, §4.7 Tables, §6 Terminal)

**Sub-spec #3h of the UI design-language rollout.** Depends on sub-spec #2
(Button primitive + terminal theme) merged. Form-input sites defer to
a future #3h2 pass after sub-spec #2b primitives land.

## Overview

Migrate host administration: `/hosts/+page.svelte` (590 lines — host list
with bulk actions, approve/reject queued rows, filters) and
`/hosts/[id]/+page.svelte` (796 — host detail with software list, trigger-
update, SSH terminal launch, tags, audit trail). Host detail contains
the canonical SSH terminal attachment point.

## Design decisions

**Q1 — "Approve / Reject" pending-enrollment buttons on host list.**

- Options:
  - (chosen) `<Button variant="primary" size="sm">Approve</Button>`,
    `<Button variant="danger" size="sm">Reject</Button>`. Matches
    ServiceStatus semantic distinction.
  - Use secondary for both. Rejected — approve vs reject is a meaningful
    semantic split; primary/danger conveys it via color.
- Reasoning: parent §4.3 treats approve as the success path (primary
  gradient) and reject as destructive (red gradient); this is exactly
  the shape the variants encode.

**Q2 — "Launch SSH" button on host detail.**

- Options:
  - (chosen) `<Button variant="primary" size="sm" leadingIcon={TerminalIcon}>Launch
    SSH</Button>`. Opens the in-page terminal panel (sub-spec #2 terminal
    theme already applies).
  - Use ghost variant. Rejected — launching SSH is a primary workflow
    action on host detail, not a side affordance.
- Reasoning: SSH terminal is the primary management entry point on
  host detail; primary variant matches that intent.

**Q3 — Host-tag chips.**

- Options:
  - (chosen) Keep tag chips as non-button `<span>` elements. They are
    not interactive on the detail page (display only) — migration not
    applicable.
  - Migrate tag chips if they're clickable. Deferred — if future
    feature work makes them clickable, migration happens then.
- Reasoning: sub-spec is migration-only; non-interactive elements stay.

**Q4 — Trigger-update site on host detail.**

- Options:
  - (chosen) Use `<UpdateAllButton>` primitive from sub-spec #2 (matches
    #3f decision).
  - Raw `<Button variant="primary">`. Rejected — UpdateAllButton owns
    the status polling contract.
- Reasoning: single canonical trigger-update primitive across the app.

**Q5 — Host-list bulk action trigger.**

- Options:
  - (chosen) Defer bulk-action bar itself to #3k. This sub-spec migrates
    the triggers that *open* it (e.g. "Bulk trigger update" button),
    not the bar.
  - Migrate bar inline. Rejected — same rationale as #3f.
- Reasoning: shared bulk-action-bar primitive is #3k territory.

## Goals

1. Every interactive button on both host files renders through
   `<Button>` or `<UpdateAllButton>`.
2. Approve/reject pending-enrollment rows adopt primary/danger variants
   respectively.
3. "Launch SSH" adopts primary + icon shape.
4. Every trigger-update call site uses `<UpdateAllButton>`.

## Non-goals

- Form-input migration (tag editor, rename field) — deferred to #3h2.
- SSH terminal UI refactor — terminal theming already done in #2.
- BatchActionBar component migration — sub-spec #3k.
- Audit trail refactor — outside Button scope.
- Host enrollment backend — out of scope.

## Scope

Files migrated:

- `frontend/src/routes/hosts/+page.svelte` — filters, approve/reject
  queued rows, row-level actions, bulk action launchers.
- `frontend/src/routes/hosts/[id]/+page.svelte` — launch SSH, trigger
  update (via UpdateAllButton), tag-editor launch button, audit filters,
  software row actions.

## Migration pattern

Standard translation rules. Special:

- Approve: `<Button variant="primary" size="sm" loading={isApproving}
  onclick={approve(id)}>Approve</Button>`.
- Reject: `<Button variant="danger" size="sm" loading={isRejecting}
  onclick={reject(id)}>Reject</Button>`.
- Launch SSH: `<Button variant="primary" size="sm" leadingIcon={TerminalIcon}
  onclick={openTerminal}>Launch SSH</Button>`.
- Trigger-update sites → `<UpdateAllButton hostIds={[hostId]} />`.

## Data flow

Template-level only. Approve/reject hit existing API endpoints; SSH
launcher opens existing terminal panel via existing store. No new
runtime behavior.

## Error handling

Button discriminated union catches invalid prop combos at compile time.
Approve/reject errors surface to toast; Button only renders loading.

## Testing

### Unit tests

Extend `hosts/+page.test.ts` / `[id]/+page.test.ts`:

- Approve button renders primary + sm + loading during call.
- Reject button renders danger + sm + loading during call.
- Launch SSH button renders primary + icon.
- Trigger-update sites render UpdateAllButton, not raw Button.
- Row-level host actions render ghost + sm + icon (view, copy hostname,
  etc.).

### Integration / e2e

- Playwright re-baseline `/hosts` (default + pending-enrollment queue
  visible) and `/hosts/[id]` (default + terminal-open + trigger-update
  in-progress). Delta enumeration: approve/reject buttons shrink to
  `h-[23px]`; uppercase 9px text; terminal theme already baselined in #2.
- Smoke test approve/reject flow — button loading state during API call.

## Rollout

Single PR titled
"feat(frontend): migrate hosts area to Button primitive (sub-spec #3h)".

1. `hosts/+page.svelte` — migrate filters, approve/reject, row actions,
   bulk launchers.
2. `hosts/[id]/+page.svelte` — migrate Launch SSH, trigger-update
   (swap to UpdateAllButton), tag editor launcher, audit filters.
3. Extend unit tests per plan.
4. Re-baseline Playwright snapshots.
5. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset classes across hosts. High sensitivity —
approve/reject is the gate for new host enrollment; SSH launch is the
primary management workflow. Mitigated by dedicated unit tests on both
flows plus Playwright coverage.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged, sub-spec #2c merged
  (`variant="secondary"` for Deactivate + `--bg-hover` for filter chips),
  sub-spec #3b merged, sub-spec #3f merged (UpdateAllButton reuse
  precedent).
- **Blocks:** #3h2 form-input migration.
- **Parallel-safe with:** sub-spec #3c–e, #3g, #3i–k, #4.
