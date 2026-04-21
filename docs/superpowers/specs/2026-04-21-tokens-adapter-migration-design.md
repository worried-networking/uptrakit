# Tokens + Adapter Migration — Sub-spec #1

**Date:** 2026-04-21
**Status:** Draft
**Parent migration:** UI Design Language migration
(spec `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`,
guide `docs/development/ui-design-language.md`)

## Overview

The approved UI design language spec (`2026-04-16-ui-design-language-design.md`)
defines a semantic token contract. The current frontend at `frontend/src/app.css`
declares literal hex/rgba values inline and
`frontend/src/theme/adapter-manifest.json` maps semantic tokens to the Skeleton
palette, but the manifest is not runtime-enforced. App tokens drift from spec
values, and the manifest is decorative.

This sub-spec migrates token declarations to a typed TypeScript source of truth
that emits CSS via a Vite virtual module. It fixes the drift, establishes a
single source of truth, and enables later sub-specs (e.g., `TerminalOutput`
theme derivation) to consume tokens programmatically.

This is **sub-spec #1** of a five-part migration. The other sub-specs (shared
`Button` primitive, call-site migration, surface-layer parity, fixture
backfill) each get their own spec → plan → implementation cycle.

## Goals

1. Every semantic token defined in the approved spec §2.1 and §2.2 uses the
   exact value from the spec table, in both dark and light themes.
2. Built-in and surface-backed UI consume the same adapter layer as required by
   spec §2.8; the manifest is no longer decorative.
3. Token drift is structurally prevented: adding or changing a token goes
   through a typed module with a pinned test table.
4. Token values are programmatically accessible (`getToken(name, theme)`) so
   later sub-specs (terminal shell, primitives) can derive colors without
   hardcoding.

## Non-goals

- Changing spec values. This sub-spec corrects the runtime to match the spec,
  not the other way around.
- Refactoring Skeleton theme configuration. The existing Skeleton preset
  palette stays loaded; only semantic tokens move.
- Migrating any component call site. Button/input/badge migrations happen in
  sub-spec #3.
- Adding new tokens.
- Changing the theme switcher implementation or the `.dark` class strategy.
- Changing any parity fixture. Sub-spec #5 handles fixture backfill.

## Current state

- `frontend/src/app.css:11-85` declares `:root` and `.dark` blocks with literal
  hex and rgba values.
- `frontend/src/theme/adapter-manifest.json` maps each semantic token to a
  Skeleton palette step (`--color-success-400` etc.) but those mappings are not
  what drives runtime.
- `frontend/src/lib/theme/adapter-manifest.test.ts` asserts manifest
  completeness against the spec §2.1/2.2 token list.
- `frontend/src/lib/theme/design-token-values.test.ts` asserts computed values
  at runtime.
- Known value drift from spec:
  - Light `--color-success-border`: `.2` (spec `.3`)
  - Light `--color-warning-bg`: `.1` (spec `.08`)
  - Light `--color-warning-border`: `.22` (spec `.28`)
  - Light `--color-error-bg`: `.08` (spec `.07`)
  - Light `--color-error-border`: `.2` (spec `.3`)
  - Light `--text-inverted`: `#f8fafc` (spec `#ffffff`)
  - Dark `--color-success-bg`: `.14` (spec `.10`)
  - Dark `--color-success-border`: `.22` (spec `.25`)
  - Dark `--color-warning-bg`: `.14` (spec `.12`)
  - Dark `--color-warning-border`: `.24` (spec `.30`)
  - Dark `--color-error-bg`: uses text-color base `rgb(253,186,116)`
    (orange-300) with alpha `.14`; spec uses `rgb(234,88,12)` (orange-600)
    with alpha `.15`
  - Dark `--color-error-border`: same base drift as `-bg`; alpha `.22`
    (spec `.35`)
  - Dark `--text-inverted`: `#09090b` (spec `#fafafa`)
- Current `app.css` uses `--theme-accent*` and `--theme-info*` intermediary
  variables that alias the semantic `--accent*` and `--color-info*` tokens.
  `tokens.ts` emits semantic tokens directly and drops the intermediary layer.
  No external consumer references `--theme-accent*` / `--theme-info*` — only
  `--accent*` and `--color-info*` are used in components — so removal is safe.
- `frontend/src/app.css:122-123` `.skip-link` uses
  `var(--color-primary-500, #0070f3)` fallback and `color: #fff` — raw hex
  instead of semantic tokens.

## Architecture

Single source of truth: `frontend/src/theme/tokens.ts`.

