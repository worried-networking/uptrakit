# Primitive Conformance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Correct border-radius, typography, and TabStrip active-state deviations in the `ui/`
primitive and surface component layer to match the design spec exactly.

**Architecture:** Pure class substitution across 9 Svelte files and 1 test file. No logic changes,
no new props, no new components. TabStrip active-state change requires Playwright baseline
regeneration; all other changes are invisible to existing Vitest tests.

**Tech Stack:** SvelteKit, Tailwind CSS, Vitest, Playwright (macOS + Chromium baseline regen)

---

## Files modified

| File | Change |
| --- | --- |
| `frontend/src/lib/components/ui/Callout.svelte:28` | `rounded-xl` → `rounded-[4px]` |
| `frontend/src/lib/components/ui/EmptyState.svelte:16` | `rounded-2xl` → `rounded-[3px]` |
| `frontend/src/lib/components/ui/SectionCard.svelte:18` | `rounded-2xl` → `rounded-[3px]` |
| `frontend/src/lib/components/ui/ProviderSelector.svelte:56` | `rounded-xl` → `rounded-[3px]` |
| `frontend/src/lib/components/ui/DataTable.svelte:76,85` | `px-4` → `px-[10px]` (two header cell locations) |
| `frontend/src/lib/components/ui/TabStrip.svelte:101,107-112` | outer `rounded-xl` → `rounded-[4px]`; button `rounded-lg` → `rounded-[3px]`; active state tint |
| `frontend/src/lib/components/ui/PageShell.svelte:25` | `text-3xl font-semibold tracking-tight` → `text-[20px] font-bold` |
| `frontend/src/lib/components/Modal.svelte:30` | `h3` class → explicit token class |
| `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte:89` | `h3` class → explicit token class |
| `frontend/src/lib/components/surfaces/SurfaceSlot.svelte:38-39` | `card` utility + `h3` class → token classes; add `data-ui` attr |
| `frontend/src/lib/components/surfaces/SurfaceSlot.test.ts:45` | update `section.card` selector to `data-ui` attr |
| `frontend/src/lib/components/SoftwareMergeWizard.svelte:237,356,367,382,401` | `h4` class → explicit token class |

---

## Task 1: Fix border-radius on Callout, EmptyState, SectionCard

**Files:**

- Modify: `frontend/src/lib/components/ui/Callout.svelte:28`
- Modify: `frontend/src/lib/components/ui/EmptyState.svelte:16`
- Modify: `frontend/src/lib/components/ui/SectionCard.svelte:17`

Spec authority: §2.3 — modals/panels/drawers = `rounded-[4px]`; cards/table wrappers/buttons = `rounded-[3px]`.
`Callout` is a panel (inline alert) → `rounded-[4px]`. `EmptyState` and `SectionCard` are cards → `rounded-[3px]`.

None of the existing tests assert on the `rounded-*` class values so no test changes are needed here. Verify this first.

- [ ] **Step 1: Verify no test asserts on the old rounded values**

```bash
cd frontend && grep -n 'rounded-xl\|rounded-2xl' src/lib/components/ui/Callout.test.ts src/lib/components/ui/EmptyState.test.ts src/lib/components/ui/SectionCard.test.ts 2>/dev/null
```

Expected: no output (none of those tests assert on radius classes).

- [ ] **Step 2: Fix Callout.svelte — `rounded-xl` → `rounded-[4px]`**

In `frontend/src/lib/components/ui/Callout.svelte` line 28, change:

```svelte
<aside class={`rounded-xl border px-4 py-3 text-sm ${toneClasses[tone]}`} data-ui="callout" data-tone={tone} {role}>
```

to:

```svelte
<aside class={`rounded-[4px] border px-4 py-3 text-sm ${toneClasses[tone]}`} data-ui="callout" data-tone={tone} {role}>
```

- [ ] **Step 3: Fix EmptyState.svelte — `rounded-2xl` → `rounded-[3px]`**

In `frontend/src/lib/components/ui/EmptyState.svelte` line 16, change:

```svelte
	class="rounded-2xl border border-dashed border-[var(--border-default)] bg-[var(--bg-surface)] px-6 py-8 text-center"
```

to:

```svelte
	class="rounded-[3px] border border-dashed border-[var(--border-default)] bg-[var(--bg-surface)] px-6 py-8 text-center"
```

