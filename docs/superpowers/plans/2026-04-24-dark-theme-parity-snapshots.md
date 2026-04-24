# Dark Theme Parity Snapshots Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `chromium-dark` Playwright project so every parity screenshot test
automatically captures dark-theme baselines alongside the existing light ones.

**Architecture:** Project-level parameterization via a new `chromium-dark` Playwright project
with `colorScheme: 'dark'`. A shared `parity-fixtures.ts` module exports `parityTest`
(a `test.extend` with a `parityTheme` fixture) and `freezeParityInputs` (theme-aware setup
helper). Both `ui-parity.test.ts` and `ui-parity-responsive.test.ts` swap their imports and
`beforeEach` to use the fixture. `parity-config.ts` gets colorScheme enforcement added to
`assertDeterministicCaptureProfile`.

**Tech Stack:** Playwright 1.x (`test.extend`, `page.emulateMedia`, `page.addInitScript`),
TypeScript, SvelteKit dev server.

---

## File Map

- Create: `frontend/tests/e2e/parity-fixtures.ts`
- Modify: `frontend/playwright.config.ts`
- Modify: `frontend/tests/e2e/parity-config.ts`
- Modify: `frontend/tests/e2e/ui-parity.test.ts`
- Modify: `frontend/tests/e2e/ui-parity-responsive.test.ts`
- Modify: `docs/development/ui/surfaces.md`
- Generate: `frontend/tests/e2e/ui-parity.test.ts-snapshots/*-chromium-dark.png`
- Generate: `frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots/*-chromium-dark.png`

---

## Task 1: Create `parity-fixtures.ts`

**Files:**

- Create: `frontend/tests/e2e/parity-fixtures.ts`

This module owns the `parityTheme` fixture and `freezeParityInputs`. Single source of truth
for theme setup during parity captures.

- [ ] **Step 1: Create the file**

```typescript
// frontend/tests/e2e/parity-fixtures.ts
import { test as base } from '@playwright/test';
import type { Page, TestInfo } from '@playwright/test';

export type ParityTheme = 'light' | 'dark';

export const parityTest = base.extend<{ parityTheme: ParityTheme }>({
	parityTheme: async ({}, use, testInfo: TestInfo) => {
		const theme = testInfo.project.name.includes('dark') ? 'dark' : 'light';
		await use(theme);
	}
});

export async function freezeParityInputs(page: Page, theme: ParityTheme) {
	await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
	await page.addInitScript((t) => {
		localStorage.setItem('theme-mode', t);
	}, theme);
}
```

- [ ] **Step 2: Verify TypeScript compiles**

```sh
cd frontend && npx tsc --noEmit
```

Expected: no errors.

- [ ] **Step 3: Commit**

```sh
git add frontend/tests/e2e/parity-fixtures.ts
git commit -m "feat(tests): add parity-fixtures with parityTheme fixture and freezeParityInputs"
```

---

## Task 2: Update `playwright.config.ts`

**Files:**

- Modify: `frontend/playwright.config.ts`

Add the `chromium-dark` project. Add explicit `colorScheme: 'light'` to the existing
`chromium` project.

- [ ] **Step 1: Update the projects array (lines 32–37)**

Replace the existing `projects` block:

```typescript
	projects: [
		{
			name: 'chromium',
			use: { ...devices['Desktop Chrome'], colorScheme: 'light' }
		},
		{
			name: 'chromium-dark',
			use: { ...devices['Desktop Chrome'], colorScheme: 'dark' }
		}
	],
```

- [ ] **Step 2: Verify both projects are visible to Playwright**

```sh
cd frontend && npx playwright test --list --project=chromium 2>&1 | tail -5
cd frontend && npx playwright test --list --project=chromium-dark 2>&1 | tail -5
```

Expected: both commands list tests without errors.

- [ ] **Step 3: Commit**

```sh
git add frontend/playwright.config.ts
git commit -m "feat(tests): add chromium-dark Playwright project for dark theme parity"
```

---

## Task 3: Update `parity-config.ts`

**Files:**

- Modify: `frontend/tests/e2e/parity-config.ts`

Three changes: (1) `assertProjectGuard` uses `startsWith` so `chromium-dark` passes,
(2) `page.evaluate` collects `prefersDark`, (3) `assertDeterministicCaptureProfile`
enforces colorScheme matches project.

- [ ] **Step 1: Fix `assertProjectGuard` (lines 33–40)**

Replace the function body:

```typescript
function assertProjectGuard() {
	const projectName = test.info().project.name;
	if (!projectName.startsWith(PARITY_REQUIRED_PROJECT)) {
		throw new Error(
			`ui parity harness requires Playwright project "${PARITY_REQUIRED_PROJECT}" ` +
				`(or a variant), received "${projectName}".`
		);
	}
}
```

- [ ] **Step 2: Add `prefersDark` to `page.evaluate` (lines 155–161)**

Replace the `page.evaluate` call:

```typescript
	const env = await page.evaluate(() => ({
		language: navigator.language,
		timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
		reducedMotion: window.matchMedia('(prefers-reduced-motion: reduce)').matches,
		devicePixelRatio: window.devicePixelRatio,
		prefersDark: window.matchMedia('(prefers-color-scheme: dark)').matches
	}));
```

