# Token Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended)
> or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate every remaining Skeleton Labs preset/surface/primary class from `lib/components/`
and all route files, replacing them with design-token equivalents from `frontend/src/theme/tokens.ts`.

**Architecture:** Pure class substitution plus targeted component swaps
(raw `<aside preset-filled-*>` → `<Callout>`, `<span class="badge preset-*">` → `<StatusBadge>`,
`<a class="btn btn-sm preset-tonal">` → `<Button variant="ghost">`).
No logic changes, no API changes. Work file by file; no ordering constraints within the spec.

**Tech Stack:** SvelteKit, Tailwind CSS, Vitest (811 tests), Playwright (baselines may need regen for component swaps)

---

## Token substitution reference

| Skeleton class | Replace with |
| --- | --- |
| `text-surface-400`, `text-surface-500` | `text-[var(--text-muted)]` |
| `text-surface-600 dark:text-surface-400`, `text-surface-600`, `text-surface-700` | `text-[var(--text-secondary)]` |
| `text-surface-900 dark:text-surface-100` | `text-[var(--text-primary)]` |
| `text-surface-700 dark:text-surface-200` | `text-[var(--text-primary)]` |
| `bg-surface-50 dark:bg-surface-900` | `bg-[var(--bg-surface)]` |
| `bg-surface-100 dark:bg-surface-800`, `bg-surface-100/800` | `bg-[var(--bg-raised)]` |
| `border-surface-200 dark:border-surface-700`, `border border-surface-200 dark:border-surface-700` | `border-[var(--border-default)]` |
| `dark:border-surface-600`, `dark:border-surface-700` (standalone) | drop — `--border-default` handles both themes |
| `divide-surface-200 dark:divide-surface-700` | `divide-[var(--border-subtle)]` |
| `hover:bg-surface-100-800-token` | `hover:bg-[var(--bg-hover)]` |
| `hover:bg-surface-200 dark:hover:bg-surface-800` | `hover:bg-[var(--bg-hover)]` |
| `hover:text-surface-700 dark:hover:text-surface-300` | (migrate element to `<Button variant="ghost">`) |
| `rounded-container-token` | `rounded-[3px]` |
| `border-surface-300-600-token` | `border-[var(--border-default)]` |
| `text-error-500` | `text-[var(--color-error)]` |
| `text-success-500` | `text-[var(--color-success)]` |
| `border-t-primary-500` | `border-t-[var(--accent)]` |
| `border-primary-500` | `border-[var(--accent)]` |
| `bg-primary-100 dark:bg-primary-900/40` | `bg-[rgba(var(--accent-rgb),0.12)]` |
| `text-primary-700 dark:text-primary-200` | `text-[var(--accent-bright)]` |
| `preset-filled-error-500` on `<aside>` | `<Callout tone="danger">` |
| `preset-tonal-surface` on `<aside>` | `<Callout tone="info">` |
| `preset-filled-warning-500` on `<aside>` | `<Callout tone="warning">` |
| `preset-filled-error-500` on `<p>` (inline error) | `bg-[var(--color-error-bg)] text-[var(--color-error)] border border-[var(--color-error-border)]` |
| `preset-filled-surface-400-600` | `bg-[var(--bg-raised)]` |
| `badge preset-tonal` | `<StatusBadge tone="info">` |
| `badge preset-tonal-warning` | `<StatusBadge tone="warning">` |
| `badge preset-tonal-error` | `<StatusBadge tone="danger">` |
| `badge preset-tonal-surface` | `<StatusBadge tone="info">` |
| `badge preset-filled-primary-500` | `<StatusBadge tone="info">` |
| `card preset-tonal-primary p-4` | `bg-[rgba(var(--accent-rgb),0.08)] rounded-[3px] border border-[rgba(var(--accent-rgb),0.15)] p-4` |
| `card preset-tonal-surface` | `bg-[var(--bg-raised)] rounded-[3px] border border-[var(--border-subtle)]` |
| `card` (standalone Skeleton utility) | `bg-[var(--bg-surface)] rounded-[3px] border border-[var(--border-subtle)]` |
| `btn btn-sm preset-tonal` on `<a>` | `<Button variant="ghost" size="sm" href=...>` |

---

## Files modified

**lib/components:**
`CheckboxList.svelte`, `AddSoftwareModal.svelte`, `SurfaceKeyValue.svelte`,
`BatchResultDialog.svelte`, `Modal.svelte`, `AssignToHostModal.svelte`,
`EditHostAssignmentModal.svelte`, `BatchActionBar.svelte`, `ToastNotifications.svelte`,
`SoftwareMergeWizard.svelte`, `surfaces/SurfaceWorkflow.svelte`

**routes:**
`+page.svelte`, `surfaces/[id]/+page.svelte`, `history/+page.svelte`,
`audit-logs/+page.svelte`, `profile/+page.svelte`, `hosts/+page.svelte`,
`hosts/[id]/+page.svelte`, `host-tags/+page.svelte`, `software/[id]/+page.svelte`,
`settings/GlobalSettingsTab.svelte`, `settings/PluginConfigsTab.svelte`,
`settings/SchedulerTab.svelte`, `settings/SystemServicesSettings.svelte`

---

## Task 1: Simple token replacements in CheckboxList, AddSoftwareModal, SurfaceKeyValue

**Files:**

- Modify: `frontend/src/lib/components/CheckboxList.svelte:34,38,52,59`
- Modify: `frontend/src/lib/components/AddSoftwareModal.svelte:59`
- Modify: `frontend/src/lib/components/surfaces/SurfaceKeyValue.svelte:16,18,20`

