# Fixture Backfill — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`
(§9 Baseline Waiver Schema)

**Sub-spec #5 of the UI design-language rollout.** Final sub-spec —
runs after every #2, #2b, #3a–k, #4 migration has landed. No primitive
work; fixture + Playwright baseline consolidation.

## Overview

After all migrations land, the set of Playwright baseline snapshots
covering the design-language rollout ends up spread across multiple
baseline-waiver PRs (one per sub-spec). `ui-parity.test.ts` and
`ui-parity-responsive.test.ts` house the long-running parity baseline
fixtures; each sub-spec contributed deltas. Sub-spec #5 consolidates the
fixture corpus: re-runs all parity scenarios, collapses the interim
per-sub-spec baselines into a single clean baseline set, prunes any
orphaned snapshots, and refreshes `frontend/src/lib/test-fixtures/ui-parity.ts`
reference data.

## Design decisions

**Q1 — Baseline consolidation strategy: fresh baseline vs accepted-delta-rollup.**

- Options:
  - (chosen) Delete every existing parity snapshot; run Playwright with
    `--update-snapshots` on a clean branch at the end of all migrations;
    commit the regenerated set as the canonical baseline. Matches the
    parent-spec §9 waiver-end state: "final re-baseline after all
    migrations accepted."
  - Preserve interim snapshots as-is. Rejected — the baseline gradually
    accumulates interim state across sub-specs; consolidation produces
    a single authoritative reference.
- Reasoning: authoritative end-of-rollout reference; diff against this
  set going forward is narrower than a fragmented accretion.

**Q2 — `ui-parity.ts` fixture data: refresh or keep.**

- Options:
  - (chosen) Regenerate fixture data from current production values.
    Token names, variant combinations, and sample class strings should
    all match post-migration reality.
  - Leave fixture data as-is. Rejected — fixture data seeded pre-
    migration; drifts against reality.
- Reasoning: stale fixture data becomes a maintenance tax and breeds
  test flakiness on otherwise-unrelated changes.

**Q3 — Orphan-snapshot pruning.**

- Options:
  - (chosen) Run `playwright test --update-snapshots` on a clean
    `tests/e2e/ui-parity.test.ts-snapshots/` directory (delete the
    directory first). This ensures no orphaned `.png` files remain
    from removed scenarios.
  - Audit manually. Rejected — error-prone at the per-file level.
- Reasoning: deleting the directory before re-generation guarantees
  freshness.

**Q4 — Responsive-baseline scope.**

- Options:
  - (chosen) Same treatment as parity baselines: delete and regenerate
    `ui-parity-responsive.test.ts-snapshots/`. Covered viewports
    (mobile, tablet, desktop) remain unchanged — only the snapshots
    regenerate.
  - Skip responsive. Rejected — responsive deltas accumulate same as
    parity.
- Reasoning: consistent treatment across both parity harnesses.

**Q5 — Fixture extraction: surface-preview and form-primitive preview.**

