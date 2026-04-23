# Fixture Backfill — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consolidate all Playwright snapshot baselines and verify fixture data after
the full UI design-language rollout.

**Architecture:** No code changes — pure test-harness consolidation. Delete stale
baseline PNGs, regenerate fresh ones via `--update-snapshots`, verify the full suite
passes cleanly. Fixture data (`ui-parity.ts`) is structural (tab IDs, labels, surface
descriptors) with no CSS class strings; verify it still passes its Vitest unit test
before touching Playwright.

**Tech Stack:** Playwright (Chromium), Vitest, SvelteKit dev server (auto-started by
Playwright config)

> **HARD PREREQUISITE: Run on macOS.** The parity suite files (`ui-parity.test.ts`,
> `ui-parity-responsive.test.ts`) contain `process.platform === 'darwin'` guards and
> call `test.skip(!isCanonicalUiParityHost, ...)`. On Linux/Windows every parity test
> is silently skipped and `--update-snapshots` writes nothing, leaving you with empty
> snapshot dirs. CI runs on Linux — do not run snapshot regen there.

---

## File Map

| File | Action |
| --- | --- |
| `frontend/src/lib/test-fixtures/ui-parity.ts` | Verify only (update if assertions changed) |
| `frontend/src/lib/test-fixtures/ui-parity.test.ts` | Verify only (update if fixture shape changed) |
| `frontend/tests/e2e/ui-parity.test.ts-snapshots/` | Delete all 20 PNGs + orphan; regenerate 19 |
| `frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots/` | Delete all 6 PNGs; regenerate 6 |
| `frontend/tests/e2e/surface-preview.spec.ts-snapshots/` | Delete 2 PNGs; regenerate 2 |
| `frontend/tests/e2e/form-primitive.spec.ts-snapshots/` | Delete 12 PNGs; regenerate 12 |
| `frontend/tests/e2e/button-primitive.spec.ts-snapshots/` | Delete 10 PNGs; regenerate 10 |

**Do not touch:**

- `frontend/tests/e2e/public-entry.spec.ts-snapshots/` — sub-spec migration baselines, not parity suite
- `frontend/tests/e2e/software-area.spec.ts-snapshots/` — same

---

## Task 1: Verify Vitest fixture test passes

**Files:**

- Read: `frontend/src/lib/test-fixtures/ui-parity.ts`
- Read: `frontend/src/lib/test-fixtures/ui-parity.test.ts`

- [ ] **Step 1: Run the fixture unit test**

```bash
cd frontend && npm run test -- ui-parity
```

Expected output: all 4 tests pass. The test file is
`src/lib/test-fixtures/ui-parity.test.ts` (Vitest, not Playwright).

```text
✓ ui parity fixtures > builds parity surface tabs with stable defaults
✓ ui parity fixtures > builds deterministic settings and software parity scenarios
✓ ui parity fixtures > keeps surface-page parity fixtures fixed for provider count and availability
✓ ui parity fixtures > builds deterministic shared visual parity fixtures ...
```

- [ ] **Step 2: If any test fails, update the fixture**

If a test fails with a diff, the fixture data has drifted from production. Check:

- `frontend/src/routes/settings/+page.svelte` lines ~95–111 for `SETTINGS_BUILT_IN_TABS`
- `frontend/src/routes/software/+page.svelte` for `SOFTWARE_BUILT_IN_TABS`

Update `frontend/src/lib/test-fixtures/ui-parity.ts` to match, then re-run the test.
Repeat until green. If all 4 tests pass immediately, skip this step.

- [ ] **Step 3: Commit if fixture was changed (skip if no changes)**

```bash
git add frontend/src/lib/test-fixtures/ui-parity.ts \
        frontend/src/lib/test-fixtures/ui-parity.test.ts
git commit -m "chore(frontend): refresh ui-parity fixture data to post-migration values (#5)"
```

---

## Task 2: Delete orphan snapshot and parity-suite snapshot directories

**Files:**

- Delete: `frontend/tests/e2e/ui-parity.test.ts-snapshots/ui-parity-surface-page-runtime-state-shell-chromium.png`
- Delete dir: `frontend/tests/e2e/ui-parity.test.ts-snapshots/`
- Delete dir: `frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots/`

