# Dark Theme Parity Snapshots

**Date:** 2026-04-24
**Status:** Approved

## Problem

`surfaces.md` requires dark theme screenshot captures for all parity pairs. Both
`ui-parity.test.ts` and `ui-parity-responsive.test.ts` hardcode `colorScheme: 'light'`,
so dark captures don't exist and parity contract closure is blocked.

## Approach

Playwright project parameterization. Add a `chromium-dark` project alongside the
existing `chromium` project. The snapshot path template already includes `{projectName}`,
so light snapshots (`*-chromium.png`) are untouched and dark snapshots auto-land as
`*-chromium-dark.png`. No test logic is duplicated — dark coverage is structural,
not conventional.

## Files Changed

### `frontend/playwright.config.ts`

Add `colorScheme: 'light'` to the existing `chromium` project (makes it explicit).
Add `chromium-dark` project:

```ts
projects: [
  {
    name: 'chromium',
    use: { ...devices['Desktop Chrome'], colorScheme: 'light' }
  },
  {
    name: 'chromium-dark',
    use: { ...devices['Desktop Chrome'], colorScheme: 'dark' }
  }
]
```

### `frontend/tests/e2e/parity-config.ts`

Three changes:

1. **`assertProjectGuard`** — change the guard to use `startsWith` instead of strict
   equality so both `chromium` and `chromium-dark` pass. The exported
   `PARITY_REQUIRED_PROJECT` constant stays `'chromium'` (documents the base browser);
   the guard function switches to a prefix check:

   ```ts
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

2. **`assertDeterministicCaptureProfile` — add `prefersDark` to `page.evaluate`** —
   extend the existing env collection object to include the colorScheme state:

   ```ts
   const env = await page.evaluate(() => ({
     language: navigator.language,
     timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
     reducedMotion: window.matchMedia('(prefers-reduced-motion: reduce)').matches,
     devicePixelRatio: window.devicePixelRatio,
     prefersDark: window.matchMedia('(prefers-color-scheme: dark)').matches   // ADD
   }));
   ```

3. **`assertDeterministicCaptureProfile` — enforce colorScheme** — add mismatch check
   after the existing DPR check. `projectName` is scoped inside `assertProjectGuard`,
   not available here — read it again:

   ```ts
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

### `frontend/tests/e2e/parity-fixtures.ts` (new file)

Exports `parityTest` (a `test.extend`) with a `parityTheme: 'light' | 'dark'` fixture
derived from project name. The fixture receives `testInfo` as the third argument — the
correct Playwright API for reading project metadata inside `test.extend`. Also exports
`freezeParityInputs`, moved here from both test files.

The `localStorage` key `theme-mode` and valid values `'light' | 'dark' | 'system'` are
confirmed from `frontend/src/lib/theme.svelte.ts` (`STORAGE_KEY` constant, `ThemeMode`
type). Setting it to `'dark'` is read by the inline script in `app.html` on page load
(before any framework JS runs), which calls `applyTheme` and toggles
`document.documentElement.classList.toggle('dark', true)`. `initTheme()` only sets up
the system-preference media query listener — it does not re-apply the theme on load.
Combined with `emulateMedia({ colorScheme: 'dark' })`, this fully activates dark tokens.