These files have only simple class substitutions — no component swaps needed.

- [ ] **Step 1: Verify no tests assert on the old class names**

```bash
cd frontend && grep -n 'surface-300-600-token\|surface-100-800-token\|rounded-container-token\|text-surface-' src/lib/components/*.test.ts src/lib/components/surfaces/SurfaceKeyValue.test.ts 2>/dev/null
```

Expected: no className assertions in tests.

- [ ] **Step 2: Fix CheckboxList.svelte**

In `frontend/src/lib/components/CheckboxList.svelte`, apply these substitutions:

Line 34 — change:

```svelte
<div class="{maxHeight} overflow-y-auto rounded-container-token border border-surface-300-600-token p-2 space-y-1">
```

to:

```svelte
<div class="{maxHeight} overflow-y-auto rounded-[3px] border border-[var(--border-default)] p-2 space-y-1">
```

Line 38 — change `hover:bg-surface-100-800-token` to `hover:bg-[var(--bg-hover)]`:

```svelte
			? 'opacity-50 cursor-not-allowed'
			: 'hover:bg-[var(--bg-hover)]'}"
```

Line 52 — change `text-surface-500` to `text-[var(--text-muted)]`:

```svelte
			<span class="text-xs text-[var(--text-muted)] truncate">{item.sublabel}</span>
```

Line 59 — change `text-surface-500` to `text-[var(--text-muted)]`:

```svelte
<p class="mt-1 text-xs text-[var(--text-muted)]">{selected.size} selected</p>
```

- [ ] **Step 3: Fix AddSoftwareModal.svelte**

In `frontend/src/lib/components/AddSoftwareModal.svelte` line 59, change:

```svelte
<p class="text-sm text-surface-500">Register a software item to start tracking updates.</p>
```

to:

```svelte
<p class="text-sm text-[var(--text-muted)]">Register a software item to start tracking updates.</p>
```

- [ ] **Step 4: Fix SurfaceKeyValue.svelte**

In `frontend/src/lib/components/surfaces/SurfaceKeyValue.svelte`:

Lines 16 and 18 — change `text-surface-500` to `text-[var(--text-muted)]`:

```svelte
{#if loading}
	<p class="py-8 text-center text-[var(--text-muted)]">Loading...</p>
{:else if entries.length === 0}
	<p class="py-8 text-center text-[var(--text-muted)]">{emptyMessage}</p>
```

Line 20 — change `divide-surface-200 dark:divide-surface-700` to `divide-[var(--border-subtle)]`:

```svelte
	<dl class="divide-y divide-[var(--border-subtle)]">
```

- [ ] **Step 5: Run Vitest**

```bash
cd frontend && npm run test -- --run --reporter=verbose 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
cd frontend && git add src/lib/components/CheckboxList.svelte src/lib/components/AddSoftwareModal.svelte src/lib/components/surfaces/SurfaceKeyValue.svelte
git commit -m "fix(frontend): replace Skeleton surface tokens in CheckboxList, AddSoftwareModal, SurfaceKeyValue"
```

---

## Task 2: Modal, BatchResultDialog — card/bg-surface cleanup

**Files:**

- Modify: `frontend/src/lib/components/Modal.svelte:22`
- Modify: `frontend/src/lib/components/BatchResultDialog.svelte:21,30,36,37,38`

- [ ] **Step 1: Fix Modal.svelte — remove `card` utility, fix bg-surface**

In `frontend/src/lib/components/Modal.svelte` line 22, change:

```svelte
		class="card bg-surface-50 dark:bg-surface-900 z-[910] flex w-full max-h-[calc(100vh-4rem)] flex-col overflow-hidden border border-[var(--border-subtle)] rounded-[4px] {maxWidth} shadow-xl"
```

to:

```svelte
		class="bg-[var(--bg-surface)] z-[910] flex w-full max-h-[calc(100vh-4rem)] flex-col overflow-hidden border border-[var(--border-subtle)] rounded-[4px] {maxWidth} shadow-xl"
```

- [ ] **Step 2: Fix BatchResultDialog.svelte**

In `frontend/src/lib/components/BatchResultDialog.svelte`:

Line 21 — `text-success-500` → `text-[var(--color-success)]`:

```svelte
				<span class="font-medium text-[var(--color-success)]">{response.succeeded.length}</span>
```

Line 30 — `text-error-500` → `text-[var(--color-error)]`:

```svelte
				<span class="font-medium text-[var(--color-error)]">{response.failed.length}</span>
```

Line 36 — `bg-surface-100 dark:bg-surface-800` → `bg-[var(--bg-raised)]`:

```svelte
					<li class="rounded bg-[var(--bg-raised)] px-3 py-2">
```

Line 37 — `text-surface-500` → `text-[var(--text-muted)]`:

```svelte
						<code class="text-xs text-[var(--text-muted)]">{failure.id}</code>
```

Line 38 — `text-error-500` → `text-[var(--color-error)]`:

```svelte
						<p class="text-[var(--color-error)]">{failure.error}</p>
```

- [ ] **Step 3: Run Vitest**

```bash
cd frontend && npm run test -- --run --reporter=verbose 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
cd frontend && git add src/lib/components/Modal.svelte src/lib/components/BatchResultDialog.svelte
git commit -m "fix(frontend): replace Skeleton surface/error tokens in Modal and BatchResultDialog"
```

