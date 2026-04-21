# Tokens + Adapter Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace literal `:root`/`.dark` CSS blocks in `frontend/src/app.css`
and the decorative `adapter-manifest.json` with a typed TypeScript source of
truth (`tokens.ts`) that emits CSS via a Vite virtual module, fixing all token
drift from spec §2.1/§2.2.

**Architecture:** `tokens.ts` exposes a `Record<TokenName, Record<Theme,
TokenValue>>` table plus `cssForTheme()` and `getToken()` helpers. A Vite
plugin resolves `virtual:theme/tokens.css`, calling `cssForTheme()` to emit
`:root`/`.dark` blocks. `app.css` `@import`s the virtual module. No runtime JS
cost, no literal values in `app.css`.

**Tech Stack:** TypeScript, Vite, vitest, SvelteKit frontend.

**Spec:** `docs/superpowers/specs/2026-04-21-tokens-adapter-migration-design.md`

**Rollout structure:**

- **PR 1 — Infrastructure only.** Add `tokens.ts`, Vite plugin, unit tests,
  golden CSS snapshot. Do NOT wire `app.css` yet.
  - **Divergence from spec rollout §1:** The spec proposes adding
    `@import 'virtual:theme/tokens.css'` alongside the literal blocks in PR1
    with the virtual module overriding via cascade. CSS `@import` must
    precede all other rules, so the imported content appears BEFORE the
    literal `:root`/`.dark` blocks, and the literals win the cascade. The
    `@layer` workaround is more surface area than necessary for a
    transitional commit. Instead, ship the infrastructure standalone in PR1,
    verified by the golden CSS vitest case, and wire+cleanup atomically in
    PR2.
- **PR 2 — Switch + cleanup.** Add `@import`, delete literal blocks + the
  `--theme-accent*`/`--theme-info*` intermediaries, fix `.skip-link`, rename
  `adapter-manifest.test.ts` → `css-contract.test.ts`, rewrite
  `design-token-values.test.ts`, delete `adapter-manifest.json`.

All frontend quality gates run from `frontend/`:

```bash
cd frontend
npm run lint
npm run format:check
npm run check
npm run test
npm run build
```

---

## File Structure

**New files (PR1):**

- `frontend/src/theme/tokens.ts` — canonical typed token table plus
  `cssForTheme`, `getToken`, internal `rgba` helper. One responsibility:
  spec-pinned values and emission helpers.
- `frontend/src/theme/tokens.test.ts` — unit tests for `tokens.ts`. Lives
  next to the module under test; pure unit.
- `frontend/vite-plugins/theme-tokens.ts` — Vite plugin that resolves the
  virtual module id and emits generated CSS by calling `cssForTheme`.
- `frontend/vite-plugins/theme-tokens.test.ts` — unit tests for the plugin.

**New directory (PR1):** `frontend/vite-plugins/`.

**Modified files (PR1):**

- `frontend/vite.config.ts` — register the plugin.

**Modified files (PR2):**

- `frontend/src/app.css` — delete literal `:root`/`.dark` blocks, delete
  `--theme-accent*`/`--theme-info*` intermediaries, add `@import
  'virtual:theme/tokens.css'`, update `.skip-link`.
- `frontend/src/lib/theme/design-token-values.test.ts` — rewrite to import
  from `tokens.ts` instead of reading `app.css`.

**Renamed files (PR2):**

- `frontend/src/lib/theme/adapter-manifest.test.ts` →
  `frontend/src/lib/theme/css-contract.test.ts` with the two manifest-specific
  `describe` blocks removed.

**Deleted files (PR2):**

- `frontend/src/theme/adapter-manifest.json`.

---

## Reference: Spec-Pinned Token Values

Copied verbatim from parent spec §2.1 (dark) and §2.2 (light). These values
are authoritative; any divergence in `tokens.ts` is a bug.

### Dark (§2.1)

| Token                    | Value                  |
| ------------------------ | ---------------------- |
| `--bg-base`              | `#09090b`              |
| `--bg-surface`           | `#111113`              |
| `--bg-raised`            | `#18181b`              |
| `--border-subtle`        | `#1c1c1f`              |
| `--border-default`       | `#27272a`              |
| `--text-muted`           | `#52525b`              |
| `--text-secondary`       | `#a1a1aa`              |
| `--text-primary`         | `#e4e4e7`              |
| `--text-inverted`        | `#fafafa`              |
| `--accent`               | `#06b6d4`              |
| `--accent-rgb`           | `6 182 212`            |
| `--accent-bright`        | `#22d3ee`              |
| `--accent-dark`          | `#0891b2`              |
| `--accent-deep`          | `#0e7490`              |
| `--color-success`        | `#4ade80`              |
| `--color-success-bg`     | `rgba(74,222,128,.10)` |
| `--color-success-border` | `rgba(74,222,128,.25)` |
| `--color-warning`        | `#fbbf24`              |
| `--color-warning-bg`     | `rgba(251,191,36,.12)` |
| `--color-warning-border` | `rgba(251,191,36,.3)`  |
| `--color-error`          | `#fdba74`              |
| `--color-error-bg`       | `rgba(234,88,12,.15)`  |
| `--color-error-border`   | `rgba(234,88,12,.35)`  |
| `--color-info`           | `#67e8f9`              |
| `--color-info-bg`        | `rgba(6,182,212,.10)`  |
| `--color-info-border`    | `rgba(6,182,212,.22)`  |

### Light (§2.2)

| Token                    | Value                 |
| ------------------------ | --------------------- |
| `--bg-base`              | `#f8fafc`             |
| `--bg-surface`           | `#ffffff`             |
| `--bg-raised`            | `#f1f5f9`             |
| `--border-subtle`        | `#e2e8f0`             |
| `--border-default`       | `#cbd5e1`             |
| `--text-muted`           | `#94a3b8`             |
| `--text-secondary`       | `#64748b`             |
| `--text-primary`         | `#0f172a`             |
| `--text-inverted`        | `#ffffff`             |
| `--accent`               | `#2563eb`             |
| `--accent-rgb`           | `37 99 235`           |
| `--accent-bright`        | `#3b82f6`             |
| `--accent-dark`          | `#1d4ed8`             |
| `--accent-deep`          | `#1e40af`             |
| `--color-success`        | `#16a34a`             |
| `--color-success-bg`     | `rgba(22,163,74,.08)` |
| `--color-success-border` | `rgba(22,163,74,.3)`  |
| `--color-warning`        | `#d97706`             |
| `--color-warning-bg`     | `rgba(217,119,6,.08)` |
| `--color-warning-border` | `rgba(217,119,6,.28)` |
| `--color-error`          | `#dc2626`             |
| `--color-error-bg`       | `rgba(220,38,38,.07)` |
| `--color-error-border`   | `rgba(220,38,38,.3)`  |
| `--color-info`           | `#0891b2`             |
| `--color-info-bg`        | `rgba(8,145,178,.08)` |
| `--color-info-border`    | `rgba(8,145,178,.22)` |

Canonical `rgba` emission form for this plan: `rgba(R, G, B, A)` with spaces
after commas, `A` rendered as the shortest decimal with a leading zero when
`< 1` (so `0.1`, `0.25`, `0.3`). All tests and golden strings use this form.