- Options:
  - (chosen) Preserve `/dev/surface-preview` and `/dev/form-primitive-preview`
    fixtures (from #2b + #4); they're exemplar harnesses that should
    persist as ongoing regression coverage. Re-baseline them here.
  - Collapse them into ui-parity. Rejected — they serve different
    audiences (primitive preview vs whole-route parity).
- Reasoning: dev-preview harnesses are narrow-scope primitive tests;
  ui-parity is broad-scope route tests; keep both.

## Goals

1. Delete every existing `ui-parity.test.ts-snapshots/` and
   `ui-parity-responsive.test.ts-snapshots/` `.png` file.
2. Regenerate Playwright snapshots via `--update-snapshots` on a clean
   branch with all migrations merged.
3. Refresh `frontend/src/lib/test-fixtures/ui-parity.ts` reference data
   (class strings, token values, variant names).
4. Re-baseline `/dev/surface-preview` and `/dev/form-primitive-preview`
   snapshots.
5. Ensure no orphan `.png` files remain in either snapshot directory.

## Non-goals

- New feature work, new primitive extraction, new test scenarios —
  nothing beyond consolidation.
- Backend fixture changes — out of frontend scope.
- Introducing screenshot review tooling — baseline-only update.
- Changing snapshot thresholds (`toHaveScreenshot` tolerance) — defaults
  preserved.
- Removing `ui-parity` tests — keep them; just regenerate baselines.

## Scope

Files touched:

- `frontend/tests/e2e/ui-parity.test.ts-snapshots/` — regenerate every
  `.png` via `--update-snapshots`.
- `frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots/` —
  regenerate every `.png`.
- Any `/dev/surface-preview` or `/dev/form-primitive-preview` snapshot
  directories under `frontend/tests/e2e/` — regenerate.
- `frontend/src/lib/test-fixtures/ui-parity.ts` — update reference
  class strings, token names, variant values.
- `frontend/src/lib/test-fixtures/ui-parity.test.ts` — update any
  reference assertions that changed shape.

## Migration pattern

Steps are mechanical:

1. `rm -rf frontend/tests/e2e/ui-parity.test.ts-snapshots/`.
2. `rm -rf frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots/`.
3. `rm -rf frontend/tests/e2e/form-primitive.spec.ts-snapshots/` if
   present.
4. `rm -rf frontend/tests/e2e/surface-preview.spec.ts-snapshots/` if
   present.
5. Run `cd frontend && npx playwright test ui-parity ui-parity-responsive
   form-primitive surface-preview --update-snapshots`.
6. Review the regenerated snapshots by eye — sanity-check a sample of
   5 (1 desktop + 1 responsive + 1 surface + 1 form-primitive + 1
   random) to ensure no rendering anomalies.
7. Refresh `frontend/src/lib/test-fixtures/ui-parity.ts` to match
   current production values — grep call sites in source for class
   strings that have changed; update fixture accordingly.
8. Run the full parity test suite without `--update-snapshots` — expect
   zero diffs.

## Data flow

No runtime changes. All steps are test-harness consolidation.

## Error handling

If regenerated snapshots exhibit visual anomalies:

- Investigate upstream migration that introduced the anomaly (likely a
  sub-spec-level bug, not a #5 concern).
- File a bug on the upstream sub-spec; do not commit anomalous
  snapshots as baseline.
- #5 PR waits until upstream fix lands.

If fixture data refresh surfaces compile errors (assertion expected
`'preset-filled-primary-500'` but code now emits `'bg-[var(--accent)]'`):

- Update fixture assertion to new value; re-run test; verify green.

## Testing

### Unit tests

- `ui-parity.test.ts` (the fixture-data test, not the Playwright test)
  — every reference assertion passes against refreshed fixture data.

### Integration / e2e

- Full Playwright suite passes without `--update-snapshots`.
- `ui-parity.test.ts` + `ui-parity-responsive.test.ts` — zero
  regression against regenerated baselines.
- `/dev/surface-preview` + `/dev/form-primitive-preview` — zero
  regression against regenerated baselines.

## Rollout

Single PR titled
"chore(frontend): re-baseline UI parity snapshots after design-language rollout (sub-spec #5)".

Prereqs: every #2, #2b, #3a–k, #4 PR merged.

1. Delete `ui-parity` + `ui-parity-responsive` +
   `form-primitive` + `surface-preview` snapshot directories.
2. Run Playwright with `--update-snapshots` on all four suites.
3. Eyeball sample snapshots for anomalies.
4. Refresh `ui-parity.ts` fixture data to match current production
   values.
5. Run full Playwright suite without `--update-snapshots`; verify all
   green.
6. Full frontend gate.

### Risk + rollback

Revert of one PR restores previous fragmented baseline set. Low-risk —
PR is baseline-only (no code change); regression surface is future
baseline drift, which re-running regen addresses.

### Dependencies + ordering

- **Blocks on:** every preceding sub-spec (#2, #2b, #3a, #3a2, #3b–k,
  #4) merged. This sub-spec is the final consolidation step.
- **Blocks:** nothing downstream. End of rollout.
- **Parallel-safe with:** nothing — must run last.