---

## Task 3: AssignToHostModal — preset-filled → Callout + surface token cleanup

**Files:**

- Modify: `frontend/src/lib/components/AssignToHostModal.svelte:261,271,275,282,375,500`

Two `<aside>` elements with `preset-filled-error-500` and `preset-tonal-surface` need replacing with `<Callout>`. Check if `Callout` is already imported.

- [ ] **Step 1: Check existing imports in AssignToHostModal.svelte**

```bash
grep -n "^import" frontend/src/lib/components/AssignToHostModal.svelte | head -20
```

If `Callout` is not yet imported, add it. It comes from `$lib/components/ui`.

- [ ] **Step 2: Fix the `preset-filled-error-500` aside (line 271)**

Current (around line 270-273):

```svelte
	<aside class="rounded-lg p-4 preset-filled-error-500">
		<p>{loadError}</p>
	</aside>
```

Replace with:

```svelte
	<Callout tone="danger" message={loadError} />
```

- [ ] **Step 3: Fix the `preset-tonal-surface` aside (line 275)**

Current (around line 274-277):

```svelte
	<aside class="rounded-lg p-4 preset-tonal-surface">
		<p class="text-sm">No hosts are registered yet. Hosts appear once an approved agent reports from a machine.</p>
	</aside>
```

Replace with:

```svelte
	<Callout tone="info" message="No hosts are registered yet. Hosts appear once an approved agent reports from a machine." />
```

- [ ] **Step 4: Fix surface tokens in AssignToHostModal.svelte**

Line 261 — `text-surface-500` → `text-[var(--text-muted)]`:

```svelte
<p class="text-sm text-[var(--text-muted)]">
	Select hosts to track <strong>{softwareItemName}</strong> on.
</p>
```

Line 282 — `border-surface-200 dark:border-surface-700` → `border-[var(--border-default)]`. Find:

```svelte
		<div class="space-y-4 border-t border-surface-200 dark:border-surface-700 pt-3">
```

Replace with:

```svelte
		<div class="space-y-4 border-t border-[var(--border-default)] pt-3">
```

Line 375 — `text-surface-400` → `text-[var(--text-muted)]`:

```svelte
					<p class="text-xs text-[var(--text-muted)]">No pre-update hooks configured.</p>
```

Line 500 — `text-surface-400` → `text-[var(--text-muted)]`:

```svelte
					<p class="text-xs text-[var(--text-muted)]">No post-update hooks configured.</p>
```

- [ ] **Step 5: Run Vitest**

```bash
cd frontend && npm run test -- --run --reporter=verbose 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
cd frontend && git add src/lib/components/AssignToHostModal.svelte
git commit -m "fix(frontend): replace preset-filled/tonal aside and surface tokens in AssignToHostModal"
```

---

## Task 4: EditHostAssignmentModal — comprehensive token migration

**Files:**

- Modify: `frontend/src/lib/components/EditHostAssignmentModal.svelte`

This file has the most violations: `preset-filled-error-500` aside (line 685),
`preset-filled-error-500` on inline `<p>` error boxes (lines 838, 871, 966, 998, 1157, 1192),
`badge preset-tonal` (lines 698, 1015), `badge preset-tonal-warning` (lines 882, 1205),
`border-surface-200/700` (lines 696, 1012), and `text-surface-*` scattered through.

First, check what `StatusBadge` API looks like and whether it's already imported:

- [ ] **Step 1: Check existing imports**

```bash
grep -n "^import" frontend/src/lib/components/EditHostAssignmentModal.svelte | head -25
```

If `StatusBadge` is not imported, add it from `$lib/components/ui`. If `Callout` is not imported, add it too.

- [ ] **Step 2: Fix the `preset-filled-error-500` aside (line 685)**

Current:

```svelte
		<aside class="rounded-lg p-4 preset-filled-error-500 text-sm">{loadError}</aside>
```

Replace with:

```svelte
		<Callout tone="danger" message={loadError} />
```

- [ ] **Step 3: Fix inline `preset-filled-error-500` error paragraphs**

There are 6 occurrences of `<p class="text-xs rounded px-2 py-1 preset-filled-error-500">`. Replace ALL of them globally:

Find:

```svelte
class="text-xs rounded px-2 py-1 preset-filled-error-500"
```

Replace with:

```svelte
class="text-xs rounded px-2 py-1 bg-[var(--color-error-bg)] text-[var(--color-error)] border border-[var(--color-error-border)]"
```

Verify the count before and after:

```bash
grep -c 'preset-filled-error-500' frontend/src/lib/components/EditHostAssignmentModal.svelte
```

Expected before: 9 (1 aside + 8 paragraphs — lines 838, 871, 966, 999, 1157, 1192, 1292, 1327).
After fixing aside (step 2) and all paragraphs: 0.

- [ ] **Step 4: Fix `badge preset-tonal` → StatusBadge (lines 698, 1015)**

These are role label badges. Current pattern:

```svelte
<span class="badge preset-tonal shrink-0 text-xs">{ROLE_LABELS[role]}</span>
```

Replace with (`shrink-0` dropped — StatusBadge has no `class` prop; flex container handles sizing):

```svelte
<StatusBadge tone="info" label={ROLE_LABELS[role]} />
```

There are 2 occurrences (lines 698 and 1015). Replace both:

```bash
grep -n 'badge preset-tonal shrink-0' frontend/src/lib/components/EditHostAssignmentModal.svelte
```