- [ ] **Step 4: Fix SectionCard.svelte — `rounded-2xl` → `rounded-[3px]`**

In `frontend/src/lib/components/ui/SectionCard.svelte` line 18, change:

```svelte
	class="rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm"
```

to:

```svelte
	class="rounded-[3px] border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm"
```

- [ ] **Step 5: Run Vitest to confirm no regressions**

```bash
cd frontend && npm run test -- --run --reporter=verbose 2>&1 | tail -20
```

Expected: all tests pass, 0 failures.

- [ ] **Step 6: Run type check**

```bash
cd frontend && npm run check 2>&1 | tail -10
```

Expected: zero errors.

- [ ] **Step 7: Commit**

```bash
cd frontend && git add src/lib/components/ui/Callout.svelte src/lib/components/ui/EmptyState.svelte src/lib/components/ui/SectionCard.svelte
git commit -m "fix(frontend): correct border-radius on Callout, EmptyState, SectionCard to spec §2.3"
```

---

## Task 2: Fix ProviderSelector radius and DataTable header padding

**Files:**

- Modify: `frontend/src/lib/components/ui/ProviderSelector.svelte:56`
- Modify: `frontend/src/lib/components/ui/DataTable.svelte:76,85`

Spec: §2.3 form fields = `rounded-[3px]`. §4.12 DataTable header cell padding = `px-[10px]`.

- [ ] **Step 1: Verify no test asserts on the old values**

```bash
cd frontend && grep -n 'rounded-xl\|px-4' src/lib/components/ui/ProviderSelector.test.ts src/lib/components/ui/DataTable.test.ts 2>/dev/null
```

Expected: no output from ProviderSelector test. DataTable test may have `px-4` references in
tested content — examine any matches. These tests generally use semantic selectors (role, text),
not className assertions; if `px-4` appears only in cell data strings it won't matter.

- [ ] **Step 2: Fix ProviderSelector.svelte — `rounded-xl` → `rounded-[3px]` on the select element**

In `frontend/src/lib/components/ui/ProviderSelector.svelte` line 56, change:

```svelte
		class="select w-full rounded-xl border-[var(--border-default)] bg-[var(--bg-surface)] text-[var(--text-primary)]"
```

to:

```svelte
		class="select w-full rounded-[3px] border-[var(--border-default)] bg-[var(--bg-surface)] text-[var(--text-primary)]"
```

- [ ] **Step 3: Fix DataTable.svelte — `px-4` → `px-[10px]` on header cells**

There are two header `<th>` elements. Both use `px-4` and both need updating to `px-[10px]`.

**Line 76** — dynamic column headers (inside `#each`):

```svelte
					class={`px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.12em] ${
```

→

```svelte
					class={`px-[10px] py-3 text-[11px] font-semibold uppercase tracking-[0.12em] ${
```

**Line 85** — row-actions column header (inside `#if rowActions`):

```svelte
								<th class="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-[0.12em]" scope="col">
```

→

```svelte
								<th class="px-[10px] py-3 text-left text-[11px] font-semibold uppercase tracking-[0.12em]" scope="col">
```

Confirm both replaced:

```bash
grep -n 'px-4' frontend/src/lib/components/ui/DataTable.svelte
```

Expected: only data-row `<td>` lines (100, 108) remain with `px-4` — the two `<th>` lines should now show `px-[10px]`.

- [ ] **Step 4: Run Vitest**

```bash
cd frontend && npm run test -- --run --reporter=verbose 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 5: Run type check**

```bash
cd frontend && npm run check 2>&1 | tail -10
```

Expected: zero errors.

- [ ] **Step 6: Commit**

```bash
cd frontend && git add src/lib/components/ui/ProviderSelector.svelte src/lib/components/ui/DataTable.svelte
git commit -m "fix(frontend): ProviderSelector rounded-[3px], DataTable header px-[10px] per spec §2.3/§4.12"
```

---

## Task 3: Fix TabStrip geometry and active-state

**Files:**

- Modify: `frontend/src/lib/components/ui/TabStrip.svelte:101,107-112`
- Playwright baseline regeneration required

Three changes in one file:

1. Outer container `rounded-xl` → `rounded-[4px]` (panel radius per §2.3)
2. Tab button `rounded-lg` → `rounded-[3px]` (button radius per §2.3)
3. Active state: `bg-[var(--accent)] text-[var(--text-inverted)] shadow-sm` →
   `bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]` (tint per §4.11)

The existing Vitest tests do NOT assert on active state classes or radius — they use `data-state`
attribute, keyboard navigation, and aria attributes. Verify this, then make the changes.

- [ ] **Step 1: Verify Vitest tests don't assert on active class or rounded values**

```bash
cd frontend && grep -n 'bg-\[var(--accent)\]\|rounded-xl\|rounded-lg\|text-\[var(--text-inverted)\]' src/lib/components/ui/TabStrip.test.ts
```

Expected: no output.

- [ ] **Step 2: Fix outer container radius**

In `frontend/src/lib/components/ui/TabStrip.svelte` line 101, change:

```svelte
	class="flex flex-wrap gap-2 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-1"
```

to:

```svelte
	class="flex flex-wrap gap-2 rounded-[4px] border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-1"
```

- [ ] **Step 3: Fix tab button radius and active state**

In `frontend/src/lib/components/ui/TabStrip.svelte`, the button element starts at line 106.
Find the class attribute that contains `rounded-lg` and the conditional active/inactive classes.
Change:

```svelte
			class="rounded-lg px-3 py-2 text-sm font-medium transition-[background,border-color,color] duration-[120ms] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-40 {isActive(
			item.id
		)
			? 'bg-[var(--accent)] text-[var(--text-inverted)] shadow-sm'
			: 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'}"