**Why the orphan matters:** `ui-parity.test.ts` has `test.skip(true, ...)` on
`'surface page ui parity: surface.page runtime-state shell'`. `--update-snapshots`
only regenerates snapshots for tests that actually run — the skipped test produces no
output and the PNG persists as a zombie file. It must be deleted before regeneration.

- [ ] **Step 1: Manually delete the orphan snapshot**

```bash
rm frontend/tests/e2e/ui-parity.test.ts-snapshots/ui-parity-surface-page-runtime-state-shell-chromium.png
```

Verify:

```bash
ls frontend/tests/e2e/ui-parity.test.ts-snapshots/ | grep runtime-state
```

Expected: no output (file deleted).

- [ ] **Step 2: Delete the parity-suite snapshot directories**

```bash
rm -rf frontend/tests/e2e/ui-parity.test.ts-snapshots/
rm -rf frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots/
```

- [ ] **Step 3: Verify the directories are gone**

```bash
ls frontend/tests/e2e/ | grep -E "ui-parity.*snapshots"
```

Expected: no output.

---

## Task 3: Delete dev-preview snapshot directories

**Files:**

- Delete dir: `frontend/tests/e2e/surface-preview.spec.ts-snapshots/`
- Delete dir: `frontend/tests/e2e/form-primitive.spec.ts-snapshots/`
- Delete dir: `frontend/tests/e2e/button-primitive.spec.ts-snapshots/`

- [ ] **Step 1: Delete the three dev-preview snapshot directories**

```bash
rm -rf frontend/tests/e2e/surface-preview.spec.ts-snapshots/
rm -rf frontend/tests/e2e/form-primitive.spec.ts-snapshots/
rm -rf frontend/tests/e2e/button-primitive.spec.ts-snapshots/
```

- [ ] **Step 2: Verify gone and out-of-scope dirs untouched**

```bash
ls frontend/tests/e2e/ | grep snapshots
```

Expected output (only these two must remain — do NOT delete them):

```text
public-entry.spec.ts-snapshots
software-area.spec.ts-snapshots
```

---

## Task 4: Regenerate all snapshots on macOS + Chromium

**Prerequisite:** Confirm macOS before proceeding.

```bash
uname -s
```

Expected: `Darwin`. If `Linux`, stop — snapshots regenerated on Linux will be rejected
by every future macOS run.

- [ ] **Step 1: Install Chromium if not already installed**

```bash
cd frontend && npx playwright install chromium
```

Safe to re-run; no-ops if already installed.

- [ ] **Step 2: Regenerate all snapshots**

```bash
cd frontend && npx playwright test \
  ui-parity ui-parity-responsive surface-preview form-primitive button-primitive \
  --update-snapshots
```

Playwright auto-starts the SvelteKit dev server (`npm run dev`) on
`http://localhost:5173` via the `webServer` config — no manual server startup needed.

Expected outcome:

- `ui-parity.test.ts-snapshots/` created with 19 PNGs
- `ui-parity-responsive.test.ts-snapshots/` created with 6 PNGs
- `surface-preview.spec.ts-snapshots/` created with 2 PNGs
- `form-primitive.spec.ts-snapshots/` created with 12 PNGs
- `button-primitive.spec.ts-snapshots/` created with 10 PNGs

Total: 49 new baseline PNGs.

If the run fails, open the HTML report:

```bash
open frontend/playwright-report/index.html
```

Common causes: port 5173 already in use (kill the process), fixture data mismatch
(re-run Task 1).

- [ ] **Step 3: Verify the snapshot counts**

```bash
ls frontend/tests/e2e/ui-parity.test.ts-snapshots/ | wc -l
ls frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots/ | wc -l
ls frontend/tests/e2e/surface-preview.spec.ts-snapshots/ | wc -l
ls frontend/tests/e2e/form-primitive.spec.ts-snapshots/ | wc -l
ls frontend/tests/e2e/button-primitive.spec.ts-snapshots/ | wc -l
```

Expected: `19`, `6`, `2`, `12`, `10`.

If any count is 0, the platform guard fired (wrong OS) or the dev server failed to
start. Do not proceed — diagnose before continuing.

---

## Task 5: Eyeball sample snapshots

Open 6 specific PNGs and visually inspect. Check: correct colour scheme (dark/light),
no blank/white frames, no layout explosions, no raw `preset-filled-*` button classes.