For the hook section at line ~1015, there is also `{ROLE_LABELS[hookRole]}s` (plural `s` appended). Handle this carefully:

Line ~1015:

```svelte
<span class="badge preset-tonal shrink-0 text-xs">{ROLE_LABELS[hookRole]}s</span>
```

→

```svelte
<StatusBadge tone="info" label="{ROLE_LABELS[hookRole]}s" />
```

- [ ] **Step 5: Fix `badge preset-tonal-warning` → StatusBadge (lines 882, 1205)**

Current:

```svelte
<span class="ml-1 badge preset-tonal-warning text-xs">set</span>
```

Replace with (StatusBadge has no `class` prop; wrap in `<span class="ml-1">` to preserve margin):

```svelte
<span class="ml-1"><StatusBadge tone="warning" label="set" /></span>
```

Both occurrences (lines 882 and 1205) follow the same pattern. Replace both.

- [ ] **Step 6: Fix `border-surface-200 dark:border-surface-700` (lines 696, 1012)**

Find:

```svelte
class="rounded-lg border border-surface-200 p-4 space-y-3 dark:border-surface-700"
```

Replace with:

```svelte
class="rounded-[3px] border border-[var(--border-default)] p-4 space-y-3"
```

Both occurrences (lines 696 and 1012) have the same pattern.

- [ ] **Step 7: Fix remaining `text-surface-*` tokens**

Run grep to find all remaining:

```bash
grep -n 'text-surface-' frontend/src/lib/components/EditHostAssignmentModal.svelte
```

For each match:

- `text-surface-500` → `text-[var(--text-muted)]`
- `text-surface-400` → `text-[var(--text-muted)]`
- `hover:text-surface-700` → part of a raw button (see below)

Line 879: `class="cursor-pointer select-none text-xs text-surface-500 hover:text-surface-700"` — this is a `<summary>` element, not a button. Apply:

- `text-surface-500` → `text-[var(--text-muted)]`
- `hover:text-surface-700` → `hover:text-[var(--text-primary)]`

All other `text-surface-500` and `text-surface-400`: → `text-[var(--text-muted)]`.

- [ ] **Step 8: Run type check and Vitest**

```bash
cd frontend && npm run check 2>&1 | tail -10 && npm run test -- --run --reporter=verbose 2>&1 | tail -20
```

Expected: zero type errors, all tests pass.

- [ ] **Step 9: Commit**

```bash
cd frontend && git add src/lib/components/EditHostAssignmentModal.svelte
git commit -m "fix(frontend): replace all Skeleton tokens in EditHostAssignmentModal"
```

---

## Task 5: BatchActionBar — raw button migration + surface tokens

**Files:**

- Modify: `frontend/src/lib/components/BatchActionBar.svelte:105,110,112,114,119-124,155`

BatchActionBar has a raw `<button>` with `hover:text-surface-700 dark:hover:text-surface-300`
that should become `<Button variant="ghost">`. Also has `bg-surface-*`, `border-surface-*`,
and `border-t-primary-500` violations.

- [ ] **Step 1: Fix toolbar container (line 105)**

Current:

```svelte
		class="fixed bottom-4 left-1/2 z-50 -translate-x-1/2 rounded-lg bg-surface-50 px-4 py-3 shadow-xl dark:bg-surface-900 border border-surface-200 dark:border-surface-700"
```

Replace with:

```svelte
		class="fixed bottom-4 left-1/2 z-50 -translate-x-1/2 rounded-[3px] bg-[var(--bg-surface)] px-4 py-3 shadow-xl border border-[var(--border-default)]"
```

- [ ] **Step 2: Fix `text-surface-500` on the status line (line 110)**

```svelte
		<div class="mb-2 text-center text-sm text-[var(--text-muted)]">
```

- [ ] **Step 3: Fix the spinner border (line 114)**

Current:

```svelte
					class="inline-block h-3 w-3 animate-spin rounded-full border-2 border-surface-300 border-t-primary-500"
```

Replace with:

```svelte
					class="inline-block h-3 w-3 animate-spin rounded-full border-2 border-[var(--border-default)] border-t-[var(--accent)]"
```

- [ ] **Step 4: Migrate the raw `<button>` to `<Button variant="ghost">` (lines 119-124)**

Current:

```svelte
				<button
					class="cursor-pointer underline hover:text-surface-700 dark:hover:text-surface-300"
					onclick={selectAllPages.onSelect}
				>
					Select all {selectAllPages.total} items across all pages
				</button>
```

Replace with (check Button import at top of file first):

```svelte
				<Button variant="ghost" onclick={selectAllPages.onSelect}>
					Select all {selectAllPages.total} items across all pages
				</Button>
```

- [ ] **Step 5: Fix the "More actions" dropdown container (line 155)**

Current:

```svelte
					class="absolute bottom-full left-0 mb-2 min-w-[10rem] overflow-hidden rounded-lg border border-surface-200 bg-surface-50 p-1 shadow-xl dark:border-surface-700 dark:bg-surface-900"
```

Replace with:

```svelte
					class="absolute bottom-full left-0 mb-2 min-w-[10rem] overflow-hidden rounded-[3px] border border-[var(--border-default)] bg-[var(--bg-surface)] p-1 shadow-xl"
```

- [ ] **Step 6: Run Vitest**

```bash
cd frontend && npm run test -- --run --reporter=verbose 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
cd frontend && git add src/lib/components/BatchActionBar.svelte
git commit -m "fix(frontend): migrate BatchActionBar raw button to <Button variant=ghost>, replace surface tokens"
```