```ts
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

### `frontend/tests/e2e/ui-parity.test.ts`

- Replace `import { expect, test } from '@playwright/test'` with
  `import { expect } from '@playwright/test'` and
  `import { parityTest as test, freezeParityInputs } from './parity-fixtures'`
- Delete the local `freezeParityInputs` function (lines 228–233 in current file)
- Remove `colorScheme: 'light'` from `test.use({...})` (now controlled by project config;
  `viewport`, `locale`, `timezoneId` stay)
- Update `beforeEach` to receive and pass `parityTheme`:

  ```ts
  test.beforeEach(async ({ page, parityTheme }) => {
    test.skip(!isCanonicalUiParityHost, canonicalUiParityReason);
    await freezeParityInputs(page, parityTheme);
  });
  ```

- **`governance: reject reduced-motion` test** — the test intentionally breaks
  `reducedMotion` to trigger the drift guard. Its `emulateMedia` call currently hardcodes
  `colorScheme: 'light'`, which would conflict with `chromium-dark`'s colorScheme
  enforcement. Remove `colorScheme` from that call — pass only `reducedMotion`:

  ```ts
  // Before (line 639):
  await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'no-preference' });

  // After:
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  ```

### `frontend/tests/e2e/ui-parity-responsive.test.ts`

- Same import swap as `ui-parity.test.ts` — `parityTest as test`, `freezeParityInputs`
  from `./parity-fixtures`
- Delete the local `freezeParityInputs` function (lines 143–148 in current file)
- `colorScheme` was never in `test.use()` here — only in the local `freezeParityInputs`
  — so no `test.use()` change needed
- Update `beforeEach` to receive and pass `parityTheme`:

  ```ts
  test.beforeEach(async ({ page, parityTheme }) => {
    test.skip(!isCanonicalUiParityHost, canonicalUiParityReason);
    await freezeParityInputs(page, parityTheme);
  });
  ```

### `docs/development/ui/surfaces.md`

The doc embeds `![...]()` references to light snapshot PNGs as the authoritative visual
record of each parity pair. Dark captures must be added alongside each light embed for
contract closure. For every existing `![...](../../../frontend/tests/e2e/ui-parity
.test.ts-snapshots/*-chromium.png)` and responsive equivalent, add a paired dark embed
referencing the `*-chromium-dark.png` counterpart.

Also update the open-gaps section (lines 263–266) to remove the "Dark theme captures
missing" entry. Do this in the same commit that adds the dark baseline PNGs and dark
image embeds — not in a separate cleanup commit.

## Snapshot Naming

Snapshot path template in `playwright.config.ts`:
`{testDir}/{testFilePath}-snapshots/{arg}-{projectName}{ext}`

```text
ui-parity.test.ts-snapshots/
  ui-parity-settings-tabs-chromium.png           (existing light)
  ui-parity-settings-tabs-chromium-dark.png      (new dark)

ui-parity-responsive.test.ts-snapshots/
  ui-parity-responsive-tablet-sidebar-overlay-chromium.png      (existing light)
  ui-parity-responsive-tablet-sidebar-overlay-chromium-dark.png (new dark)
```

Snapshots from each test file land in their own `*-snapshots/` subdirectory.
Existing light baselines (`*-chromium.png`) are untouched. Dark baselines are generated once via:

```sh
npx playwright test --project=chromium-dark --update-snapshots
```

Must run on macOS (canonical host guard). All screenshot-producing tests in both
`ui-parity.test.ts` and `ui-parity-responsive.test.ts` run under this command —
no `--grep` filter needed.

## Governance Tests

The governance tests (`enforce harness diff`, `reject non-allowlisted mask`,
`reject viewport drift`, `reject reduced-motion`) run under both projects. All are
stateless assertions that pass identically in both themes — except `reject
reduced-motion`, which is fixed above (remove hardcoded `colorScheme: 'light'`).

The `governance: mask budget uses union area` test produces a snapshot. Its dark variant
(`ui-parity-governance-mask-union-area-chromium-dark.png`) will differ slightly (CSS
background token color) and is acceptable.

## CI Impact

`workers: 1` in CI means both projects run sequentially. Dark suite adds approximately
the same wall-clock time as the existing light suite. No CI config changes required.

## Closure Condition

Parity closure (per `surfaces.md`) requires:

1. Paired dark/light captures committed for all required pairs — satisfied structurally
   by the `chromium-dark` project (any future `captureParityScreenshot` call
   automatically gets a dark capture)
2. Dark image embeds added to `surfaces.md` alongside existing light embeds
3. Open-gaps entry "Dark theme captures missing" removed from `surfaces.md`