The spec tables above use the CSS dot-prefix shorthand (`.10`, `.3`, etc.)
and omit whitespace inside `rgba(...)`. This plan's canonical form is
**CSS-equivalent** to the spec form — the browser parses `rgba(74,222,128,.10)`
and `rgba(74, 222, 128, 0.1)` to the same computed value. The plan form is
chosen because it is what `String(Number(x.toFixed(3)))` emits from JS and
what Prettier formats; that lets the `rgba()` helper, the test `EXPECTED`
tables, the golden CSS string, and the inline snapshots all agree on a
single textual form without ceremony. The visual regression gate
(`getComputedStyle`) is the ultimate source of truth — both textual forms
produce the same pixels.

---

## PR 1 — Infrastructure

### Task 1: Create `tokens.ts` with failing test first

**Files:**

- Create: `frontend/src/theme/tokens.ts`
- Create: `frontend/src/theme/tokens.test.ts`

- [ ] **Step 1: Write the failing test**

Create `frontend/src/theme/tokens.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { tokens, cssForTheme, getToken, type TokenName, type Theme } from './tokens';

const EXPECTED: Record<TokenName, Record<Theme, string>> = {
  '--bg-base': { dark: '#09090b', light: '#f8fafc' },
  '--bg-surface': { dark: '#111113', light: '#ffffff' },
  '--bg-raised': { dark: '#18181b', light: '#f1f5f9' },
  '--border-subtle': { dark: '#1c1c1f', light: '#e2e8f0' },
  '--border-default': { dark: '#27272a', light: '#cbd5e1' },
  '--text-muted': { dark: '#52525b', light: '#94a3b8' },
  '--text-secondary': { dark: '#a1a1aa', light: '#64748b' },
  '--text-primary': { dark: '#e4e4e7', light: '#0f172a' },
  '--text-inverted': { dark: '#fafafa', light: '#ffffff' },
  '--accent': { dark: '#06b6d4', light: '#2563eb' },
  '--accent-rgb': { dark: '6 182 212', light: '37 99 235' },
  '--accent-bright': { dark: '#22d3ee', light: '#3b82f6' },
  '--accent-dark': { dark: '#0891b2', light: '#1d4ed8' },
  '--accent-deep': { dark: '#0e7490', light: '#1e40af' },
  '--color-success': { dark: '#4ade80', light: '#16a34a' },
  '--color-success-bg': {
    dark: 'rgba(74, 222, 128, 0.1)',
    light: 'rgba(22, 163, 74, 0.08)'
  },
  '--color-success-border': {
    dark: 'rgba(74, 222, 128, 0.25)',
    light: 'rgba(22, 163, 74, 0.3)'
  },
  '--color-warning': { dark: '#fbbf24', light: '#d97706' },
  '--color-warning-bg': {
    dark: 'rgba(251, 191, 36, 0.12)',
    light: 'rgba(217, 119, 6, 0.08)'
  },
  '--color-warning-border': {
    dark: 'rgba(251, 191, 36, 0.3)',
    light: 'rgba(217, 119, 6, 0.28)'
  },
  '--color-error': { dark: '#fdba74', light: '#dc2626' },
  '--color-error-bg': {
    dark: 'rgba(234, 88, 12, 0.15)',
    light: 'rgba(220, 38, 38, 0.07)'
  },
  '--color-error-border': {
    dark: 'rgba(234, 88, 12, 0.35)',
    light: 'rgba(220, 38, 38, 0.3)'
  },
  '--color-info': { dark: '#67e8f9', light: '#0891b2' },
  '--color-info-bg': {
    dark: 'rgba(6, 182, 212, 0.1)',
    light: 'rgba(8, 145, 178, 0.08)'
  },
  '--color-info-border': {
    dark: 'rgba(6, 182, 212, 0.22)',
    light: 'rgba(8, 145, 178, 0.22)'
  }
};

const EXPECTED_TOKEN_NAMES = Object.keys(EXPECTED) as TokenName[];

describe('tokens', () => {
  it('defines every TokenName for both dark and light themes', () => {
    for (const name of EXPECTED_TOKEN_NAMES) {
      expect(tokens[name], `missing entry for ${name}`).toBeDefined();
      expect(tokens[name].dark, `missing dark value for ${name}`).toBeTruthy();
      expect(tokens[name].light, `missing light value for ${name}`).toBeTruthy();
    }
  });

  it('pins every (name, theme) pair to the spec-approved value', () => {
    for (const name of EXPECTED_TOKEN_NAMES) {
      expect(tokens[name].dark, `dark ${name}`).toBe(EXPECTED[name].dark);
      expect(tokens[name].light, `light ${name}`).toBe(EXPECTED[name].light);
    }
  });

  it('exposes getToken as a lookup helper equivalent to the table', () => {
    for (const name of EXPECTED_TOKEN_NAMES) {
      expect(getToken(name, 'dark')).toBe(EXPECTED[name].dark);
      expect(getToken(name, 'light')).toBe(EXPECTED[name].light);
    }
  });

  it('accent-rgb values parse as three integers in 0..255 separated by single spaces', () => {
    for (const theme of ['dark', 'light'] as const) {
      const parts = tokens['--accent-rgb'][theme].split(' ');
      expect(parts).toHaveLength(3);
      for (const part of parts) {
        const n = Number(part);
        expect(Number.isInteger(n)).toBe(true);
        expect(n).toBeGreaterThanOrEqual(0);
        expect(n).toBeLessThanOrEqual(255);
      }
    }
  });

  it('cssForTheme emits every TokenName exactly once for the given theme', () => {
    for (const theme of ['dark', 'light'] as const) {
      const css = cssForTheme(theme);
      for (const name of EXPECTED_TOKEN_NAMES) {
        const occurrences = css.split(`${name}:`).length - 1;
        expect(occurrences, `${name} in ${theme} css`).toBe(1);
      }
    }
  });

  it('cssForTheme emits each pair as `  --name: value;` on its own line', () => {
    const css = cssForTheme('dark');
    for (const name of EXPECTED_TOKEN_NAMES) {
      expect(css).toContain(`  ${name}: ${EXPECTED[name].dark};`);
    }
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd frontend && npm run test -- src/theme/tokens.test.ts`
Expected: FAIL — `Cannot find module './tokens'` or similar.

- [ ] **Step 3: Write minimal implementation**

Create `frontend/src/theme/tokens.ts`:

```ts
export type Theme = 'dark' | 'light';

export type TokenName =
  | '--bg-base'
  | '--bg-surface'
  | '--bg-raised'
  | '--border-subtle'
  | '--border-default'
  | '--text-muted'
  | '--text-secondary'
  | '--text-primary'
  | '--text-inverted'
  | '--accent'
  | '--accent-rgb'
  | '--accent-bright'
  | '--accent-dark'
  | '--accent-deep'
  | '--color-success'
  | '--color-success-bg'
  | '--color-success-border'
  | '--color-warning'
  | '--color-warning-bg'
  | '--color-warning-border'
  | '--color-error'
  | '--color-error-bg'
  | '--color-error-border'
  | '--color-info'
  | '--color-info-bg'
  | '--color-info-border';

export type TokenValue = string;

/** Emit `rgba(R, G, B, A)` from a space-separated RGB base and an alpha. */
function rgba(base: string, alpha: number): TokenValue {
  const [r, g, b] = base.split(' ');
  // Strip trailing zeros so `0.10` → `0.1` and `0.30` → `0.3`.
  const a = String(Number(alpha.toFixed(3)));
  return `rgba(${r}, ${g}, ${b}, ${a})`;
}

const successBase = { dark: '74 222 128', light: '22 163 74' };
const warningBase = { dark: '251 191 36', light: '217 119 6' };
const errorBase = { dark: '234 88 12', light: '220 38 38' };
const infoBase = { dark: '6 182 212', light: '8 145 178' };

export const tokens: Record<TokenName, Record<Theme, TokenValue>> = {
  '--bg-base': { dark: '#09090b', light: '#f8fafc' },
  '--bg-surface': { dark: '#111113', light: '#ffffff' },
  '--bg-raised': { dark: '#18181b', light: '#f1f5f9' },
  '--border-subtle': { dark: '#1c1c1f', light: '#e2e8f0' },
  '--border-default': { dark: '#27272a', light: '#cbd5e1' },
  '--text-muted': { dark: '#52525b', light: '#94a3b8' },
  '--text-secondary': { dark: '#a1a1aa', light: '#64748b' },
  '--text-primary': { dark: '#e4e4e7', light: '#0f172a' },
  '--text-inverted': { dark: '#fafafa', light: '#ffffff' },
  '--accent': { dark: '#06b6d4', light: '#2563eb' },
  '--accent-rgb': { dark: '6 182 212', light: '37 99 235' },
  '--accent-bright': { dark: '#22d3ee', light: '#3b82f6' },
  '--accent-dark': { dark: '#0891b2', light: '#1d4ed8' },
  '--accent-deep': { dark: '#0e7490', light: '#1e40af' },
  '--color-success': { dark: '#4ade80', light: '#16a34a' },
  '--color-success-bg': {
    dark: rgba(successBase.dark, 0.1),
    light: rgba(successBase.light, 0.08)
  },
  '--color-success-border': {
    dark: rgba(successBase.dark, 0.25),
    light: rgba(successBase.light, 0.3)
  },
  '--color-warning': { dark: '#fbbf24', light: '#d97706' },
  '--color-warning-bg': {
    dark: rgba(warningBase.dark, 0.12),
    light: rgba(warningBase.light, 0.08)
  },
  '--color-warning-border': {
    dark: rgba(warningBase.dark, 0.3),
    light: rgba(warningBase.light, 0.28)
  },
  '--color-error': { dark: '#fdba74', light: '#dc2626' },
  '--color-error-bg': {
    dark: rgba(errorBase.dark, 0.15),
    light: rgba(errorBase.light, 0.07)
  },
  '--color-error-border': {
    dark: rgba(errorBase.dark, 0.35),
    light: rgba(errorBase.light, 0.3)
  },
  '--color-info': { dark: '#67e8f9', light: '#0891b2' },
  '--color-info-bg': {
    dark: rgba(infoBase.dark, 0.1),
    light: rgba(infoBase.light, 0.08)
  },
  '--color-info-border': {
    dark: rgba(infoBase.dark, 0.22),
    light: rgba(infoBase.light, 0.22)
  }
};

const TOKEN_NAMES = Object.keys(tokens) as TokenName[];

/** Emit `  --name: value;` lines for one theme block. */
export function cssForTheme(theme: Theme): string {
  return TOKEN_NAMES.map((name) => `  ${name}: ${tokens[name][theme]};`).join('\n');
}

/** Lookup helper for programmatic consumers (terminal shell, xterm theme). */
export function getToken(name: TokenName, theme: Theme): TokenValue {
  return tokens[name][theme];
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd frontend && npm run test -- src/theme/tokens.test.ts`
Expected: PASS, 6 tests green.

- [ ] **Step 5: Run typecheck + lint + format**

```bash
cd frontend
npm run check
npm run lint
npm run format
```

Expected: no errors from the new files. `npm run format` may rewrite lines;
re-run `npm run lint` and `npm run check` after if it does.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/theme/tokens.ts frontend/src/theme/tokens.test.ts
git commit -m "feat(frontend): add typed tokens module (sub-spec #1)"
```

---

### Task 2: Create Vite plugin `theme-tokens.ts` with failing test first

**Files:**

- Create: `frontend/vite-plugins/theme-tokens.ts`
- Create: `frontend/vite-plugins/theme-tokens.test.ts`

The `frontend/vite-plugins/` directory does not yet exist; the first file
creation will establish it.

- [ ] **Step 1: Write the failing test**

Create `frontend/vite-plugins/theme-tokens.test.ts`:

```ts
import { describe, expect, it, vi } from 'vitest';
import { themeTokensPlugin, VIRTUAL_ID } from './theme-tokens';
import { tokens } from '../src/theme/tokens';

type VitePluginHook<K extends keyof ReturnType<typeof themeTokensPlugin>> =
  ReturnType<typeof themeTokensPlugin>[K];

function callResolveId(id: string): string | undefined {
  const plugin = themeTokensPlugin();
  const resolveId = plugin.resolveId as VitePluginHook<'resolveId'>;
  if (typeof resolveId !== 'function') return undefined;
  const result = resolveId.call({} as never, id, undefined, {} as never);
  return typeof result === 'string' ? result : undefined;
}

function callLoad(id: string): string | undefined {
  const plugin = themeTokensPlugin();
  const load = plugin.load as VitePluginHook<'load'>;
  if (typeof load !== 'function') return undefined;
  const result = load.call({} as never, id, undefined);
  return typeof result === 'string' ? result : undefined;
}