---

## Task 6: ToastNotifications + SoftwareMergeWizard token migration

**Files:**

- Modify: `frontend/src/lib/components/ToastNotifications.svelte:387`
- Modify: `frontend/src/lib/components/SoftwareMergeWizard.svelte:322,324,327,343,349-354,358,361,363,371,374,376`

### ToastNotifications

- [ ] **Step 1: Verify Button is imported in ToastNotifications.svelte**

```bash
grep -n "^import.*Button" frontend/src/lib/components/ToastNotifications.svelte
```

If not present, add: `import Button from '$lib/components/Button.svelte';`

- [ ] **Step 2: Migrate `<a class="btn btn-sm preset-tonal">` (line 387)**

Current:

```svelte
								<a href="/settings/global" class="btn btn-sm preset-tonal">Go to Global Settings</a>
```

Replace with:

```svelte
								<Button variant="ghost" size="sm" href="/settings/global">Go to Global Settings</Button>
```

### SoftwareMergeWizard

- [ ] **Step 3: Verify StatusBadge is imported in SoftwareMergeWizard.svelte**

```bash
grep -n "^import.*StatusBadge" frontend/src/lib/components/SoftwareMergeWizard.svelte
```

If not present, add: `import { StatusBadge } from '$lib/components/ui';`

- [ ] **Step 4: Fix badge classes and text-surface tokens in the candidate list**

Line 322 — `badge preset-tonal-surface text-xs` → `<StatusBadge tone="info">`:

Current:

```svelte
							<span class="badge preset-tonal-surface text-xs">{candidate.host_count} host(s)</span>
```

Replace with:

```svelte
							<StatusBadge tone="info" label="{candidate.host_count} host(s)" />
```

Line 324 — `badge preset-filled-primary-500 text-xs` → `<StatusBadge tone="info">`:

Current:

```svelte
								<span class="badge preset-filled-primary-500 text-xs">Seed item</span>
```

Replace with:

```svelte
								<StatusBadge tone="info" label="Seed item" />
```

Line 327 — `text-surface-500` → `text-[var(--text-muted)]`:

```svelte
						<p class="mt-1 text-sm text-[var(--text-muted)]">Plugins: {pluginSummary(candidate)}</p>
```

Line 343 — `text-surface-500` → `text-[var(--text-muted)]`:

```svelte
		<p class="text-sm text-[var(--text-muted)]">
```

- [ ] **Step 5: Fix the preview info card (line 349)**

Current:

```svelte
		<div class="card preset-tonal-primary p-4">
			<p class="text-sm text-surface-700 dark:text-surface-200">
```

Replace with:

```svelte
		<div class="bg-[rgba(var(--accent-rgb),0.08)] rounded-[3px] border border-[rgba(var(--accent-rgb),0.15)] p-4">
			<p class="text-sm text-[var(--text-primary)]">
```

- [ ] **Step 6: Fix the Keep/Delete/Moved/Present section cards and badges**

Line 358 — `card p-4` → token card:

```svelte
				<div class="bg-[var(--bg-surface)] rounded-[3px] border border-[var(--border-subtle)] p-4">
```

Line 361 — `badge preset-filled-primary-500 text-xs` → `<StatusBadge tone="info">`:

```svelte
					<StatusBadge tone="info" label="Survivor" />
```

Line 363 — `text-surface-500` → `text-[var(--text-muted)]`:

```svelte
					<p class="mt-1 text-sm text-[var(--text-muted)]">{preview.survivor.host_count} host(s)</p>
```

Line 371 — `card p-4` (inside `{#each}`) → token card:

```svelte
					<div class="bg-[var(--bg-surface)] rounded-[3px] border border-[var(--border-subtle)] p-4">
```

Line 374 — `badge preset-tonal-error text-xs` → `<StatusBadge tone="danger">`:

```svelte
						<StatusBadge tone="danger" label="Merged away" />
```

Line 376 — `text-surface-500` → `text-[var(--text-muted)]`:

```svelte
					<p class="mt-1 text-sm text-[var(--text-muted)]">{loser.host_count} host(s)</p>
```

Repeat `card p-4` → token card for the Moved links and Already present sections (lines ~383 and ~403).
Repeat `text-surface-500` → `text-[var(--text-muted)]` for any remaining occurrences in that section.

Verify all remaining Skeleton violations are gone:

```bash
grep -n 'preset-\|text-surface-\|bg-surface-\|border-surface-\|badge ' frontend/src/lib/components/SoftwareMergeWizard.svelte
```

Expected: only intentional non-Skeleton references (none).

- [ ] **Step 7: Run Vitest**

```bash
cd frontend && npm run test -- --run --reporter=verbose 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
cd frontend && git add src/lib/components/ToastNotifications.svelte src/lib/components/SoftwareMergeWizard.svelte
git commit -m "fix(frontend): replace Skeleton preset/surface tokens in ToastNotifications and SoftwareMergeWizard"
```

---

## Task 7: SurfaceWorkflow token migration

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte`

SurfaceWorkflow has `card preset-tonal-surface` (line 420), `text-primary-*`/`bg-primary-*`,
`border-primary-500` (spinner), and `text-surface-*` scattered throughout.

- [ ] **Step 1: Audit all violations in SurfaceWorkflow.svelte**

```bash
grep -n 'preset-\|text-surface-\|bg-surface-\|border-surface-\|bg-primary-\|text-primary-\|border-primary-' frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte
```

Note every line number and violation. Cross-reference against the substitution table at the top of this plan.

- [ ] **Step 2: Fix the action card (line 420)**

Current:

```svelte
						<label class="card flex items-start gap-3 p-3 {isChecked ? 'preset-tonal-surface' : 'opacity-60'}">