A Vite plugin resolves a virtual CSS module that contains `:root` and `.dark`
declarations generated from `tokens.ts`. `frontend/src/app.css` imports the
virtual module at build time. No runtime JS cost. No literal values in
`app.css`. No JSON manifest.

```text
tokens.ts                              ← canonical spec values, typed
    ↓ (Vite plugin: theme-tokens)
virtual:theme/tokens.css               ← generated :root + .dark blocks
    ↓ @import
app.css                                ← imports virtual module; keeps global rules
    ↓
Runtime DOM                            ← browser applies :root + .dark as today
```

Skeleton preset palette is not removed. Skeleton components continue to use
`--color-success-500`, `--color-surface-*`, etc. Semantic `--bg-*`, `--text-*`,
`--accent*`, and `--color-{success,warning,error,info}-*` tokens are owned by
`tokens.ts` and no longer by `app.css` literal blocks.

Cascade precedence: `@import 'virtual:theme/tokens.css'` is placed AFTER the
Skeleton theme imports in `app.css`. Where a semantic token name collides with
a Skeleton palette step, the virtual module wins because it is declared later
in the same scope. Skeleton's numbered palette (`--color-*-500` etc.) is
untouched.

## Components

### `frontend/src/theme/tokens.ts` (new)

Canonical module. Exports a typed token table plus helpers.

Shape:

```ts
export type Theme = 'dark' | 'light';

export type TokenName =
  | '--bg-base' | '--bg-surface' | '--bg-raised'
  | '--border-subtle' | '--border-default'
  | '--text-primary' | '--text-secondary'
  | '--text-muted' | '--text-inverted'
  | '--accent' | '--accent-rgb'
  | '--accent-bright' | '--accent-dark' | '--accent-deep'
  | '--color-success' | '--color-success-bg' | '--color-success-border'
  | '--color-warning' | '--color-warning-bg' | '--color-warning-border'
  | '--color-error'   | '--color-error-bg'   | '--color-error-border'
  | '--color-info'    | '--color-info-bg'    | '--color-info-border';

export type TokenValue = string;

export const tokens: Record<TokenName, Record<Theme, TokenValue>>;

/** Emit `  --name: value;` lines for one theme block. */
export function cssForTheme(theme: Theme): string;

/** Lookup helper for programmatic consumers (terminal shell, xterm theme). */
export function getToken(name: TokenName, theme: Theme): TokenValue;
```

Opacity math lives inside the module via a small `rgba(base, alpha)` helper so
spec bases + alphas stay readable:

```ts
const successBase = { dark: '74 222 128', light: '22 163 74' };
// '--color-success-border' dark: rgba(successBase.dark, 0.25)
// '--color-success-border' light: rgba(successBase.light, 0.30)
```

Values pinned to the spec tables, §2.1 and §2.2. Inverted tokens carry
spec-correct values (dark `#fafafa`, light `#ffffff`).

### `frontend/vite-plugins/theme-tokens.ts` (new)

The `frontend/vite-plugins/` directory does not currently exist and is created
by this sub-spec.

Vite plugin with virtual module id `virtual:theme/tokens.css`.

- `resolveId` hook returns the virtual id for an exact string match.
- `load` hook imports `tokens.ts` and concatenates:

  ```css
  :root {
    color-scheme: light;
    <cssForTheme('light')>
  }
  .dark {
    color-scheme: dark;
    <cssForTheme('dark')>
  }
  ```

- `handleHotUpdate({ file, server })` hook: when `file` resolves to
  `frontend/src/theme/tokens.ts`, the plugin calls
  `server.moduleGraph.invalidateModule(virtualModule)` and returns the virtual
  module in the affected-modules array so Vite streams the updated CSS chunk
  to the browser without a full reload. `load()` is a sync string return — no
  async filesystem work needed.
- Registered in `frontend/vite.config.ts` alongside existing plugins.

### `frontend/src/app.css` (modified)

- `:root { ... }` and `.dark { ... }` blocks deleted.
- New line near the top (after existing Tailwind/Skeleton imports):

  ```css
  @import 'virtual:theme/tokens.css';
  ```

- `.skip-link` updated: `background: var(--accent);` and
  `color: var(--text-inverted);`.
- Global rules preserved: `@custom-variant dark`, `@plugin
  '@tailwindcss/forms'`, Skeleton imports, `cerberus` theme, input padding,
  transition triplet, focus-visible box-shadow, error focus border, disabled
  opacity, z-index layer selectors.

### `frontend/src/theme/tokens.test.ts` (new)