describe('theme-tokens Vite plugin', () => {
  it('exposes the canonical virtual id constant', () => {
    expect(VIRTUAL_ID).toBe('virtual:theme/tokens.css');
  });

  it('resolveId returns the resolved id for the virtual module', () => {
    expect(callResolveId(VIRTUAL_ID)).toBe('\0' + VIRTUAL_ID);
  });

  it('resolveId returns undefined for unrelated ids', () => {
    expect(callResolveId('some-other-module')).toBeUndefined();
    expect(callResolveId('virtual:theme/other.css')).toBeUndefined();
  });

  it('load returns undefined for unrelated ids', () => {
    expect(callLoad('\0virtual:theme/other.css')).toBeUndefined();
  });

  it('load emits :root and .dark blocks for the resolved virtual id', () => {
    const css = callLoad('\0' + VIRTUAL_ID);
    expect(css).toBeDefined();
    expect(css).toContain(':root {');
    expect(css).toContain('color-scheme: light;');
    expect(css).toContain('.dark {');
    expect(css).toContain('color-scheme: dark;');
  });

  it('load declares every TokenName twice (once per theme)', () => {
    const css = callLoad('\0' + VIRTUAL_ID)!;
    for (const name of Object.keys(tokens)) {
      const occurrences = css.split(`${name}:`).length - 1;
      expect(occurrences, `${name} declaration count`).toBe(2);
    }
  });

  it('handleHotUpdate invalidates the virtual module when tokens.ts changes', () => {
    const plugin = themeTokensPlugin();
    const invalidateModule = vi.fn();
    const virtualModule = { id: '\0' + VIRTUAL_ID };
    const server = {
      moduleGraph: {
        getModuleById: vi.fn().mockReturnValue(virtualModule),
        invalidateModule
      }
    };

    const handleHotUpdate = plugin.handleHotUpdate as VitePluginHook<'handleHotUpdate'>;
    if (typeof handleHotUpdate !== 'function') {
      throw new Error('handleHotUpdate hook missing');
    }

    const ctx = {
      file: '/abs/path/to/frontend/src/theme/tokens.ts',
      server,
      modules: [],
      read: async () => '',
      timestamp: Date.now()
    } as never;
    const result = handleHotUpdate.call({} as never, ctx);

    expect(invalidateModule).toHaveBeenCalledWith(virtualModule);
    expect(Array.isArray(result) ? result : [result]).toContain(virtualModule);
  });

  it('handleHotUpdate ignores unrelated file changes', () => {
    const plugin = themeTokensPlugin();
    const invalidateModule = vi.fn();
    const server = {
      moduleGraph: {
        getModuleById: vi.fn(),
        invalidateModule
      }
    };

    const handleHotUpdate = plugin.handleHotUpdate as VitePluginHook<'handleHotUpdate'>;
    if (typeof handleHotUpdate !== 'function') {
      throw new Error('handleHotUpdate hook missing');
    }

    const ctx = {
      file: '/abs/path/to/frontend/src/routes/+page.svelte',
      server,
      modules: [],
      read: async () => '',
      timestamp: Date.now()
    } as never;
    handleHotUpdate.call({} as never, ctx);

    expect(invalidateModule).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd frontend && npm run test -- vite-plugins/theme-tokens.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Write minimal implementation**

Create `frontend/vite-plugins/theme-tokens.ts`:

```ts
import type { Plugin } from 'vite';
import { cssForTheme } from '../src/theme/tokens';

export const VIRTUAL_ID = 'virtual:theme/tokens.css';
const RESOLVED_VIRTUAL_ID = '\0' + VIRTUAL_ID;
const TOKENS_SOURCE_SUFFIX = 'src/theme/tokens.ts';

export function themeTokensPlugin(): Plugin {
  return {
    name: 'uptrakit:theme-tokens',
    resolveId(id) {
      if (id === VIRTUAL_ID) return RESOLVED_VIRTUAL_ID;
      return undefined;
    },
    load(id) {
      if (id !== RESOLVED_VIRTUAL_ID) return undefined;
      return [
        ':root {',
        '  color-scheme: light;',
        cssForTheme('light'),
        '}',
        '.dark {',
        '  color-scheme: dark;',
        cssForTheme('dark'),
        '}',
        ''
      ].join('\n');
    },
    handleHotUpdate({ file, server }) {
      if (!file.endsWith(TOKENS_SOURCE_SUFFIX)) return;
      const virtualModule = server.moduleGraph.getModuleById(RESOLVED_VIRTUAL_ID);
      if (!virtualModule) return;
      server.moduleGraph.invalidateModule(virtualModule);
      return [virtualModule];
    }
  };
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd frontend && npm run test -- vite-plugins/theme-tokens.test.ts`
Expected: PASS, 8 tests green.

- [ ] **Step 5: Run typecheck + lint + format**

```bash
cd frontend
npm run check
npm run lint
npm run format
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add frontend/vite-plugins/theme-tokens.ts frontend/vite-plugins/theme-tokens.test.ts
git commit -m "feat(frontend): add theme-tokens Vite plugin (sub-spec #1)"
```

---

### Task 3: Register plugin in `vite.config.ts` and extend vitest coverage

**Files:**

- Modify: `frontend/vite.config.ts`
- Modify: `frontend/vitest.config.ts`

`themeTokensPlugin()` is placed **first** in the plugin array so its
`resolveId` hook runs before `@tailwindcss/vite` and `sveltekit()`'s CSS
passes — when `app.css` declares `@import 'virtual:theme/tokens.css'`, the
virtual id must resolve before Tailwind scans the stylesheet for content.

Vitest coverage currently includes only `src/lib/**`. The new token source
of truth lives at `src/theme/` and the plugin at `vite-plugins/`; both are
extended into coverage so future regressions surface against the
thresholds.

- [ ] **Step 1: Update the Vite config**

**Verify before overwriting:** read the current `frontend/vite.config.ts`
and compare against the snippet below. At plan authoring time the file
contained exactly these fields (`plugins`, `build.modulePreload.polyfill`,
`server.proxy`). If the real file has additional fields (new `test`,
`optimizeDeps`, `define`, extra plugins), do NOT use the full-file replace
— instead apply the two targeted edits: (a) add the
`import { themeTokensPlugin } from './vite-plugins/theme-tokens';` line
with the other imports, and (b) prepend `themeTokensPlugin()` to the
existing `plugins` array. Only use the full-file replacement if your diff
shows no additional fields.

Replace the contents of `frontend/vite.config.ts`:

```ts
import tailwindcss from '@tailwindcss/vite';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';
import { themeTokensPlugin } from './vite-plugins/theme-tokens';

export default defineConfig({
  plugins: [themeTokensPlugin(), tailwindcss(), sveltekit()],
  build: {
    modulePreload: {
      // The polyfill injects scripts via blob: URLs, which would require
      // 'blob:' in script-src and weaken the CSP. All browsers that can run
      // this app support <link rel="modulepreload"> natively.
      polyfill: false
    }
  },
  server: {
    proxy: {
      '/api': {
        target: 'https://localhost:8443',
        secure: false
      }
    }
  }
});
```

- [ ] **Step 2: Extend vitest coverage**

Current `frontend/vitest.config.ts` contains a `test` object whose trailing
portion looks like:

```ts
setupFiles: ['./src/test-setup.ts'],
// Exclude Playwright E2E tests — they are run separately via `npm run test:e2e`.
exclude: ['tests/e2e/**', 'node_modules/**'],
coverage: {
  provider: 'v8',
  include: ['src/lib/**'],
  exclude: ['src/lib/**/*.test.ts', 'src/lib/**/*.test.svelte'],
  thresholds: {
    lines: 70,
    branches: 65,
    functions: 70
  }
}
```

Replace the `coverage: { ... }` block with:

```ts
coverage: {
  provider: 'v8',
  include: ['src/lib/**', 'src/theme/**', 'vite-plugins/**'],
  exclude: [
    'src/lib/**/*.test.ts',
    'src/lib/**/*.test.svelte',
    'src/theme/**/*.test.ts',
    'vite-plugins/**/*.test.ts'
  ],
  thresholds: {
    lines: 70,
    branches: 65,
    functions: 70
  }
}
```

The `test.exclude` array and `setupFiles` above it are unchanged.

- [ ] **Step 3: Verify build still succeeds**

Run: `cd frontend && npm run build`
Expected: build completes without errors. The virtual module is registered
but not yet imported by any CSS, so it is effectively dead code in the bundle.

- [ ] **Step 4: Run full test + typecheck**

```bash
cd frontend
npm run check
npm run test
```

Expected: all tests pass (no regressions).

- [ ] **Step 5: Commit**

```bash
git add frontend/vite.config.ts frontend/vitest.config.ts
git commit -m "feat(frontend): register theme-tokens Vite plugin (sub-spec #1)"
```

---

### Task 4: Add golden CSS snapshot test

**Files:**

- Modify: `frontend/vite-plugins/theme-tokens.test.ts`

Pins the exact emitted CSS string. Any future edit to `tokens.ts` or the
plugin template forces the snapshot to update deliberately, catching
accidental formatting or value drift.

- [ ] **Step 1: Append the golden CSS test**

Insert the following `it(...)` block as the last statement inside the
existing `describe('theme-tokens Vite plugin', ...)` body in
`frontend/vite-plugins/theme-tokens.test.ts` — i.e., after the last
existing `it('handleHotUpdate ignores unrelated file changes', ...)` block
and before the closing `});` of the `describe`:

```ts
it('emits the spec-pinned golden CSS for both themes', () => {
  const css = callLoad('\0' + VIRTUAL_ID)!;
  const expected = [
    ':root {',
    '  color-scheme: light;',
    '  --bg-base: #f8fafc;',
    '  --bg-surface: #ffffff;',
    '  --bg-raised: #f1f5f9;',
    '  --border-subtle: #e2e8f0;',
    '  --border-default: #cbd5e1;',
    '  --text-muted: #94a3b8;',
    '  --text-secondary: #64748b;',
    '  --text-primary: #0f172a;',
    '  --text-inverted: #ffffff;',
    '  --accent: #2563eb;',
    '  --accent-rgb: 37 99 235;',
    '  --accent-bright: #3b82f6;',
    '  --accent-dark: #1d4ed8;',
    '  --accent-deep: #1e40af;',
    '  --color-success: #16a34a;',
    '  --color-success-bg: rgba(22, 163, 74, 0.08);',
    '  --color-success-border: rgba(22, 163, 74, 0.3);',
    '  --color-warning: #d97706;',
    '  --color-warning-bg: rgba(217, 119, 6, 0.08);',
    '  --color-warning-border: rgba(217, 119, 6, 0.28);',
    '  --color-error: #dc2626;',
    '  --color-error-bg: rgba(220, 38, 38, 0.07);',
    '  --color-error-border: rgba(220, 38, 38, 0.3);',
    '  --color-info: #0891b2;',
    '  --color-info-bg: rgba(8, 145, 178, 0.08);',
    '  --color-info-border: rgba(8, 145, 178, 0.22);',
    '}',
    '.dark {',
    '  color-scheme: dark;',
    '  --bg-base: #09090b;',
    '  --bg-surface: #111113;',
    '  --bg-raised: #18181b;',
    '  --border-subtle: #1c1c1f;',
    '  --border-default: #27272a;',
    '  --text-muted: #52525b;',
    '  --text-secondary: #a1a1aa;',
    '  --text-primary: #e4e4e7;',
    '  --text-inverted: #fafafa;',
    '  --accent: #06b6d4;',
    '  --accent-rgb: 6 182 212;',
    '  --accent-bright: #22d3ee;',
    '  --accent-dark: #0891b2;',
    '  --accent-deep: #0e7490;',
    '  --color-success: #4ade80;',
    '  --color-success-bg: rgba(74, 222, 128, 0.1);',
    '  --color-success-border: rgba(74, 222, 128, 0.25);',
    '  --color-warning: #fbbf24;',
    '  --color-warning-bg: rgba(251, 191, 36, 0.12);',
    '  --color-warning-border: rgba(251, 191, 36, 0.3);',
    '  --color-error: #fdba74;',
    '  --color-error-bg: rgba(234, 88, 12, 0.15);',
    '  --color-error-border: rgba(234, 88, 12, 0.35);',
    '  --color-info: #67e8f9;',
    '  --color-info-bg: rgba(6, 182, 212, 0.1);',
    '  --color-info-border: rgba(6, 182, 212, 0.22);',
    '}',
    ''
  ].join('\n');

  expect(css).toBe(expected);
});
```

- [ ] **Step 2: Run the test**

Run: `cd frontend && npm run test -- vite-plugins/theme-tokens.test.ts`
Expected: PASS. If the test fails with a diff, compare against the
authoritative table in the plan header — the mismatch is either in
`tokens.ts` values or in the `rgba` emitter formatting.

- [ ] **Step 3: Commit**

```bash
git add frontend/vite-plugins/theme-tokens.test.ts
git commit -m "test(frontend): pin golden CSS for theme-tokens plugin (sub-spec #1)"
```

---

### Task 5: PR1 quality gates + push

- [ ] **Step 1: Run the full frontend gate**

```bash
cd frontend
npm run lint
npm run format:check
npm run check
npm run test
npm run build
```

Expected: all green. No warnings about the new code.

- [ ] **Step 2: Verify nothing in `src/` was accidentally modified**

Run: `git status frontend/src/app.css frontend/src/theme/adapter-manifest.json frontend/src/lib/theme/`
Expected: no changes. The only files touched in PR1 are `src/theme/tokens.ts`,
`src/theme/tokens.test.ts`, `vite-plugins/theme-tokens.ts`,
`vite-plugins/theme-tokens.test.ts`, and `vite.config.ts`.

- [ ] **Step 3: Open PR1**

Push branch and open a PR titled "feat(frontend): add typed token source of
truth + Vite plugin (sub-spec #1 PR 1)". Body explains: "Infrastructure only,
not wired to `app.css` yet. PR2 switches the import and cleans up literals."

PR1 is mergeable on its own — no visual regression, no behavior change.

---

## PR 2 — Switch + cleanup

PR2 assumes PR1 is merged. All remaining changes land together because the
CSS file, the two test files, and the deleted JSON are mutually dependent:
deleting the literal `:root` block without rewriting
`design-token-values.test.ts` breaks CI.

### Task 6: Rewrite `design-token-values.test.ts` to import from `tokens.ts`

**Files:**

- Modify: `frontend/src/lib/theme/design-token-values.test.ts`

Done first because `tokens.ts` already carries the spec-correct values; the
rewritten test will pass against the pre-PR2 codebase (literal `app.css`
blocks still present, but the test no longer depends on them).

**TDD note:** This task rewrites an existing test file whose implementation
target (`tokens.ts`) already shipped in PR1. There is no red step — the
rewrite goes straight to green. Tasks 1 and 2 follow the strict red→green
cycle because they create new modules from nothing; Task 6 is an
intentional deviation.

- [ ] **Step 1: Replace the file contents**

Overwrite `frontend/src/lib/theme/design-token-values.test.ts` with:

```ts
import { describe, expect, it } from 'vitest';
import { cssForTheme, tokens, type TokenName, type Theme } from '../../theme/tokens';

const SPEC: Record<TokenName, Record<Theme, string>> = {
  '--bg-base': { dark: '#09090b', light: '#f8fafc' },
  '--bg-surface': { dark: '#111113', light: '#ffffff' },
  '--bg-raised': { dark: '#18181b', light: '#f1f5f9' },
  '--border-subtle': { dark: '#1c1c1f', light: '#e2e8f0' },
  '--border-default': { dark: '#27272a', light: '#cbd5e1' },
  '--text-muted': { dark: '#52525b', light: '#94a3b8' },
  '--text-secondary': { dark: '#a1a1aa', light: '#64748b' },
  '--text-primary': { dark: '#e4e4e7', light: '#0f172a' },
  '--text-inverted': { dark: '#fafafa', light: '#ffffff' },
  '--accent': { dark: '#06b6d4', light: '#2563eb' },
  '--accent-rgb': { dark: '6 182 212', light: '37 99 235' },
  '--accent-bright': { dark: '#22d3ee', light: '#3b82f6' },
  '--accent-dark': { dark: '#0891b2', light: '#1d4ed8' },
  '--accent-deep': { dark: '#0e7490', light: '#1e40af' },
  '--color-success': { dark: '#4ade80', light: '#16a34a' },
  '--color-success-bg': {
    dark: 'rgba(74, 222, 128, 0.1)',
    light: 'rgba(22, 163, 74, 0.08)'
  },
  '--color-success-border': {
    dark: 'rgba(74, 222, 128, 0.25)',
    light: 'rgba(22, 163, 74, 0.3)'
  },
  '--color-warning': { dark: '#fbbf24', light: '#d97706' },
  '--color-warning-bg': {
    dark: 'rgba(251, 191, 36, 0.12)',
    light: 'rgba(217, 119, 6, 0.08)'
  },
  '--color-warning-border': {
    dark: 'rgba(251, 191, 36, 0.3)',
    light: 'rgba(217, 119, 6, 0.28)'
  },
  '--color-error': { dark: '#fdba74', light: '#dc2626' },
  '--color-error-bg': {
    dark: 'rgba(234, 88, 12, 0.15)',
    light: 'rgba(220, 38, 38, 0.07)'
  },
  '--color-error-border': {
    dark: 'rgba(234, 88, 12, 0.35)',
    light: 'rgba(220, 38, 38, 0.3)'
  },
  '--color-info': { dark: '#67e8f9', light: '#0891b2' },
  '--color-info-bg': {
    dark: 'rgba(6, 182, 212, 0.1)',
    light: 'rgba(8, 145, 178, 0.08)'
  },
  '--color-info-border': {
    dark: 'rgba(6, 182, 212, 0.22)',
    light: 'rgba(8, 145, 178, 0.22)'
  }
};

const SPEC_NAMES = Object.keys(SPEC) as TokenName[];

describe('design token values', () => {
  it('pins every dark-theme token to the approved spec value', () => {
    for (const name of SPEC_NAMES) {
      expect(tokens[name].dark, `dark ${name}`).toBe(SPEC[name].dark);
    }
  });

  it('pins every light-theme token to the approved spec value', () => {
    for (const name of SPEC_NAMES) {
      expect(tokens[name].light, `light ${name}`).toBe(SPEC[name].light);
    }
  });

  it('keeps info tokens distinct from accent tokens in both themes', () => {
    expect(tokens['--color-info'].dark).not.toBe(tokens['--accent'].dark);
    expect(tokens['--color-info'].light).not.toBe(tokens['--accent'].light);
  });

  it('snapshot: cssForTheme(light) output matches the canonical form', () => {
    expect(cssForTheme('light')).toMatchInlineSnapshot(`
"  --bg-base: #f8fafc;
  --bg-surface: #ffffff;
  --bg-raised: #f1f5f9;
  --border-subtle: #e2e8f0;
  --border-default: #cbd5e1;
  --text-muted: #94a3b8;
  --text-secondary: #64748b;
  --text-primary: #0f172a;
  --text-inverted: #ffffff;
  --accent: #2563eb;
  --accent-rgb: 37 99 235;
  --accent-bright: #3b82f6;
  --accent-dark: #1d4ed8;
  --accent-deep: #1e40af;
  --color-success: #16a34a;
  --color-success-bg: rgba(22, 163, 74, 0.08);
  --color-success-border: rgba(22, 163, 74, 0.3);
  --color-warning: #d97706;
  --color-warning-bg: rgba(217, 119, 6, 0.08);
  --color-warning-border: rgba(217, 119, 6, 0.28);
  --color-error: #dc2626;
  --color-error-bg: rgba(220, 38, 38, 0.07);
  --color-error-border: rgba(220, 38, 38, 0.3);
  --color-info: #0891b2;
  --color-info-bg: rgba(8, 145, 178, 0.08);
  --color-info-border: rgba(8, 145, 178, 0.22);"
`);
  });

  it('snapshot: cssForTheme(dark) output matches the canonical form', () => {
    expect(cssForTheme('dark')).toMatchInlineSnapshot(`
"  --bg-base: #09090b;
  --bg-surface: #111113;
  --bg-raised: #18181b;
  --border-subtle: #1c1c1f;
  --border-default: #27272a;
  --text-muted: #52525b;
  --text-secondary: #a1a1aa;
  --text-primary: #e4e4e7;
  --text-inverted: #fafafa;
  --accent: #06b6d4;
  --accent-rgb: 6 182 212;
  --accent-bright: #22d3ee;
  --accent-dark: #0891b2;
  --accent-deep: #0e7490;
  --color-success: #4ade80;
  --color-success-bg: rgba(74, 222, 128, 0.1);
  --color-success-border: rgba(74, 222, 128, 0.25);
  --color-warning: #fbbf24;
  --color-warning-bg: rgba(251, 191, 36, 0.12);
  --color-warning-border: rgba(251, 191, 36, 0.3);
  --color-error: #fdba74;
  --color-error-bg: rgba(234, 88, 12, 0.15);
  --color-error-border: rgba(234, 88, 12, 0.35);
  --color-info: #67e8f9;
  --color-info-bg: rgba(6, 182, 212, 0.1);
  --color-info-border: rgba(6, 182, 212, 0.22);"
`);
  });
});
```

- [ ] **Step 2: Run the rewritten test**

Run: `cd frontend && npm run test -- src/lib/theme/design-token-values.test.ts`
Expected: PASS, 5 tests green.

- [ ] **Step 3: Run typecheck**

Run: `cd frontend && npm run check`
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/theme/design-token-values.test.ts
git commit -m "test(frontend): rewrite design-token-values test to use tokens.ts (sub-spec #1)"
```

---

### Task 7: Rename `adapter-manifest.test.ts` → `css-contract.test.ts` and strip manifest blocks

**Files:**

- Rename: `frontend/src/lib/theme/adapter-manifest.test.ts` →
  `frontend/src/lib/theme/css-contract.test.ts`
- Overwrite the renamed file with the content shown in Step 2 below. The
  replacement omits the two manifest-related `it` blocks that existed in
  the original (manifest completeness + mapping pins), renames the
  enclosing `describe` wrapper from `'adapter manifest'` to
  `'app.css structural contract'`, and preserves the z-index / transition
  / focus-visible / aria-invalid assertions.

- [ ] **Step 1: Use `git mv` to preserve history**

```bash
git mv frontend/src/lib/theme/adapter-manifest.test.ts frontend/src/lib/theme/css-contract.test.ts
```

- [ ] **Step 2: Replace the file contents**

Overwrite `frontend/src/lib/theme/css-contract.test.ts` with (the two
manifest `it` blocks removed; structural app.css assertions preserved):

```ts
import { describe, expect, it } from 'vitest';

// @ts-expect-error node:fs is not part of the browser-focused frontend type environment
const { readFileSync } = await import('node:fs');
// @ts-expect-error node:url is not part of the browser-focused frontend type environment
const { fileURLToPath } = await import('node:url');

function resolveFromThisTest(relativePath: string): string {
  const resolved = new URL(relativePath, import.meta.url);
  if (resolved.protocol === 'file:') {
    return fileURLToPath(resolved);
  }

  // Vitest can expose non-file module URLs; keep resolution anchored to this test URL.
  return decodeURIComponent(resolved.pathname).replace(/^\/@fs/, '');
}

const appCss = readFileSync(resolveFromThisTest('../../app.css'), 'utf8');

describe('app.css structural contract', () => {
  it('pins the shared layering z-index contract in app.css', () => {
    expect(appCss).toMatch(/\[data-ui='app-shell-header'\][\s\S]*?z-index:\s*10;/);
    expect(appCss).toMatch(/\[data-ui='app-shell-sidebar'\][\s\S]*?z-index:\s*20;/);
    expect(appCss).toMatch(/\[data-ui='context-menu-shell'\][\s\S]*?z-index:\s*100;/);
    expect(appCss).toMatch(/\[data-ui='toast-notifications'\][\s\S]*?z-index:\s*500;/);
    expect(appCss).toMatch(/\[data-ui='modal-backdrop'\][\s\S]*?z-index:\s*900;/);
    expect(appCss).toMatch(/\[data-ui='modal-shell'\][\s\S]*?z-index:\s*910;/);
  });

  it('pins global transition and focus-visible interaction rules', () => {
    const transitionDeclarations = [...appCss.matchAll(/transition:\s*([^;]+);/g)].map(
      (match) => match[1]
    );
    expect(transitionDeclarations.length).toBeGreaterThan(0);

    const allowedTransitionProperties = new Set(['background', 'border-color', 'color']);
    for (const declaration of transitionDeclarations) {
      const properties = declaration
        .split(',')
        .map((segment: string) => segment.trim().split(/\s+/)[0])
        .filter(Boolean);

      for (const property of properties) {
        expect(allowedTransitionProperties).toContain(property);
      }
    }

    expect(appCss).toMatch(
      /:is\(button, \[href\], input, select, textarea, summary, \[role='button'\], \[role='tab'\]\):focus-visible[\s\S]*?outline:\s*none;[\s\S]*?box-shadow:\s*0 0 0 3px rgba\(var\(--accent-rgb\), 0.25\);/
    );
    expect(appCss).toMatch(
      /:is\(input, select, textarea\)\[aria-invalid='true'\]:focus-visible[\s\S]*?border-color:\s*var\(--color-error-border\);/
    );
  });
});
```

- [ ] **Step 3: Run the test**

Run: `cd frontend && npm run test -- src/lib/theme/css-contract.test.ts`
Expected: PASS, 2 tests green (against the still-unchanged `app.css`).

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/theme/css-contract.test.ts
git commit -m "refactor(frontend): rename adapter-manifest test to css-contract (sub-spec #1)"
```

---

### Task 8: Delete `adapter-manifest.json`

**Files:**

- Delete: `frontend/src/theme/adapter-manifest.json`

- [ ] **Step 1: Remove the file**

```bash
git rm frontend/src/theme/adapter-manifest.json
```

- [ ] **Step 2: Verify no consumer remains**

Run a repo-wide grep to confirm nothing still imports the manifest:

```bash
cd frontend
grep -rn "adapter-manifest" src vite-plugins tests 2>/dev/null || echo "no references"
```

Expected output: `no references`. If any hit comes back, inspect and remove
the reference before proceeding.

- [ ] **Step 3: Run tests**

Run: `cd frontend && npm run test`
Expected: all green. `css-contract.test.ts` no longer references the
manifest; `tokens.test.ts` and `design-token-values.test.ts` never did.

- [ ] **Step 4: Commit**

```bash
git commit -m "chore(frontend): remove decorative adapter-manifest.json (sub-spec #1)"
```

---

### Task 9: Update `app.css` — delete literal blocks, add virtual import, fix skip-link

**Files:**

- Modify: `frontend/src/app.css`

- [ ] **Step 1: Replace the file contents**

**Verify before overwriting:** read the current `frontend/src/app.css` and
diff against the snippet below. At plan authoring time the file contained
exactly these top-level directives (`@import 'tailwindcss';`,
`@custom-variant dark ...`, `@plugin '@tailwindcss/forms';`, three
`@import '@skeletonlabs/...'` lines, the `:root` and `.dark` literal blocks
about to be deleted, the interactive rules, `.skip-link`, and the six
`[data-ui='...']` z-index selectors). If the real file has new Skeleton
imports, new `@plugin` lines, extra top-level CSS (new selectors, rules, or
`@layer` blocks), or a different Skeleton theme import, do NOT use the
full-file replace — instead apply targeted edits: (a) delete the `:root { ...
}` and `.dark { ... }` literal blocks, (b) add
`@import 'virtual:theme/tokens.css';` after the last `@import '@skeletonlabs/...'`
line, (c) change `.skip-link` `background` from
`var(--color-primary-500, #0070f3)` to `var(--accent)` and `color` from
`#fff` to `var(--text-inverted)`. Only use the full-file replacement if the
diff shows the file matches this snippet exactly.

Overwrite `frontend/src/app.css` with:

```css
@import 'tailwindcss';

@custom-variant dark (&:where(.dark, .dark *));

@plugin '@tailwindcss/forms';

@import '@skeletonlabs/skeleton';
@import '@skeletonlabs/skeleton-svelte';
@import '@skeletonlabs/skeleton/themes/cerberus';

@import 'virtual:theme/tokens.css';

.input,
.textarea,
select {
  padding: 0.5rem 0.75rem;
}

:is(button, [href], input, select, textarea, summary, [role='button'], [role='tab']) {
  transition:
    background 0.12s,
    border-color 0.12s,
    color 0.12s;
}

:is(button, [href], input, select, textarea, summary, [role='button'], [role='tab']):focus-visible {
  outline: none;
  box-shadow: 0 0 0 3px rgba(var(--accent-rgb), 0.25);
}

:is(input, select, textarea)[aria-invalid='true']:focus-visible {
  border-color: var(--color-error-border);
}

:is(button, [role='button'], [role='tab'])[disabled],
:is(button, [role='button'], [role='tab'])[aria-disabled='true'] {
  opacity: 0.4;
  pointer-events: none;
}

.skip-link {
  position: absolute;
  transform: translateY(-100%);
  top: 0;
  left: 0;
  z-index: 100;
  padding: 0.5rem 1rem;
  background: var(--accent);
  color: var(--text-inverted);
  border-radius: 0 0 0.25rem 0;
  font-weight: 600;
}

.skip-link:focus {
  transform: translateY(0);
}

[data-ui='app-shell-header'] {
  z-index: 10;
}

[data-ui='app-shell-sidebar'],
[data-ui='app-shell-sidebar-backdrop'],
[data-ui='app-shell-mobile-nav'],
[data-ui='app-shell-mobile-overflow-backdrop'],
[data-ui='app-shell-mobile-overflow-sheet'] {
  z-index: 20;
}

[data-ui='context-menu-shell'] {
  z-index: 100;
}

[data-ui='toast-notifications'] {
  z-index: 500;
}

[data-ui='modal-backdrop'] {
  z-index: 900;
}

[data-ui='modal-shell'] {
  z-index: 910;
}
```

Notes on what changed vs. previous `app.css`:

- Removed the entire `:root { color-scheme: light; ... }` block (was lines
  11–47). Values now emitted by the virtual module.
- Removed the entire `.dark { color-scheme: dark; ... }` block (was lines
  49–85). Values now emitted by the virtual module.
- Added `@import 'virtual:theme/tokens.css';` after the Skeleton imports.
- `.skip-link` `background` switched from `var(--color-primary-500, #0070f3)`
  to `var(--accent)`. `color` switched from `#fff` to `var(--text-inverted)`.

- [ ] **Step 2: Verify css-contract.test.ts still passes**

Run: `cd frontend && npm run test -- src/lib/theme/css-contract.test.ts`
Expected: PASS. Z-index selectors, transition triplet, focus-visible rule,
and aria-invalid rule remain in the file.

- [ ] **Step 3: Verify design-token-values.test.ts still passes**

Run: `cd frontend && npm run test -- src/lib/theme/design-token-values.test.ts`
Expected: PASS. Test does not read `app.css`.

- [ ] **Step 4: Verify build + dev server still start**

```bash
cd frontend
npm run build
```

Expected: build succeeds. The virtual module is resolved by the plugin and
its output inlined into the final stylesheet bundle.

- [ ] **Step 5: Smoke check the rendered page**

```bash
cd frontend
npm run dev
```

Open `http://localhost:5173/` and confirm the page renders (dark theme by
default). Toggle the theme switcher to light and back to dark; confirm no
FOUC and that colors match the spec. Kill the dev server (`Ctrl+C`) when
done.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/app.css
git commit -m "feat(frontend): switch app.css to virtual tokens module (sub-spec #1)"
```

---

### Task 10: PR2 full quality gate + Playwright smoke

- [ ] **Step 1: Run the full frontend gate**

```bash
cd frontend
npm run lint
npm run format:check
npm run check
npm run test
npm run build
```

Expected: all green.

- [ ] **Step 2: Run existing Playwright E2E suite**

```bash
cd frontend
npm run test:e2e
```

Expected: all existing e2e tests pass. The parent spec mandates 0.5% visual
diff tolerance on paired snapshots; if any snapshot exceeds 0.5%, stop and
follow up per sub-spec §Testing "Visual regression" — the corrected token
values intentionally change a small set of known-delta snapshots (light
success/error borders more saturated, dark success bg lighter, dark error
bg/border orange-600-based, `--text-inverted` contrast delta in accent
fills). If any snapshot fails, update the fixture or file a waiver per
parent spec §9; do NOT silently relax the threshold.

- [ ] **Step 3: Verify the `--color-error-bg` value in the browser**

While a dev server is running, open the browser devtools console on the app
and execute:

```js
getComputedStyle(document.documentElement).getPropertyValue('--color-error-bg').trim();
```

Expected (light theme, default): `rgba(220, 38, 38, 0.07)`. Toggle the
`.dark` class on `<html>` and re-run; expected: `rgba(234, 88, 12, 0.15)`.
This confirms the virtual module is being applied and overrides any stale
browser cache.

- [ ] **Step 4: Verify no orphaned references to deleted files**

```bash
grep -rn "adapter-manifest" frontend/src frontend/vite-plugins 2>/dev/null || echo "clean"
grep -rn "--theme-accent" frontend/src 2>/dev/null || echo "clean"
grep -rn "--theme-info" frontend/src 2>/dev/null || echo "clean"
```

Expected: three `clean` lines. If any hit remains, remove the reference or
migrate it to the new semantic token name.

- [ ] **Step 5: Open PR2**

Push branch and open a PR titled "feat(frontend): switch to virtual tokens
module + drop adapter-manifest (sub-spec #1 PR 2)". Body links back to PR1
and calls out the known-delta snapshots.

---

## Self-Review

### Spec coverage checklist

- [x] Goal 1 (spec-correct values, both themes) — Task 1 pins values in
  `tokens.ts`; Task 4 golden CSS locks emission; Task 6 rewrites
  `design-token-values.test.ts` to enforce values against the module.
- [x] Goal 2 (built-in + surface UI consume same adapter) — Tasks 1–3 ship
  the adapter layer; Task 9 wires `app.css` through it.
- [x] Goal 3 (drift prevented structurally) — Task 1's `Record<TokenName,
  Record<Theme, TokenValue>>` + typed union catches missing tokens at
  compile time; Task 4 golden CSS catches format drift.
- [x] Goal 4 (`getToken(name, theme)` available) — Task 1 exports
  `getToken`; Task 1 Step 1 test asserts it is equivalent to the table.
- [x] Non-goals honored — plan touches no Skeleton config, no call sites,
  no new tokens, no theme switcher, no fixtures.
- [x] Current-state drift entries all corrected — Task 1 `tokens.ts` uses
  spec values for every drifted entry (light `--color-success-border` .3,
  light `--color-warning-bg` .08, light `--color-warning-border` .28, light
  `--color-error-bg` .07, light `--color-error-border` .3, light
  `--text-inverted` `#ffffff`, dark `--color-success-bg` .10, dark
  `--color-success-border` .25, dark `--color-warning-bg` .12, dark
  `--color-warning-border` .30, dark `--color-error-bg` orange-600 .15,
  dark `--color-error-border` orange-600 .35, dark `--text-inverted`
  `#fafafa`).
- [x] Architecture — Tasks 1–3 build `tokens.ts` → plugin → `@import`
  chain; Task 9 wires the `@import`.
- [x] Components section — every file listed in spec §Components has a
  corresponding task: `tokens.ts` (Task 1), `tokens.test.ts` (Task 1),
  `theme-tokens.ts` (Task 2), `theme-tokens.test.ts` (Task 2), modified
  `app.css` (Task 9), modified `design-token-values.test.ts` (Task 6),
  renamed `css-contract.test.ts` (Task 7), deleted `adapter-manifest.json`
  (Task 8).
- [x] Data flow — build-time path verified by Task 9 Step 4 build; runtime
  path verified by Task 9 Step 5 dev smoke; test-time path verified by
  Tasks 1, 4, 6.
- [x] Error handling — TS union + `Record` enforces completeness (compile-
  time); drift-detection (test-time) by Tasks 1 Step 1 and 6 Step 1; Task
  10 Step 3 confirms runtime resolution.
- [x] Testing — unit (Tasks 1, 2, 4), integration (Task 6), e2e (Task 10
  Step 2), structural app.css (Task 7).
- [x] Rollout PR1/PR2 split — honored (divergence called out in Rollout
  structure above).
- [x] Intermediary removal (`--theme-accent*`, `--theme-info*`) — Task 9
  removes them with the literal blocks; Task 10 Step 4 greps for leftovers.
- [x] Skip-link fix — Task 9 Step 1.
- [x] CI gate "no raw hex/rgba outside `frontend/src/theme/`" — parent
  spec calls this out as a new CI gate but scopes its wiring to later
  sub-specs; not in this plan's scope. Task 10 Step 4 covers the immediate
  regression check for `--theme-accent*` / `--theme-info*` leftovers.

**Placeholder scan:** No "TBD", "TODO", or unfilled blocks. Every code
step carries the full code to paste.

**Type consistency:** `TokenName`, `Theme`, `TokenValue`, `tokens`,
`cssForTheme`, `getToken`, `themeTokensPlugin`, `VIRTUAL_ID` used
identically across Tasks 1, 2, 4, 6.

**Golden CSS + rewritten test consistency:** The inline snapshot in Task 6
Step 1 matches `cssForTheme()` output for both themes exactly. The golden
CSS in Task 4 wraps the same strings in `:root { color-scheme: light; ...
}` and `.dark { color-scheme: dark; ... }` shells. Both are derived from
the same authoritative table in this plan's header. Any future spec value
change requires updating six places: (1) the plan header Dark/Light tables,
(2) `tokens.ts`, (3) the Task 1 `EXPECTED` table, (4) the Task 4 golden
CSS, (5) the Task 6 `SPEC` table, and (6) both Task 6 `toMatchInlineSnapshot`
blocks — deliberate friction by design.