```

Replace with:

```svelte
						<label class="rounded-[3px] border border-[var(--border-subtle)] flex items-start gap-3 p-3 {isChecked ? 'bg-[var(--bg-raised)]' : 'bg-[var(--bg-surface)] opacity-60'}">
```

- [ ] **Step 3: Fix the loading spinner (line 468)**

Current:

```svelte
					<div class="border-primary-500 h-8 w-8 animate-spin rounded-full border-4 border-t-transparent"></div>
```

Replace with:

```svelte
					<div class="border-[var(--accent)] h-8 w-8 animate-spin rounded-full border-4 border-t-transparent"></div>
```

- [ ] **Step 4: Replace all remaining `text-surface-*`, `bg-primary-*`, `text-primary-*` per the substitution table**

For each violation found in step 1, apply the substitution from the table at the top of this plan. Common patterns:

- `text-surface-500` → `text-[var(--text-muted)]`
- `bg-primary-100 dark:bg-primary-900/40` → `bg-[rgba(var(--accent-rgb),0.12)]`
- `text-primary-700 dark:text-primary-200` → `text-[var(--accent-bright)]`
- `border-primary-500` → `border-[var(--accent)]`

- [ ] **Step 5: Verify no Skeleton classes remain**

```bash
grep -n 'preset-\|text-surface-\|bg-surface-\|border-surface-\|bg-primary-\|text-primary-\|border-primary-' frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte
```

Expected: no output.

- [ ] **Step 6: Run Vitest**

```bash
cd frontend && npm run test -- --run --reporter=verbose 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
cd frontend && git add src/lib/components/surfaces/SurfaceWorkflow.svelte
git commit -m "fix(frontend): replace Skeleton preset/primary/surface tokens in SurfaceWorkflow"
```

---

## Task 8: Simple token replacements in route files

**Files:**

- Modify: `frontend/src/routes/+page.svelte:161`
- Modify: `frontend/src/routes/surfaces/[id]/+page.svelte:54,62`
- Modify: `frontend/src/routes/audit-logs/+page.svelte:216`
- Modify: `frontend/src/routes/profile/+page.svelte:182`
- Modify: `frontend/src/routes/host-tags/+page.svelte:480`
- Modify: `frontend/src/routes/settings/PluginConfigsTab.svelte:1276,1288`
- Modify: `frontend/src/routes/settings/SchedulerTab.svelte:124,140`
- Modify: `frontend/src/routes/settings/SystemServicesSettings.svelte:199`

These files have only simple `text-surface-*`, `bg-surface-*`, or `text-error-500` violations.

- [ ] **Step 1: Audit each file to confirm exact violations**

```bash
grep -n 'text-surface-\|bg-surface-\|text-error-500\|text-success-500\|preset-' \
  frontend/src/routes/+page.svelte \
  frontend/src/routes/surfaces/[id]/+page.svelte \
  frontend/src/routes/audit-logs/+page.svelte \
  frontend/src/routes/profile/+page.svelte \
  frontend/src/routes/host-tags/+page.svelte \
  frontend/src/routes/settings/PluginConfigsTab.svelte \
  frontend/src/routes/settings/SchedulerTab.svelte \
  frontend/src/routes/settings/SystemServicesSettings.svelte
```

Note: `routes/settings/+page.svelte:250` is listed in the audit but was already migrated —
confirm it shows `text-[var(--text-secondary)]` and skip if so.

- [ ] **Step 2: Apply substitutions**

For each violation found in step 1, apply per the table at the top of this plan:

**routes/+page.svelte:161** — `text-surface-500` → `text-[var(--text-muted)]`

**routes/surfaces/[id]/+page.svelte:54,62** — `text-surface-500` → `text-[var(--text-muted)]`

**routes/audit-logs/+page.svelte:216** — `text-surface-500` → `text-[var(--text-muted)]`

**routes/profile/+page.svelte:182** — `bg-surface-100 dark:bg-surface-800` → `bg-[var(--bg-raised)]`:

```svelte
			class="rounded-md bg-[var(--bg-raised)] p-3 font-mono text-sm break-all whitespace-pre-wrap"
```

**routes/host-tags/+page.svelte:480** — `text-surface-400` → `text-[var(--text-muted)]`

**routes/settings/PluginConfigsTab.svelte:1276** — `text-surface-500` → `text-[var(--text-muted)]`

**routes/settings/PluginConfigsTab.svelte:1288** — `text-error-500` → `text-[var(--color-error)]`

**routes/settings/SchedulerTab.svelte:124** — `text-surface-500` → `text-[var(--text-muted)]`

**routes/settings/SchedulerTab.svelte:129** — `text-surface-500` → `text-[var(--text-muted)]`

**routes/settings/SchedulerTab.svelte:141** — `text-error-500` → `text-[var(--color-error)]`

**routes/settings/SystemServicesSettings.svelte:200** — `text-surface-600 dark:text-surface-400`
→ `text-[var(--text-secondary)]`

- [ ] **Step 3: Run Vitest**

```bash
cd frontend && npm run test -- --run --reporter=verbose 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
cd frontend && git add \
  src/routes/+page.svelte \
  src/routes/surfaces/\[id\]/+page.svelte \
  src/routes/audit-logs/+page.svelte \
  src/routes/profile/+page.svelte \
  src/routes/host-tags/+page.svelte \
  src/routes/settings/PluginConfigsTab.svelte \
  src/routes/settings/SchedulerTab.svelte \
  src/routes/settings/SystemServicesSettings.svelte