- [ ] **Step 3: Add colorScheme enforcement after the DPR check (after line 172)**

After the existing DPR check:

```typescript
	if (Math.abs(env.devicePixelRatio - 1) > 0.001) {
		throw new Error(`ui parity DPR drift: expected 1, received ${env.devicePixelRatio}.`);
	}
```

Add:

```typescript
	const projectName = test.info().project.name;
	const expectedDark = projectName.includes('dark');
	if (env.prefersDark !== expectedDark) {
		throw new Error(
			`ui parity colorScheme mismatch: project "${projectName}" expects ` +
				`${expectedDark ? 'dark' : 'light'} but page has ` +
				`${env.prefersDark ? 'dark' : 'light'}.`
		);
	}
```

- [ ] **Step 4: Verify TypeScript compiles**

```sh
cd frontend && npx tsc --noEmit
```

Expected: no errors.

- [ ] **Step 5: Run governance tests under the light project**

```sh
cd frontend && npx playwright test --project=chromium ui-parity.test.ts --grep "governance" 2>&1 | tail -20
```

Expected: all governance tests pass.

- [ ] **Step 6: Commit**

```sh
git add frontend/tests/e2e/parity-config.ts
git commit -m "feat(tests): enforce colorScheme in parity harness, allow chromium-dark project"
```

---

## Task 4: Update `ui-parity.test.ts`

**Files:**

- Modify: `frontend/tests/e2e/ui-parity.test.ts`

Five changes: import swap, delete local `freezeParityInputs`, remove `colorScheme: 'light'`
from `test.use`, update `beforeEach`, fix governance `reject reduced-motion` test.

- [ ] **Step 1: Swap imports (line 1)**

Replace:

```typescript
import { expect, test } from '@playwright/test';
```

With:

```typescript
import { expect } from '@playwright/test';
import { parityTest as test, freezeParityInputs } from './parity-fixtures';
```

- [ ] **Step 2: Remove `colorScheme: 'light'` from `test.use` (lines 21–26)**

Replace the `test.use` block:

```typescript
test.use({
	viewport: PARITY_VIEWPORT_PRESETS.desktop,
	locale: 'en-US',
	timezoneId: 'UTC'
});
```

(`colorScheme` is now set by the project config — remove it here.)

- [ ] **Step 3: Delete the local `freezeParityInputs` function (lines 228–233)**

Delete these lines entirely — the function is now imported:

```typescript
async function freezeParityInputs(page: Page) {
	await page.addInitScript(() => {
		localStorage.setItem('theme-mode', 'light');
	});
	await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'reduce' });
}
```

- [ ] **Step 4: Update `beforeEach` to receive and forward `parityTheme` (lines 591–594)**

Replace:

```typescript
test.beforeEach(async ({ page }) => {
	test.skip(!isCanonicalUiParityHost, canonicalUiParityReason);
	await freezeParityInputs(page);
});
```

With:

```typescript
test.beforeEach(async ({ page, parityTheme }) => {
	test.skip(!isCanonicalUiParityHost, canonicalUiParityReason);
	await freezeParityInputs(page, parityTheme);
});
```

- [ ] **Step 5: Fix `governance: reject reduced-motion` test (line 639)**

Inside the `'ui parity governance: reject reduced-motion drift'` test, find:

```typescript
	await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'no-preference' });
```

Replace with:

```typescript
	await page.emulateMedia({ reducedMotion: 'no-preference' });
```

