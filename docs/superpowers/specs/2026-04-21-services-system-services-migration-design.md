# Services + System Services Routes Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`
(§4.3 Buttons, §4.5 Cards, §4.7 Tables)

**Sub-spec #3i of the UI design-language rollout.** Depends on sub-spec #2
(Button primitive) merged. Form-input sites defer to a future #3i2 pass
after sub-spec #2b primitives land.

## Overview

Migrate two sibling route pages: `/services/+page.svelte` (675 lines —
Service admin: register services, approve/reject, assign to hosts)
and `/system-services/+page.svelte` (642 — System service admin: built-in
update triggers, diagnostic services). Both share structural patterns
(list + filters + row actions + bulk triggers) but hit different API
surfaces.

## Design decisions

**Q1 — ServiceStatus row-level action variants.**

- Options:
  - (chosen) Approve: `primary`; Reject: `danger`; Deactivate: `secondary`
    (reversible); Reactivate (currently deactivated): `primary`. Matches
    the ServiceStatus variants `Pending | Approved | Rejected | Deactivated`
    from shared-types.
  - Use `secondary` for every state change. Rejected — loses the visual
    distinction between approve and deactivate (different blast
    radius).
- Reasoning: variant choice maps to reversibility and intent per parent
  §4.3; approve is go-forward primary, deactivate is reversible hence
  secondary, reject is destructive hence danger.

**Q2 — Host-assignment modal launch.**

- Options:
  - (chosen) Migrate the launcher button only; modal itself lives in
    `AssignToHostModal` / `EditHostAssignmentModal` components — those
    defer to #3k.
  - Migrate modal in-place. Rejected — shared modal component is #3k
    territory.
- Reasoning: scope discipline matching #3f / #3h precedent.

**Q3 — System-service "Run now" trigger.**

- Options:
  - (chosen) `<Button variant="primary" size="sm" loading={isRunning}
    leadingIcon={PlayIcon}>Run now</Button>`. Signals a primary workflow
    action (ad-hoc invocation).
  - `<Button variant="secondary">`. Rejected — Run-now is the main
    affordance on system services; it deserves primary weight.
- Reasoning: primary variant matches main-affordance intent on a row.

**Q4 — Filter/status pills on both lists.**

- Options:
  - (chosen) Same ghost+active-override pattern as history (#3g),
    navbar (#3b), settings tabs (#3c). Fourth consumer of the pattern.
  - Extract `<FilterChip>` primitive now. Deferred — still mechanical
    override; revisit the extraction threshold after a fifth consumer
    emerges.
- Reasoning: four consumers is close to the extraction threshold but
  still mechanical; keep YAGNI discipline here.

## Goals

1. Every interactive button on both files renders through `<Button>`.
2. State-transition actions use variants aligned to the ServiceStatus
   semantics (approve=primary, reject=danger, deactivate=secondary).
3. "Run now" on system services adopts primary + icon + loading shape.
4. Filter/status pills adopt the ghost+active-override shape.

## Non-goals

- Form-input migration — deferred to #3i2.
- Assign-to-host modal migration — sub-spec #3k.
- Service backend logic — out of scope.
- New system service types — feature work.

## Scope

Files migrated:

- `frontend/src/routes/services/+page.svelte` — register, approve,
  reject, deactivate, reactivate, assign-to-host launcher, filters,
  bulk launchers.
- `frontend/src/routes/system-services/+page.svelte` — run now, edit,
  delete, filters, bulk launchers.

## Migration pattern

Standard translation rules. Special:

- Approve: `<Button variant="primary" size="sm" loading={isApproving}>
  Approve</Button>`.
- Reject: `<Button variant="danger" size="sm" loading={isRejecting}>
  Reject</Button>`.
- Deactivate: `<Button variant="secondary" size="sm" loading={isDeactivating}>
  Deactivate</Button>`.
- Run now: `<Button variant="primary" size="sm" leadingIcon={PlayIcon}
  loading={isRunning}>Run now</Button>`.
- Filter pills: `<Button variant="ghost" size="sm" class={active ?
  'bg-[var(--bg-hover)] text-[var(--accent)]' : ''}>`.

## Data flow

Template-level only. Existing approve/reject/deactivate/run-now API
calls unchanged.

## Error handling

Button discriminated union catches invalid prop combos at compile time.
Toast pipelines unchanged.

## Testing

### Unit tests

Extend `services/+page.test.ts` and `system-services/+page.test.ts`:

- Approve button: primary + sm + loading.
- Reject button: danger + sm + loading.
- Deactivate button: secondary + sm + loading.
- Run now: primary + sm + icon + loading.
- Filter pill active-state override.
- Bulk launcher buttons render with correct variant.

### Integration / e2e

- Playwright re-baseline `/services` (default + pending queue visible)
  and `/system-services` (default + filter active + in-progress run).
  Delta enumeration: action buttons shrink to `h-[23px]`; uppercase 9px
  text; primary/danger/secondary variant colors visible.
- Smoke test approve/reject/deactivate flow — loading states render.

## Rollout

Single PR titled
"feat(frontend): migrate services + system services to Button primitive (sub-spec #3i)".

1. `services/+page.svelte` — migrate state-transition + filter + bulk
   launcher buttons.
2. `system-services/+page.svelte` — migrate Run now + edit + delete +
   filter buttons.
3. Extend unit tests per plan.
4. Re-baseline Playwright snapshots.
5. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset classes across services + system-
services. High sensitivity — state-transition actions gate service
lifecycle; mitigated by per-action unit tests.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged, sub-spec #3b merged.
- **Blocks:** #3i2 form-input migration.
- **Parallel-safe with:** sub-spec #3c–h, #3j–k, #4.