git commit -m "fix(frontend): replace residual Skeleton surface/error tokens in route files"
```

---

## Task 9: routes/hosts — text-surface, text-error, raw button migration

**Files:**

- Modify: `frontend/src/routes/hosts/+page.svelte:437,563`
- Modify: `frontend/src/routes/hosts/[id]/+page.svelte:584,607,624,643`

**hosts/+page.svelte** has `text-surface-400` and a raw button menu item with `text-error-500 hover:bg-surface-200`.

**hosts/[id]/+page.svelte** has `btn btn-sm preset-tonal` on an `<a>`, a `preset-tonal-surface` aside, a `badge preset-tonal`, and `text-surface-500`.

- [ ] **Step 1: Audit hosts/+page.svelte for all violations**

```bash
grep -n 'text-surface-\|bg-surface-\|text-error-500\|preset-\|hover:bg-surface-' frontend/src/routes/hosts/+page.svelte
```

- [ ] **Step 2: Fix hosts/+page.svelte**

Line 437 — `text-surface-400` → `text-[var(--text-muted)]`:

```svelte
					<span class="text-[var(--text-muted)]">&mdash;</span>
```

Line 563 — raw button menu item with `text-error-500 hover:bg-surface-200 dark:hover:bg-surface-800`.
This is a `<button>` that acts as a danger action in a context menu. Replace class:

```svelte
			class="w-full rounded-md px-3 py-2 text-left text-sm text-[var(--color-error)] hover:bg-[var(--bg-hover)]"
```

- [ ] **Step 3: Audit hosts/[id]/+page.svelte for all violations**

```bash
grep -n 'text-surface-\|bg-surface-\|text-error-500\|preset-\|btn btn-sm' frontend/src/routes/hosts/\[id\]/+page.svelte
```

- [ ] **Step 4: Fix hosts/[id]/+page.svelte**

Line 584 — `<a class="btn btn-sm preset-tonal">` → `<Button variant="ghost" size="sm">`. Check that Button is imported, then:

Current:

```svelte
							<a href="/software/{item.id}" class="btn btn-sm preset-tonal">View</a>
```

Replace with:

```svelte
							<Button variant="ghost" size="sm" href="/software/{item.id}">View</Button>
```

Line 607 — `text-surface-500` → `text-[var(--text-muted)]`

Line 624 — `<aside class="rounded-lg p-4 preset-tonal-surface text-sm">` → `<Callout tone="info">`. Current:

```svelte
					<aside class="rounded-lg p-4 preset-tonal-surface text-sm">
						<p>
							No host-specific allowlist configured. Add an entry to restrict which discovery plugins run on this
							host — any host-specific entries will override the tenant-wide allowlist completely.
						</p>
					</aside>
```

Replace with:

```svelte
					<Callout tone="info" message="No host-specific allowlist configured. Add an entry to restrict which discovery plugins run on this host — any host-specific entries will override the tenant-wide allowlist completely." />
```

(Check if Callout is already imported; add `import { Callout } from '$lib/components/ui';` if not.)

Line 643 — `<span class="badge preset-tonal">` → `<StatusBadge tone="info">`. Current:

```svelte
								<td><span class="badge preset-tonal">{entry.plugin_type}</span></td>
```

Replace with:

```svelte
								<td><StatusBadge tone="info" label={entry.plugin_type} /></td>
```

(Check if StatusBadge is already imported; add `import { StatusBadge } from '$lib/components/ui';` if not.)

- [ ] **Step 5: Verify no violations remain**

```bash
grep -n 'text-surface-\|bg-surface-\|preset-\|btn btn-sm' \
  frontend/src/routes/hosts/+page.svelte \
  frontend/src/routes/hosts/\[id\]/+page.svelte