(`colorScheme` must not be hardcoded — it follows the project config so the dark project
doesn't trigger a spurious colorScheme mismatch before the reducedMotion check fires.)

- [ ] **Step 6: Verify TypeScript compiles**

```sh
cd frontend && npx tsc --noEmit
```

Expected: no errors.

- [ ] **Step 7: Run light parity suite**

```sh
cd frontend && npx playwright test --project=chromium ui-parity.test.ts 2>&1 | tail -20
```

Expected: all tests pass (same results as before these changes).

- [ ] **Step 8: Commit**

```sh
git add frontend/tests/e2e/ui-parity.test.ts
git commit -m "refactor(tests): use parityTest fixture and shared freezeParityInputs in ui-parity"
```

---

## Task 5: Update `ui-parity-responsive.test.ts`

**Files:**

- Modify: `frontend/tests/e2e/ui-parity-responsive.test.ts`

Three changes: import swap, delete local `freezeParityInputs`, update `beforeEach`.

- [ ] **Step 1: Swap imports (line 1)**

Replace:

```typescript
import { expect, test } from '@playwright/test';
```

With:

```typescript
import { expect } from '@playwright/test';
import { parityTest as test, freezeParityInputs } from './parity-fixtures';
```

- [ ] **Step 2: Delete the local `freezeParityInputs` function (lines 143–148)**

Delete these lines entirely — the function is now imported:

```typescript
async function freezeParityInputs(page: Page) {
	await page.addInitScript(() => {
		localStorage.setItem('theme-mode', 'light');
	});
	await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'reduce' });
}
```

- [ ] **Step 3: Update `beforeEach` (lines 358–361)**

Replace:

```typescript
test.beforeEach(async ({ page }) => {
	test.skip(!isCanonicalUiParityHost, canonicalUiParityReason);
	await freezeParityInputs(page);
});
```

With:

```typescript
test.beforeEach(async ({ page, parityTheme }) => {
	test.skip(!isCanonicalUiParityHost, canonicalUiParityReason);
	await freezeParityInputs(page, parityTheme);
});
```

- [ ] **Step 4: Verify TypeScript compiles**

```sh
cd frontend && npx tsc --noEmit
```

Expected: no errors.

- [ ] **Step 5: Run light responsive suite**

```sh
cd frontend && npx playwright test --project=chromium ui-parity-responsive.test.ts 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```sh
git add frontend/tests/e2e/ui-parity-responsive.test.ts
git commit -m "refactor(tests): use parityTest fixture in ui-parity-responsive"
```

---

## Task 6: Dry-run dark project (expect baseline failures)

Verify wiring before generating baselines. **Must run on macOS** — the canonical host guard
(`isCanonicalUiParityHost = process.platform === 'darwin'`) skips all parity tests on other
platforms.

- [ ] **Step 1: Dry-run dark parity suite**

```sh
cd frontend && npx playwright test --project=chromium-dark ui-parity.test.ts 2>&1 \
  | grep -E "passed|failed|skipped|Error" | head -20
```

Expected: governance tests pass; screenshot tests fail with
`"ui-parity-*-chromium-dark.png" is missing in snapshots`.

If you see a `colorScheme mismatch`, `viewport mismatch`, or TypeScript error — stop and fix
before proceeding.

- [ ] **Step 2: Dry-run dark responsive suite**

```sh
cd frontend && npx playwright test --project=chromium-dark ui-parity-responsive.test.ts 2>&1 \
  | grep -E "passed|failed|skipped|Error" | head -20
```

Expected: same — missing snapshot errors only.

---

## Task 7: Generate dark baselines

**Must run on macOS.**

- [ ] **Step 1: Generate baselines for `ui-parity.test.ts`**

```sh
cd frontend && npx playwright test --project=chromium-dark ui-parity.test.ts \
  --update-snapshots 2>&1 | tail -10
```

Expected: all tests pass; new `*-chromium-dark.png` files created in
`frontend/tests/e2e/ui-parity.test.ts-snapshots/`.

- [ ] **Step 2: Generate baselines for `ui-parity-responsive.test.ts`**

```sh
cd frontend && npx playwright test --project=chromium-dark ui-parity-responsive.test.ts \
  --update-snapshots 2>&1 | tail -10
```

Expected: new `*-chromium-dark.png` files in
`frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots/`.

- [ ] **Step 3: Verify dark baselines pass**

```sh
cd frontend && npx playwright test --project=chromium-dark 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 4: Verify light baselines still pass**

```sh
cd frontend && npx playwright test --project=chromium 2>&1 | tail -10
```

Expected: all tests pass (existing baselines untouched).

- [ ] **Step 5: Check generated file counts**

```sh
ls frontend/tests/e2e/ui-parity.test.ts-snapshots/*chromium-dark* | wc -l
ls frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots/*chromium-dark* | wc -l
```

Expected: > 0 in each directory.

- [ ] **Step 6: Commit baselines**

```sh
git add \
  "frontend/tests/e2e/ui-parity.test.ts-snapshots/" \
  "frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots/"
git commit -m "test(baselines): generate dark theme parity snapshot baselines"
```

---

## Task 8: Update `surfaces.md`

**Files:**

- Modify: `docs/development/ui/surfaces.md`

Add dark PNG embeds alongside every existing light embed. Remove the open-gaps entry for
dark captures. Do both changes in one commit.

- [ ] **Step 1: Find all light snapshot embeds**

```sh
grep -n "chromium\.png" docs/development/ui/surfaces.md
```

Note every line returned — each is a light embed that needs a dark pair.

- [ ] **Step 2: Add paired dark embeds**

For every embed of the form:

```markdown
![description](../../../frontend/tests/e2e/ui-parity.test.ts-snapshots/ui-parity-SOMETHING-chromium.png)
```

Add immediately after:

```markdown
![description (dark)](../../../frontend/tests/e2e/ui-parity.test.ts-snapshots/ui-parity-SOMETHING-chromium-dark.png)
```

Apply the same pattern for any `ui-parity-responsive.test.ts-snapshots/` embeds.

- [ ] **Step 3: Remove the open-gaps entry**

Find and delete the block (around lines 263–266) starting with:

```markdown
- **Dark theme captures missing.**
```

Include any continuation lines for that bullet point.

- [ ] **Step 4: Verify markdownlint passes**

```sh
npx markdownlint --config .markdownlint.json docs/development/ui/surfaces.md
```

Expected: no errors.

- [ ] **Step 5: Commit**

```sh
git add docs/development/ui/surfaces.md
git commit -m "docs(surfaces): add dark theme parity embeds, close dark-captures open gap"
```