```

to:

```svelte
			class="rounded-[3px] px-3 py-2 text-sm font-medium transition-[background,border-color,color] duration-[120ms] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-40 {isActive(
			item.id
		)
			? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
			: 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'}"
```

- [ ] **Step 4: Run Vitest**

```bash
cd frontend && npm run test -- --run --reporter=verbose 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 4a: Run type check**

```bash
cd frontend && npm run check 2>&1 | tail -10
```

Expected: zero errors.

- [ ] **Step 5: Delete stale Playwright baselines**

The active-state change will visually differ from the old baselines. Delete them so regeneration creates fresh ones.

```bash
rm -rf frontend/tests/e2e/ui-parity.test.ts-snapshots/
rm -rf frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots/
```

- [ ] **Step 6: Regenerate Playwright baselines**

**IMPORTANT:** This must run on macOS with Chromium. The parity suite has a `process.platform === 'darwin'` guard and will silently skip on Linux.

```bash
cd frontend && npx playwright test ui-parity ui-parity-responsive --update-snapshots
```

Expected: all parity tests pass (they regenerate, not compare). New PNGs written to `tests/e2e/ui-parity.test.ts-snapshots/` and `tests/e2e/ui-parity-responsive.test.ts-snapshots/`.

- [ ] **Step 7: Visually verify the new baselines**

Open a few regenerated PNGs (e.g. `ui-parity-software-page-chromium.png`) and confirm the
active tab shows a tinted background with accent-colored text, not a solid filled tab.

- [ ] **Step 8: Commit**

```bash
cd frontend && git add src/lib/components/ui/TabStrip.svelte tests/e2e/ui-parity.test.ts-snapshots/ tests/e2e/ui-parity-responsive.test.ts-snapshots/
git commit -m "fix(frontend): TabStrip radius + tint active state per spec §2.3/§4.11; regen parity baselines"
```

---

## Task 4: Fix PageShell h1 typography

**Files:**

- Modify: `frontend/src/lib/components/ui/PageShell.svelte:25`

Spec §2.4: h1 = `text-[20px] font-bold text-[var(--text-primary)]`. Current value:
`text-3xl font-semibold tracking-tight text-[var(--text-primary)]`. The
`text-[var(--text-primary)]` already matches — only size, weight, and tracking need updating.

The existing PageShell test uses `getByRole('heading', { name: ... })` — it does not assert on class values. No test changes needed.

- [ ] **Step 1: Verify test doesn't assert on h1 class**

```bash
cd frontend && grep -n 'text-3xl\|font-semibold\|tracking-tight' src/lib/components/ui/PageShell.test.ts
```

Expected: no output.

- [ ] **Step 2: Fix PageShell.svelte h1 typography**

In `frontend/src/lib/components/ui/PageShell.svelte` line 25, change:

```svelte
			<h1 class="text-3xl font-semibold tracking-tight text-[var(--text-primary)]">{title}</h1>
```