```

Expected: no output.

- [ ] **Step 6: Run Vitest**

```bash
cd frontend && npm run test -- --run --reporter=verbose 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
cd frontend && git add src/routes/hosts/+page.svelte "src/routes/hosts/[id]/+page.svelte"
git commit -m "fix(frontend): migrate hosts pages — preset/surface tokens, btn → Button, badge → StatusBadge"
```

---

## Task 10: routes/software/[id], history — badge, card, h3, text-error

**Files:**

- Modify: `frontend/src/routes/software/[id]/+page.svelte:848-858`
- Modify: `frontend/src/routes/history/+page.svelte:682,686,711`

- [ ] **Step 1: Fix software/[id]/+page.svelte**

Audit first:

```bash
grep -n 'preset-\|text-surface-' "frontend/src/routes/software/[id]/+page.svelte"
```

Line 848 — `badge preset-tonal text-xs` inside a `{#if}` → `<StatusBadge tone="info">`. Current:

```svelte
							>{#if host.qualifier}<span class="badge preset-tonal text-xs ml-1 font-mono">{host.qualifier}</span
```

Replace with (check StatusBadge import):

```svelte
							>{#if host.qualifier}<StatusBadge tone="info" label={host.qualifier} class="ml-1 font-mono"
```

Wait — the pattern here is inline with surrounding content. Be careful to preserve the whitespace and adjacent text. The full context:

```svelte
						<a href="/hosts/{host.host_id}" class="hover:underline font-medium">{host.hostname}</a
						>{#if host.qualifier}<span class="badge preset-tonal text-xs ml-1 font-mono">{host.qualifier}</span
							>{/if}
```

Replace the span (StatusBadge has no `class` prop; drop `ml-1 font-mono` — the badge has its own
spacing and the qualifier is short enough that extra font/spacing is not needed):

```svelte
>{#if host.qualifier}<StatusBadge tone="info" label={host.qualifier} />{/if}
```

Lines 851, 856, 857 — `text-surface-500` → `text-[var(--text-muted)]`.

- [ ] **Step 2: Fix history/+page.svelte**

Audit:

```bash
grep -n 'preset-\|text-surface-\|bg-surface-\|text-error-\|class="h3"\|card ' frontend/src/routes/history/+page.svelte
```

Line 682 — inline dialog `card bg-surface-50 dark:bg-surface-900` → token classes. Current:

```svelte
			class="card bg-surface-50 dark:bg-surface-900 w-full max-w-lg space-y-4 p-6 shadow-xl"
```

Replace with:

```svelte
			class="bg-[var(--bg-surface)] rounded-[3px] border border-[var(--border-subtle)] w-full max-w-lg space-y-4 p-6 shadow-xl"
```

Line 686 — `<h3 class="h3">` — this is Skeleton typography in a route file (not in a primitive, so it falls under token migration). Replace:

```svelte
			<h3 class="text-[13px] font-bold text-[var(--text-primary)]">Trigger Software Update</h3>
```

Line 711 — `text-error-500` → `text-[var(--color-error)]`:

```svelte
					<span>Target Version <span class="text-[var(--color-error)]">*</span></span>
```

- [ ] **Step 3: Run Vitest**

```bash
cd frontend && npm run test -- --run --reporter=verbose 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
cd frontend && git add "src/routes/software/[id]/+page.svelte" src/routes/history/+page.svelte
git commit -m "fix(frontend): badge → StatusBadge, card/surface tokens, h3 typography in software and history routes"
```

---

## Task 11: settings/GlobalSettingsTab — remaining surface/preset tokens

**Files:**

- Modify: `frontend/src/routes/settings/GlobalSettingsTab.svelte`

The spec mentions `text-surface-*`, `bg-surface-100-900`, `preset-filled-warning-500`,
`preset-filled-surface-400-600`. From the code read, lines 280-360 already use `Callout`
properly. Violations are likely elsewhere in this large file.

- [ ] **Step 1: Audit all violations**

```bash
grep -n 'preset-\|text-surface-\|bg-surface-\|border-surface-\|divide-surface-' frontend/src/routes/settings/GlobalSettingsTab.svelte
```

- [ ] **Step 2: Apply substitutions**

For each match, apply per the substitution table at the top of this plan:

- `preset-filled-warning-500` on `<aside>` → `<Callout tone="warning">`
- `preset-filled-surface-400-600` → `bg-[var(--bg-raised)]`
- `text-surface-*` → appropriate `text-[var(--text-*)]` per table
- `bg-surface-*` → appropriate `bg-[var(--bg-*)]` per table

After applying all substitutions, verify:

```bash
grep -n 'preset-\|text-surface-\|bg-surface-\|border-surface-' frontend/src/routes/settings/GlobalSettingsTab.svelte
```

Expected: no output.

- [ ] **Step 3: Run Vitest**

```bash
cd frontend && npm run test -- --run --reporter=verbose 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 4: Run full type check**

```bash
cd frontend && npm run check 2>&1 | tail -10
```

Expected: zero errors.

- [ ] **Step 5: Commit**

```bash
cd frontend && git add src/routes/settings/GlobalSettingsTab.svelte
git commit -m "fix(frontend): replace remaining Skeleton tokens in GlobalSettingsTab"
```

---

## Final sweep and verification

- [ ] **Sweep entire frontend for any remaining Skeleton violations**

```bash
cd frontend && grep -rn 'preset-filled-\|preset-tonal\|text-surface-\|bg-surface-\|border-surface-\|divide-surface-\|bg-primary-\|text-primary-\|border-primary-\|text-error-500\|text-success-500\|btn btn-sm\|badge preset\| h3\b\| h4\b' src/ --include="*.svelte" | grep -v '.test.ts'
```

Expected: no output (or only false positives already documented in the audit report).

- [ ] **Run full type check + test suite**

```bash
cd frontend && npm run check && npm run test -- --run 2>&1 | tail -10
```

Expected: zero type errors, all Vitest tests pass.

- [ ] **Dark-mode smoke test**

Open the app in both light and dark mode. Navigate to a page with migrated components (e.g. hosts list, software detail, settings). Confirm:

1. Muted text is visibly lighter/dimmer than primary text in both modes.
2. Error/success indicators have their correct colors.
3. No component appears broken or unstyled.

- [ ] **Playwright baseline update (if needed)**

Component-level substitutions (`<aside>` → `<Callout>`, `<span class="badge">` → `<StatusBadge>`,
`<a class="btn">` → `<Button>`) produce visual diffs. If parity tests fail after the component
swaps:

```bash
cd frontend && npx playwright test ui-parity ui-parity-responsive --update-snapshots
```

Must run on macOS + Chromium. Visually verify the new snapshots before committing:

```bash
cd frontend && git add tests/e2e/
git commit -m "chore(frontend): update parity baselines after Skeleton component → primitive swaps"
```