- [ ] **Step 1: Desktop parity snapshot**

```bash
open frontend/tests/e2e/ui-parity.test.ts-snapshots/ui-parity-settings-tabs-chromium.png
```

Expect: settings tab strip in post-migration style, terminal-dark/light theme.

- [ ] **Step 2: Responsive parity snapshot**

```bash
open frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots/ui-parity-responsive-mobile-bottom-nav-overflow-chromium.png
```

Expect: mobile viewport, bottom nav rendered, no layout anomalies.

- [ ] **Step 3: Surface-preview snapshot**

```bash
open frontend/tests/e2e/surface-preview.spec.ts-snapshots/dark-full-chromium.png
```

Expect: `/dev/surface-preview` route, dark theme, interactive surface components.

- [ ] **Step 4: Form-primitive snapshot**

```bash
open frontend/tests/e2e/form-primitive.spec.ts-snapshots/dark-input-states-chromium.png
```

Expect: input components in dark theme with normal/focus/error/disabled states.

- [ ] **Step 5: Button-primitive snapshot**

```bash
open frontend/tests/e2e/button-primitive.spec.ts-snapshots/dark-variants-chromium.png
```

Expect: Button variants (primary/secondary/ghost/danger) in dark theme, accent gradient
on primary.

- [ ] **Step 6: Random parity sanity check**

```bash
open frontend/tests/e2e/ui-parity.test.ts-snapshots/ui-parity-context-menu-shell-chromium.png
```

Expect: context menu shell, correct styling.

If any snapshot looks wrong, do NOT commit. File a bug on the upstream migration that
caused the anomaly and wait for a fix.

---

## Task 6: Run full Playwright suite without --update-snapshots

- [ ] **Step 1: Run the verification pass**

```bash
cd frontend && npx playwright test \
  ui-parity ui-parity-responsive surface-preview form-primitive button-primitive
```

Expected: all tests pass, zero pixel diffs. Any diff here means a snapshot was
regenerated against stale state — investigate before committing.

- [ ] **Step 2: Check HTML report if any test fails**

```bash
open frontend/playwright-report/index.html
```

If the diff shows a real rendering regression (migration bug), do not commit — file a
bug on the upstream sub-spec. If the diff is a false alarm (e.g. anti-aliasing
variance within the 0.5% threshold), verify the pixel ratio is within
`PARITY_MAX_DIFF_PIXEL_RATIO = 0.005` (from `frontend/tests/e2e/parity-config.ts`).

---

## Task 7: Run Vitest fixture test one final time

- [ ] **Step 1: Regression guard**

```bash
cd frontend && npm run test -- ui-parity
```

Expected: same 4 green tests as Task 1. Confirms fixture data refresh (if any) is still
aligned with the now-regenerated Playwright assertions.

---

## Task 8: Commit the regenerated baselines

- [ ] **Step 1: Stage all snapshot directories**

```bash
git add \
  frontend/tests/e2e/ui-parity.test.ts-snapshots/ \
  frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots/ \
  frontend/tests/e2e/surface-preview.spec.ts-snapshots/ \
  frontend/tests/e2e/form-primitive.spec.ts-snapshots/ \
  frontend/tests/e2e/button-primitive.spec.ts-snapshots/
```

- [ ] **Step 2: Verify only snapshot files are staged**

```bash
git diff --staged --name-only | grep -v "spec.ts-snapshots\|test.ts-snapshots"
```

Expected: no output (only snapshot files staged, no accidental source changes).

- [ ] **Step 3: Verify the orphan is removed from tracking**

```bash
git diff --staged --name-only | grep runtime-state
```

Expected: one line showing the orphan as deleted:

```text
frontend/tests/e2e/ui-parity.test.ts-snapshots/ui-parity-surface-page-runtime-state-shell-chromium.png
```

- [ ] **Step 4: Commit**

```bash
git commit -m "$(cat <<'EOF'
chore(frontend): re-baseline UI parity snapshots after design-language rollout (#5)

Delete and regenerate all parity + dev-preview Playwright baselines.
Remove orphaned runtime-state-shell snapshot (test.skip'd scenario).
Consolidates per-sub-spec interim baselines into a single clean set.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 5: Run full frontend gate as final verification**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: all steps pass.