Replaces the role previously filled by `lib/theme/adapter-manifest.test.ts` for
completeness; keeps runtime value parity checks.

Assertions:

- Every `TokenName` is defined for both `'dark'` and `'light'`.
- Each `(name, theme)` pair matches a frozen spec-value table copied verbatim
  from spec §2.1/§2.2. The table lives in the test file; spec drift forces both
  the spec and the test to update together.
- `--accent-rgb` parses as three integers in `0..255` separated by single
  spaces.
- `rgba(base, alpha)` helper round-trips: `rgba('74 222 128', 0.10)` produces
  `rgba(74, 222, 128, 0.1)` (or whatever canonical form the helper emits).
- `cssForTheme('light')` emits every token exactly once.

### `frontend/vite-plugins/theme-tokens.test.ts` (new)

- `resolveId('virtual:theme/tokens.css')` returns the virtual id.
- `load(virtualId)` returns a string containing `:root {` and `.dark {`.
- Emitted CSS declares every `TokenName` twice (one per theme).
- `handleHotUpdate` invalidates the virtual module when `tokens.ts` is touched.

### `frontend/src/lib/theme/design-token-values.test.ts` (updated)

Current test reads `app.css` via `node:fs` and greps literal rgba/hex strings
per token. Post-migration the literal blocks no longer exist, so the test is
rewritten as follows:

- Drop `node:fs` + `app.css` read.
- Import `tokens` and `cssForTheme` from `../../theme/tokens` (relative from
  `frontend/src/lib/theme/`). Vitest resolves TS under Vite's default resolver
  — no config change required.
- Assert each `(TokenName, Theme)` pair equals the spec-pinned string.
- Snapshot `cssForTheme('light')` and `cssForTheme('dark')` outputs so any
  future format change (whitespace, declaration order) is flagged.

No Playwright or DOM rendering involved; the test is pure-unit.

### `frontend/src/lib/theme/adapter-manifest.test.ts` (renamed, split)

File is renamed to `frontend/src/lib/theme/css-contract.test.ts`. The two
manifest-specific `describe` blocks are deleted. The structural assertions
unrelated to the JSON manifest stay and continue to guard `app.css`:

- Z-index layer contract (`data-ui` selectors → spec §2.7 values).
- Global transition triplet (`background`, `border-color`, `color` only).
- `:focus-visible` box-shadow rule (`0 0 0 3px rgba(var(--accent-rgb), 0.25)`).
- Error-state focus border (`border-color: var(--color-error-border)` on
  `[aria-invalid='true']`).

These assertions read `app.css` via `node:fs` today and continue to do so
after PR2; the literal `:root`/`.dark` blocks being deleted does not affect
them because they match structural selectors lower in the file.

Manifest completeness + value-pinning checks move to `tokens.test.ts`. The TS
`Record<TokenName, ...>` type catches missing tokens at compile time; the new
test repeats the check at runtime.

### `frontend/src/theme/adapter-manifest.json` (removed)

Decorative manifest deleted. `tokens.ts` is the contract.

## Data flow

### Build time

1. Vite reads `frontend/src/app.css`.
2. `app.css` declaration `@import 'virtual:theme/tokens.css'` triggers the
   `theme-tokens` plugin's `resolveId` hook.
3. Plugin `load` hook imports `tokens.ts`, calls `cssForTheme('light')` and
   `cssForTheme('dark')`, concatenates them under `:root` and `.dark`
   selectors.
4. Emitted CSS string is returned to Vite and inlined into the final
   stylesheet bundle.

### Runtime

- Browser loads the bundled CSS; `:root` custom properties apply under the
  default light theme.
- The `.dark` class on `<html>` (existing theme switcher behavior) overrides
  with dark values.