to:

```svelte
			<h1 class="text-[20px] font-bold text-[var(--text-primary)]">{title}</h1>
```

- [ ] **Step 3: Run Vitest**

```bash
cd frontend && npm run test -- --run --reporter=verbose 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 4: Run type check**

```bash
cd frontend && npm run check 2>&1 | tail -10
```

Expected: zero errors.

- [ ] **Step 5: Commit**

```bash
cd frontend && git add src/lib/components/ui/PageShell.svelte
git commit -m "fix(frontend): PageShell h1 to text-[20px] font-bold per spec §2.4"
```

---

## Task 5: Fix Skeleton `h3` class in Modal and SurfaceRenderer

**Files:**

- Modify: `frontend/src/lib/components/Modal.svelte:30`
- Modify: `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte:89`

Spec §2.4: h3 = `text-[13px] font-bold text-[var(--text-primary)]`. The Skeleton `h3` class is a theme-coupled utility.

- [ ] **Step 1: Verify tests don't assert on the `h3` Skeleton class**

```bash
cd frontend && grep -n ' h3\b' src/lib/components/Modal.test.ts src/lib/components/surfaces/SurfaceRenderer.test.ts
```

Expected: no className assertions. Tests use `getByRole('heading')` or `data-ui` selectors.

- [ ] **Step 2: Fix Modal.svelte**

In `frontend/src/lib/components/Modal.svelte` line 30, change:

```svelte
				<h3 class="h3" id="modal-title">{title}</h3>
```

to:

```svelte
				<h3 class="text-[13px] font-bold text-[var(--text-primary)]" id="modal-title">{title}</h3>
```

- [ ] **Step 3: Fix SurfaceRenderer.svelte**

In `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte` line 89, change:

```svelte
			<h3 class="h3">{node.title}</h3>
```

to:

```svelte
			<h3 class="text-[13px] font-bold text-[var(--text-primary)]">{node.title}</h3>
```

- [ ] **Step 4: Run Vitest**

```bash
cd frontend && npm run test -- --run --reporter=verbose 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 5: Run type check**

```bash
cd frontend && npm run check 2>&1 | tail -10
```

Expected: zero errors.

- [ ] **Step 6: Commit**

```bash
cd frontend && git add src/lib/components/Modal.svelte src/lib/components/surfaces/SurfaceRenderer.svelte
git commit -m "fix(frontend): replace Skeleton h3 class with explicit token typography in Modal and SurfaceRenderer"
```

---

## Task 6: Fix SurfaceSlot — replace `card` utility and `h3` class + update test

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceSlot.svelte:38-39`
- Modify: `frontend/src/lib/components/surfaces/SurfaceSlot.test.ts:45`

Two changes to SurfaceSlot.svelte:

1. `<section class="card space-y-4 p-4">` → token classes + add `data-ui="surface-slot-item"` attribute
2. `<h2 class="h3">` → explicit token class (keep `h2` tag — it's the correct semantic heading level)

The existing test at line 44 uses `querySelectorAll('section.card')` to verify no surface items
render when surfaces=[]. After our change, `.card` is removed from the section. The test would
vacuously pass (0 matches = expected 0), but we update it to be meaningful using the new
`data-ui` attribute.

- [ ] **Step 1: Update the test to use the new data-ui attribute**

In `frontend/src/lib/components/surfaces/SurfaceSlot.test.ts` line 45, change:

```ts
		expect(container.querySelectorAll('section.card')).toHaveLength(0);
```

to:

```ts
		expect(container.querySelectorAll('[data-ui="surface-slot-item"]')).toHaveLength(0);
```

- [ ] **Step 2: Run the test to confirm it fails (test now checks for data-ui attr that doesn't exist yet)**

```bash
cd frontend && npm run test -- --run src/lib/components/surfaces/SurfaceSlot.test.ts --reporter=verbose
```

Expected: the "keeps structural slot container" test case PASSES (0 surface-slot-item found = 0
expected — vacuously true since attr doesn't exist). This is expected — we'll confirm the attr
works when surfaces are present after implementing the change. Move to Step 3.

- [ ] **Step 3: Fix SurfaceSlot.svelte — replace card class + add data-ui + fix h3**

In `frontend/src/lib/components/surfaces/SurfaceSlot.svelte` lines 38-39, change:

```svelte
			<section class="card space-y-4 p-4">
				<h2 class="h3">{surface.label}</h2>
```

to:

```svelte
			<section class="bg-[var(--bg-surface)] rounded-[3px] border border-[var(--border-subtle)] space-y-4 p-4" data-ui="surface-slot-item">
				<h2 class="text-[13px] font-bold text-[var(--text-primary)]">{surface.label}</h2>
```

- [ ] **Step 4: Run Vitest for SurfaceSlot only**

```bash
cd frontend && npm run test -- --run src/lib/components/surfaces/SurfaceSlot.test.ts --reporter=verbose
```

Expected: all 4 tests pass.

- [ ] **Step 5: Run full Vitest suite**

```bash
cd frontend && npm run test -- --run --reporter=verbose 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 6: Run type check**

```bash
cd frontend && npm run check 2>&1 | tail -10
```

Expected: zero errors.

- [ ] **Step 7: Commit**

```bash
cd frontend && git add src/lib/components/surfaces/SurfaceSlot.svelte src/lib/components/surfaces/SurfaceSlot.test.ts
git commit -m "fix(frontend): SurfaceSlot card → token classes, h3 → explicit typography; add data-ui attr"
```

---

## Task 7: Fix SoftwareMergeWizard `h4` typography

**Files:**

- Modify: `frontend/src/lib/components/SoftwareMergeWizard.svelte:237,356,367,382,401`

Spec: `h4` is not in §2.4. Map to h3 values per design decision Q5:
`text-[13px] font-bold text-[var(--text-primary)]`. All five h4 elements in the merge
preview section use `class="h4"`.

- [ ] **Step 1: Verify tests don't assert on the `h4` Skeleton class**

```bash
cd frontend && grep -n '"h4"\| h4\b' src/lib/components/SoftwareMergeWizard.test.ts src/lib/components/software-merge-wizard.test.ts 2>/dev/null
```

Expected: no className assertions. The tests should use text content or role selectors.

- [ ] **Step 2: Replace all `h4` Skeleton class instances**

There are 5 `<h4 class="h4">` elements in `frontend/src/lib/components/SoftwareMergeWizard.svelte` — at lines approximately 237, 356, 367, 382, 401.

Run this to verify the count before editing:

```bash
grep -n 'class="h4"' frontend/src/lib/components/SoftwareMergeWizard.svelte
```

Expected output (5 lines):

```text
237:			<h4 class="h4">Choose the software item to keep</h4>
356:			<h4 class="h4">Keep</h4>
367:			<h4 class="h4">Delete</h4>
382:			<h4 class="h4">Moved links</h4>
401:			<h4 class="h4">Already present</h4>
```

Replace each `class="h4"` with `class="text-[13px] font-bold text-[var(--text-primary)]"` for
all 5 occurrences. They share the same replacement value so do a global replace:

Open `frontend/src/lib/components/SoftwareMergeWizard.svelte` and replace every instance of:

```svelte
class="h4"
```

with:

```svelte
class="text-[13px] font-bold text-[var(--text-primary)]"
```

Confirm with:

```bash
grep -n 'class="h4"' frontend/src/lib/components/SoftwareMergeWizard.svelte
```

Expected: no output (all replaced).

- [ ] **Step 3: Run Vitest**

```bash
cd frontend && npm run test -- --run --reporter=verbose 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 4: Run type check**

```bash
cd frontend && npm run check 2>&1 | tail -20
```

Expected: zero errors.

- [ ] **Step 5: Commit**

```bash
cd frontend && git add src/lib/components/SoftwareMergeWizard.svelte
git commit -m "fix(frontend): SoftwareMergeWizard h4 → text-[13px] font-bold per spec §2.4 (Q5)"
```

---

## Final verification

- [ ] **Run full check + test suite**

```bash
cd frontend && npm run check && npm run test -- --run 2>&1 | tail -10
```

Expected: zero type errors, all Vitest tests pass.

- [ ] **Visual spot-check (browser)**

Start the dev server and open any page that has a TabStrip (e.g. the Settings page). Verify:

1. Active tab shows a **tinted** background (not solid accent fill) with accent-colored text.
2. SectionCard and EmptyState have tight (almost square) 3px corners.
3. PageShell h1 is visually smaller than before (20px vs the old 30px `text-3xl`).