- No JavaScript reads or writes tokens at runtime under normal operation.
- `getToken(name, theme)` remains available for programmatic consumers that
  cannot rely on CSS cascade (e.g., xterm.js in sub-spec #2).

### HMR

- Editing `tokens.ts` invalidates the virtual module.
- Vite pushes an updated stylesheet chunk.
- Browser re-applies the custom properties live without a full reload.

### Test time

- `vitest` imports `tokens.ts` directly and asserts against the spec-pinned
  table.
- TypeScript compilation enforces completeness across the `TokenName` union.

## Error handling

Build-time:

- Adding a new `TokenName` to the union without a value in `tokens` → TS
  compile error; Vite build fails.
- Omitting `'dark'` or `'light'` for any token → TS compile error.
- Malformed rgba/hex emitted by the helper → Vite CSS parser error surfaces in
  dev or build output.

Test-time:

- Spec drift: spec table in `tokens.test.ts` differs from `tokens.ts` values →
  vitest assertion fails with a diff highlighting the drifting token.
- `--accent-rgb` malformed → vitest fails before any Playwright run.

Runtime:

- No runtime JS path exists for token resolution under normal operation, so no
  runtime errors are possible.
- Skip-link fallback removed; `var(--accent)` and `var(--text-inverted)` always
  resolve because the virtual module declares them.

Migration:

- The PR that introduces `tokens.ts` plus the Vite plugin keeps the literal
  `:root`/`.dark` blocks in `app.css` temporarily (transitional guard).
- A follow-up commit in the same PR deletes the literal blocks after verifying
  the virtual module emits expected CSS. If visual regression fails at that
  step, the literal block is restored and the virtual import removed to roll
  back.
- Rollback path stays trivial: revert the `app.css` import line and restore
  literal blocks from git history.

## Testing

Unit (`vitest`):

- `theme/tokens.test.ts`:
  - Every `TokenName × Theme` present (TS enforces; test asserts at runtime
    for paranoia).
  - Spec table pinned: each token value equals the frozen expected string from
    spec §2.1/§2.2.
  - `--accent-rgb` matches `r g b` format with three `0..255` ints.
  - `rgba(base, alpha)` helper round-trips.
  - `cssForTheme('light')` emits every token exactly once.
- `vite-plugins/theme-tokens.test.ts`:
  - `load('virtual:theme/tokens.css')` returns a string that includes
    `:root {` and `.dark {`.
  - Emitted CSS declares every `TokenName` twice (one per theme).
  - HMR hook invalidates the virtual module when `tokens.ts` changes.

Integration:

- `design-token-values.test.ts` updated to import from `tokens.ts` so the test
  runs against the emitted values and catches virtual-module regressions
  end-to-end.
- Playwright smoke: `/login` and `/` (authenticated home) rendered in light +
  dark, `getComputedStyle(document.documentElement).getPropertyValue(
  '--color-success-border')` matches the spec value for each theme.

Visual regression (spec §3 parity gates):

- No new fixtures in this sub-spec; sub-spec #5 handles the fixture matrix.
- Existing paired snapshots are re-run; diffs up to `0.5%` are tolerated per
  spec §3.
- Known expected deltas from the value corrections (must pass):
  - Light success/error borders slightly more saturated (opacity `.2 → .3`).
  - Dark success background slightly lighter (`.14 → .10`).
  - Dark error background and border shift from orange-300 base to orange-600
    base.
  - `--text-inverted` visible only in small badge/accent-fill cases; diff
    expected minimal.
- Any snapshot that exceeds `0.5%` after value correction is flagged in the
  implementation plan for follow-up review.

CI gates added:

- `vitest` fails on drift.
- Grep lint: no raw `#[0-9a-fA-F]{3,8}` or `rgba?\(` outside
  `frontend/src/theme/` (xterm-related exceptions scoped to sub-spec #2 and
  called out there).

## Dependencies and sequencing

- Depends on: nothing upstream. This is the first sub-spec.
- Unblocks: sub-spec #2 (shared `Button` primitive + terminal theme
  derivation) consumes `getToken()` to source xterm colors from semantic
  tokens.
- Independent of: sub-specs #3 (call-site migration), #4 (surface parity),
  #5 (fixture backfill). They proceed once `tokens.ts` ships.

## Rollout

1. PR 1 — add `tokens.ts`, Vite plugin, `tokens.test.ts`, and
   `theme-tokens.test.ts`. Keep literal `:root`/`.dark` blocks in `app.css`;
   add `@import 'virtual:theme/tokens.css'` alongside so the virtual module
   overrides the literals in cascade order. A dedicated vitest case compares
   the virtual module output to a frozen golden CSS string (spec-pinned)
   mechanically — no manual diff.
2. PR 2 — delete literal `:root`/`.dark` blocks in `app.css`; delete the
   `--theme-accent*` and `--theme-info*` intermediary variables; delete
   `adapter-manifest.json`; rename `adapter-manifest.test.ts` →
   `css-contract.test.ts` per the components section; rewrite
   `design-token-values.test.ts` per the components section; fix skip-link to
   use `var(--accent)` + `var(--text-inverted)`. Run full parity snapshot
   pass.
3. If parity snapshots exceed `0.5%` on any pair, investigate per-token delta
   and either accept the corrected spec value (update fixtures) or file a
   waiver per spec §9.

## Open questions

None.
