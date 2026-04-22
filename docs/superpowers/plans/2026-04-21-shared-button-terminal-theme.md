# Shared Button Primitive + Terminal Theme Derivation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship prop-driven `Button.svelte` + semantically-separate `UpdateAllButton.svelte` primitives, extract xterm palette into a typed module
bound to `tokens.ts`, and migrate five canary call sites.

**Architecture:** Two PRs. PR1 adds two new danger-hover tokens, three new components (`Button`, `UpdateAllButton`, `terminal-palette`) with Vitest
coverage, zero consumer touches. PR2 swaps `TerminalOutput.svelte` to the palette module, migrates five canary sites, adds a dev-only
`/dev/button-preview` route, and baselines a new Playwright suite.

**Tech Stack:** Svelte 5 runes + snippets, SvelteKit, Tailwind v4, Vitest inline snapshots, Playwright, `@xterm/xterm` types, `tokens.ts`
virtual-module CSS cascade (sub-spec #1).

**Spec:** `docs/superpowers/specs/2026-04-21-shared-button-terminal-theme-design.md`

**Working directory for all commands:** workspace root `/Users/andreyyantsen/Development/uptrakit`. Any frontend command is shown with `cd frontend
&&` prefix.

---

## File structure

### New files (PR1)

| Path | Responsibility |
| --- | --- |
| `frontend/src/lib/components/Button.svelte` | Prop-driven button primitive; dual render branches (`<button>` / `<a>`). |
| `frontend/src/lib/components/Button.test.ts` | Variant × size matrix, TS type gates, loading/disabled gating. |
| `frontend/src/lib/components/UpdateAllButton.svelte` | Row-level "update all" badge primitive; idle vs dim states. |
| `frontend/src/lib/components/UpdateAllButton.test.ts` | State branches, count suffix, dim-click gating. |
| `frontend/src/theme/terminal-palette.ts` | Exports `TERMINAL_THEME: ITheme` bound to `tokens.ts` dark values + local ANSI hex. |
| `frontend/src/theme/terminal-palette.test.ts` | Per-slot parent-spec §6 binding checks + full inline snapshot. |

### Modified files (PR1)

| Path | Change |
| --- | --- |
| `frontend/src/theme/tokens.ts` | Add `--color-error-bg-hover`, `--color-error-border-hover`; extend `TokenName`. |
| `frontend/src/theme/tokens.test.ts` | Extend `EXPECTED` table; preserve all existing tests. |
| `frontend/src/lib/theme/design-token-values.test.ts` | Extend `SPEC` table; regenerate both inline CSS snapshots. |
| `frontend/vite-plugins/theme-tokens.test.ts` | Regenerate the spec-pinned golden CSS block. |

### New files (PR2)

| Path | Responsibility |
| --- | --- |
| `frontend/src/routes/dev/button-preview/+page.svelte` | Dev-only gallery rendering every variant × size × state permutation. |
| `frontend/tests/e2e/button-primitive.spec.ts` | Playwright snapshots of `/dev/button-preview` in dark + light. |

### Modified files (PR2)

| Path | Change |
| --- | --- |
| `frontend/src/lib/components/TerminalOutput.svelte` | Delete inline `TERMINAL_THEME` literal; import from `../../theme/terminal-palette`. |
| `frontend/src/lib/components/TerminalOutput.test.ts` | Add one `it` asserting reference identity (`===`) against the imported constant. |
| `frontend/src/routes/profile/+page.svelte` | "Save changes" → `<Button variant="primary" type="submit" loading={…}>`. |
| `frontend/src/lib/components/ConfirmDialog.svelte` | Cancel button → `<Button variant="ghost">`. |
| `frontend/src/lib/components/ui/PublicEntryShell.svelte` | "Back to login" → `<Button variant="ghost" href="/login">`. |
| `frontend/src/routes/settings/EnrollmentTokenSettings.svelte` | First "Revoke" action → `<Button variant="danger" size="sm" leadingIcon={…}>`. |
| `frontend/src/routes/software/+page.svelte` | Header "Update all" → `<UpdateAllButton state={…} count={…} onclick={…} />`. |
| `frontend/tests/e2e/ui-parity.test.ts-snapshots/` + `ui-parity-responsive.test.ts-snapshots/` | Re-baseline the five canary scenes (macOS + Chromium only). |

---

## Global conventions

- **Svelte 5 runes:** every new component uses `$props()` destructuring, `let { … }: Props = $props();` pattern, snippets for slot content.
- **Class concatenation:** primitives accept `class?: string`; merge with internal base via template string with a single space separator. Do not
  introduce `clsx` or similar — project convention is bare strings.
- **Prettier settings** (`frontend/.prettierrc`): `useTabs: true`, `printWidth: 120`, `singleQuote: true`, `trailingComma: 'none'`. All new TS/Svelte
  files must match or `npm run format:check` fails.
- **TS strictness:** `frontend/tsconfig.json` has strict on. `export type` instead of `export interface` for plain object shapes, to match existing
  components.
- **Paths:** `$lib` alias covers `src/lib/`. Cross-directory imports from `src/lib/**` into `src/theme/**` use relative paths
  (`../../theme/terminal-palette`) per spec §Terminal output consumer.

---

## PR1 — Primitives + palette module

### Task 1: Add two danger-hover tokens + propagate to all four consumers

**Files:**

- Modify: `frontend/src/theme/tokens.ts`
- Modify: `frontend/src/theme/tokens.test.ts`
- Modify: `frontend/src/lib/theme/design-token-values.test.ts`
- Modify: `frontend/vite-plugins/theme-tokens.test.ts`

**Rationale:** Inline snapshots live only in `design-token-values.test.ts` + `theme-tokens.test.ts`. The `tokens.test.ts` file is value-table-only.
All four must ship in the same commit so the frontend test suite stays green.

- [ ] **Step 1: Add `TokenName` union entries and table rows**

Edit `frontend/src/theme/tokens.ts`:

```ts
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
  | '--color-error-bg-hover'
  | '--color-error-border-hover'
  | '--color-info'
  | '--color-info-bg'
  | '--color-info-border';
```

Inside the `tokens` record literal, immediately after the existing `'--color-error-border'` entry (current line 84-87), insert:

```ts
  '--color-error-bg-hover': {
    dark: rgba(errorBase.dark, 0.22),
    light: rgba(errorBase.light, 0.14)
  },
  '--color-error-border-hover': {
    dark: rgba(errorBase.dark, 0.5),
    light: rgba(errorBase.light, 0.45)
  },
```

- [ ] **Step 2: Run `tokens.test.ts` — expect failure**

Run: `cd frontend && npx vitest run src/theme/tokens.test.ts`
Expected: test "pins every (name, theme) pair to the spec-approved value" passes but "defines every TokenName for both dark and light themes" +
"exposes getToken as a lookup helper" + "cssForTheme emits every TokenName" fail because `EXPECTED` does not include the new tokens but the code now
emits them (`cssForTheme` emits 28 entries, loop checks 26). Actually: the for-loop iterates `EXPECTED_TOKEN_NAMES`, so existing tests pass; however
the `TokenName` type change makes the `EXPECTED: Record<TokenName, …>` literal require the two new keys, so `tsc` (via `vitest`) fails with `Property
'--color-error-bg-hover' is missing in type`. Verify the failure says "missing property `--color-error-bg-hover`".

- [ ] **Step 3: Extend `EXPECTED` table in `tokens.test.ts`**

Edit `frontend/src/theme/tokens.test.ts` — inside the `EXPECTED` object literal, immediately after the `'--color-error-border'` block, insert:

```ts
  '--color-error-bg-hover': {
    dark: 'rgba(234, 88, 12, 0.22)',
    light: 'rgba(220, 38, 38, 0.14)'
  },
  '--color-error-border-hover': {
    dark: 'rgba(234, 88, 12, 0.5)',
    light: 'rgba(220, 38, 38, 0.45)'
  },
```

- [ ] **Step 4: Run `tokens.test.ts` — expect pass**

Run: `cd frontend && npx vitest run src/theme/tokens.test.ts`
Expected: all 6 tests pass.

- [ ] **Step 5: Run `design-token-values.test.ts` — expect failure on SPEC + both snapshots**

Run: `cd frontend && npx vitest run src/lib/theme/design-token-values.test.ts`
Expected: TS error "Property '--color-error-bg-hover' is missing" on the `SPEC` object literal, and if that is skipped, the two
`toMatchInlineSnapshot` blocks fail because the emitted CSS now contains 28 lines but the snapshot pins 26.

- [ ] **Step 6: Extend `SPEC` + regenerate snapshots in `design-token-values.test.ts`**

Edit `frontend/src/lib/theme/design-token-values.test.ts` — inside the `SPEC` literal, after `'--color-error-border'`:

```ts
  '--color-error-bg-hover': {
    dark: 'rgba(234, 88, 12, 0.22)',
    light: 'rgba(220, 38, 38, 0.14)'
  },
  '--color-error-border-hover': {
    dark: 'rgba(234, 88, 12, 0.5)',
    light: 'rgba(220, 38, 38, 0.45)'
  },
```

Replace the light snapshot body (currently ending with `--color-error-border: rgba(220, 38, 38, 0.3);`) — insert two lines between
`--color-error-border` and `--color-info`:

```text
  --color-error-bg-hover: rgba(220, 38, 38, 0.14);
  --color-error-border-hover: rgba(220, 38, 38, 0.45);
```

Replace the dark snapshot body analogously — insert between `--color-error-border` and `--color-info`:

```text
  --color-error-bg-hover: rgba(234, 88, 12, 0.22);
  --color-error-border-hover: rgba(234, 88, 12, 0.5);
```

- [ ] **Step 7: Run `design-token-values.test.ts` — expect pass**

Run: `cd frontend && npx vitest run src/lib/theme/design-token-values.test.ts`
Expected: all 4 tests pass (2 value tests + 2 inline snapshots).

- [ ] **Step 8: Run `theme-tokens.test.ts` — expect failure on golden CSS**

Run: `cd frontend && npx vitest run vite-plugins/theme-tokens.test.ts`
Expected: "emits the spec-pinned golden CSS for both themes" fails because emitted CSS now includes two extra lines per theme. Other tests still pass
(they count occurrences, not lines).

- [ ] **Step 9: Extend the golden CSS in `theme-tokens.test.ts`**

Edit `frontend/vite-plugins/theme-tokens.test.ts` — inside the `expected` array literal. In the light block, between the line `'
--color-error-border: rgba(220, 38, 38, 0.3);',` and `'  --color-info: #0891b2;',`, insert:

```ts
        '  --color-error-bg-hover: rgba(220, 38, 38, 0.14);',
        '  --color-error-border-hover: rgba(220, 38, 38, 0.45);',
```

In the dark block, between `'  --color-error-border: rgba(234, 88, 12, 0.35);',` and `'  --color-info: #67e8f9;',`, insert:

```ts
        '  --color-error-bg-hover: rgba(234, 88, 12, 0.22);',
        '  --color-error-border-hover: rgba(234, 88, 12, 0.5);',
```

- [ ] **Step 10: Run the theme test suite — expect pass**

Run: `cd frontend && npx vitest run src/theme/ src/lib/theme/ vite-plugins/theme-tokens.test.ts`
Expected: all suites pass.

- [ ] **Step 11: Commit**

```bash
git add frontend/src/theme/tokens.ts \
  frontend/src/theme/tokens.test.ts \
  frontend/src/lib/theme/design-token-values.test.ts \
  frontend/vite-plugins/theme-tokens.test.ts
git commit -m "feat(theme): add danger-variant hover tokens (sub-spec #2 PR1)"
```

---

### Task 2: Scaffold `Button.svelte` primitive (types, render branches, class contract)

**Files:**

- Create: `frontend/src/lib/components/Button.svelte`

- [ ] **Step 1: Create the component file**

Write `frontend/src/lib/components/Button.svelte`:

```svelte
<script lang="ts" module>
  import type { Snippet } from 'svelte';
  import type { MouseEventHandler } from 'svelte/elements';

  export type ButtonVariant = 'primary' | 'ghost' | 'danger';
  export type ButtonSize = 'sm' | 'md';

  type CommonProps = {
    variant: ButtonVariant;
    size?: ButtonSize;
    disabled?: boolean;
    loading?: boolean;
    leadingIcon?: Snippet;
    trailingIcon?: Snippet;
    children: Snippet;
    class?: string;
  };

  export type ButtonProps =
    | (CommonProps & {
        href: string;
        type?: never;
        onclick?: never;
      })
    | (CommonProps & {
        href?: never;
        type?: 'button' | 'submit' | 'reset';
        onclick?: MouseEventHandler<HTMLButtonElement>;
      });
</script>

<script lang="ts">
  const BASE =
    'inline-flex items-center gap-1.5 rounded-[3px] font-bold uppercase tracking-wide ' +
    'transition-[background,border-color,color] duration-[0.12s] ' +
    'disabled:opacity-40 disabled:pointer-events-none ' +
    'aria-disabled:opacity-40 aria-disabled:pointer-events-none ' +
    'active:opacity-[0.88]';

  const SIZE_CLASSES: Record<ButtonSize, string> = {
    md: 'h-[23px] px-3 text-[9px]',
    sm: 'h-[19px] px-2 text-[8.5px]'
  };

  const VARIANT_CLASSES: Record<ButtonVariant, string> = {
    primary:
      'bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))] ' +
      'text-[var(--text-inverted)] ' +
      'hover:bg-[linear-gradient(90deg,var(--accent-dark),var(--accent-bright))]',
    ghost:
      'bg-transparent border border-[var(--border-default)] ' +
      'text-[var(--text-primary)] ' +
      'hover:bg-[var(--bg-raised)]',
    danger:
      'bg-[var(--color-error-bg)] ' +
      'border border-[var(--color-error-border)] ' +
      'text-[var(--color-error)] ' +
      'hover:bg-[var(--color-error-bg-hover)] ' +
      'hover:border-[var(--color-error-border-hover)]'
  };

  let {
    variant,
    size = 'md',
    disabled = false,
    loading = false,
    leadingIcon,
    trailingIcon,
    children,
    class: className = '',
    href,
    type,
    onclick
  }: ButtonProps = $props();

  const computedClass = $derived(
    [BASE, SIZE_CLASSES[size], VARIANT_CLASSES[variant], className].filter(Boolean).join(' ')
  );

  const inert = $derived(disabled || loading);

  function handleLinkClick(e: MouseEvent) {
    if (inert) e.preventDefault();
  }

  function handleLinkKeydown(e: KeyboardEvent) {
    if (inert && (e.key === ' ' || e.key === 'Enter')) e.preventDefault();
  }
</script>

{#snippet spinner()}
  <span
    class="inline-block h-[9px] w-[9px] animate-spin rounded-full border border-current border-t-transparent [animation-duration:0.7s]"
    aria-hidden="true"
  ></span>
{/snippet}

{#if href !== undefined}
  <a
    {href}
    role="button"
    aria-disabled={inert || undefined}
    aria-busy={loading || undefined}
    onclick={handleLinkClick}
    onkeydown={handleLinkKeydown}
    class={computedClass}
  >
    {#if loading}
      {@render spinner()}
    {:else if leadingIcon}
      {@render leadingIcon()}
    {/if}
    {@render children()}
    {#if trailingIcon && !loading}
      {@render trailingIcon()}
    {/if}
  </a>
{:else}
  <button
    type={type ?? 'button'}
    disabled={inert}
    aria-busy={loading || undefined}
    class={computedClass}
    onclick={inert ? undefined : onclick}
  >
    {#if loading}
      {@render spinner()}
    {:else if leadingIcon}
      {@render leadingIcon()}
    {/if}
    {@render children()}
    {#if trailingIcon && !loading}
      {@render trailingIcon()}
    {/if}
  </button>
{/if}
```

- [ ] **Step 2: Type-check the new component in isolation**

Run: `cd frontend && npx svelte-check --tsconfig tsconfig.json --fail-on-warnings src/lib/components/Button.svelte`
(If the flag form is not supported, run the full check: `cd frontend && npm run check`.)
Expected: no errors. Svelte 5 + the discriminated union on `ButtonProps` must compile cleanly.

- [ ] **Step 3: Lint + format the new file**

Run: `cd frontend && npx prettier --check src/lib/components/Button.svelte && npx eslint src/lib/components/Button.svelte`
Expected: both pass. If prettier fails, run `npx prettier --write src/lib/components/Button.svelte` and re-run the check.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/components/Button.svelte
git commit -m "feat(ui): add Button primitive component (sub-spec #2 PR1)"
```

---

### Task 3: Add `Button.test.ts` covering variants, sizes, gating, and TS type guards

**Files:**

- Create: `frontend/src/lib/components/Button.test.ts`

**Why co-located with `Button.svelte`:** Matches existing convention (`ConfirmDialog.test.ts` lives next to `ConfirmDialog.svelte`).

- [ ] **Step 1: Write the failing test file**

Write `frontend/src/lib/components/Button.test.ts`:

```ts
import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import Button from './Button.svelte';
import type { ButtonProps } from './Button.svelte';

function textSnippet(value: string) {
  return createRawSnippet(() => ({
    render: () => `<span>${value}</span>`
  }));
}

function mdButton(extra: Partial<ButtonProps> = {}) {
  return {
    variant: 'primary' as const,
    children: textSnippet('Go'),
    ...extra
  };
}

describe('Button primitive', () => {
  it('renders a <button type="button"> by default when href is omitted', () => {
    const { container } = render(Button, mdButton());
    const btn = container.querySelector('button');
    expect(btn).not.toBeNull();
    expect(btn?.getAttribute('type')).toBe('button');
  });

  it('honours explicit type="submit" on the button branch', () => {
    const { container } = render(Button, mdButton({ type: 'submit' }));
    expect(container.querySelector('button')?.getAttribute('type')).toBe('submit');
  });

  it('renders an <a role="button"> when href is set', () => {
    const { container } = render(Button, mdButton({ href: '/login', variant: 'ghost' }));
    const link = container.querySelector('a');
    expect(link).not.toBeNull();
    expect(link?.getAttribute('href')).toBe('/login');
    expect(link?.getAttribute('role')).toBe('button');
  });

  it('applies md size classes by default', () => {
    const { container } = render(Button, mdButton());
    const cls = container.querySelector('button')!.className;
    expect(cls).toContain('h-[23px]');
    expect(cls).toContain('px-3');
    expect(cls).toContain('text-[9px]');
  });

  it('applies sm size classes when size="sm"', () => {
    const { container } = render(Button, mdButton({ size: 'sm' }));
    const cls = container.querySelector('button')!.className;
    expect(cls).toContain('h-[19px]');
    expect(cls).toContain('px-2');
    expect(cls).toContain('text-[8.5px]');
  });

  it('primary variant uses accent-gradient background classes', () => {
    const { container } = render(Button, mdButton({ variant: 'primary' }));
    const cls = container.querySelector('button')!.className;
    expect(cls).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
    expect(cls).toContain('text-[var(--text-inverted)]');
  });

  it('ghost variant uses transparent bg + border-default', () => {
    const { container } = render(Button, mdButton({ variant: 'ghost' }));
    const cls = container.querySelector('button')!.className;
    expect(cls).toContain('bg-transparent');
    expect(cls).toContain('border-[var(--border-default)]');
    expect(cls).toContain('text-[var(--text-primary)]');
  });

  it('danger variant uses error tokens including new hover tokens', () => {
    const { container } = render(Button, mdButton({ variant: 'danger' }));
    const cls = container.querySelector('button')!.className;
    expect(cls).toContain('bg-[var(--color-error-bg)]');
    expect(cls).toContain('border-[var(--color-error-border)]');
    expect(cls).toContain('text-[var(--color-error)]');
    expect(cls).toContain('hover:bg-[var(--color-error-bg-hover)]');
    expect(cls).toContain('hover:border-[var(--color-error-border-hover)]');
  });

  it('sets disabled attr when disabled prop is true', () => {
    const { container } = render(Button, mdButton({ disabled: true }));
    expect(container.querySelector('button')?.hasAttribute('disabled')).toBe(true);
  });

  it('sets disabled + aria-busy when loading=true', () => {
    const { container } = render(Button, mdButton({ loading: true }));
    const btn = container.querySelector('button')!;
    expect(btn.hasAttribute('disabled')).toBe(true);
    expect(btn.getAttribute('aria-busy')).toBe('true');
  });

  it('swaps leadingIcon for an animate-spin spinner when loading=true', () => {
    const leadingIcon = textSnippet('ICON');
    const { container } = render(Button, mdButton({ loading: true, leadingIcon }));
    const btn = container.querySelector('button')!;
    expect(btn.querySelector('.animate-spin')).not.toBeNull();
    expect(btn.innerHTML).not.toContain('ICON');
  });

  it('does not fire consumer onclick when loading=true', async () => {
    const onclick = vi.fn();
    const { container } = render(Button, mdButton({ loading: true, onclick }));
    await fireEvent.click(container.querySelector('button')!);
    expect(onclick).not.toHaveBeenCalled();
  });

  it('does not fire consumer onclick when disabled=true', async () => {
    const onclick = vi.fn();
    const { container } = render(Button, mdButton({ disabled: true, onclick }));
    await fireEvent.click(container.querySelector('button')!);
    expect(onclick).not.toHaveBeenCalled();
  });

  it('fires consumer onclick in normal state', async () => {
    const onclick = vi.fn();
    const { container } = render(Button, mdButton({ onclick }));
    await fireEvent.click(container.querySelector('button')!);
    expect(onclick).toHaveBeenCalledTimes(1);
  });

  it('link branch sets aria-disabled when disabled + preventDefault on click', async () => {
    const { container } = render(
      Button,
      mdButton({ variant: 'ghost', href: '/x', disabled: true })
    );
    const link = container.querySelector('a')!;
    expect(link.getAttribute('aria-disabled')).toBe('true');
    const event = new MouseEvent('click', { bubbles: true, cancelable: true });
    link.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
  });

  it('link branch preventDefaults Enter/Space keydown when loading', () => {
    const { container } = render(
      Button,
      mdButton({ variant: 'ghost', href: '/x', loading: true })
    );
    const link = container.querySelector('a')!;
    const enter = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
    link.dispatchEvent(enter);
    expect(enter.defaultPrevented).toBe(true);
    const space = new KeyboardEvent('keydown', { key: ' ', bubbles: true, cancelable: true });
    link.dispatchEvent(space);
    expect(space.defaultPrevented).toBe(true);
  });

  it('concatenates consumer class after internal classes', () => {
    const { container } = render(Button, mdButton({ class: 'extra-marker' }));
    const cls = container.querySelector('button')!.className;
    expect(cls).toContain('extra-marker');
    expect(cls).toContain('h-[23px]');
  });

  it('rejects invalid prop combinations at the TS level', () => {
    // These assignments exist only to document the discriminated-union contract.
    // `@ts-expect-error` forces the test file to fail type-check if the union ever
    // loosens to accept these shapes.
    const children = textSnippet('x');
    // @ts-expect-error — href + type must not coexist
    const _bad1: ButtonProps = { variant: 'primary', href: '/x', type: 'submit', children };
    // @ts-expect-error — href + onclick must not coexist
    const _bad2: ButtonProps = { variant: 'primary', href: '/x', onclick: () => {}, children };
    void _bad1;
    void _bad2;
  });
});
```

- [ ] **Step 2: Run the test — expect it to pass straight through**

Run: `cd frontend && npx vitest run src/lib/components/Button.test.ts`
Expected: all assertions pass. The `@ts-expect-error` assertions in the final `it` validate the discriminated-union design; the test file only
compiles if Vitest's TS transform also enforces those errors.

- [ ] **Step 3: If the two `@ts-expect-error` lines fail to register as errors**

Run: `cd frontend && npx tsc --noEmit -p tsconfig.json 2>&1 | grep -E 'Button\.test\.ts.*TS[0-9]+'`
Expected: no output. If output shows "Unused '@ts-expect-error'", the discriminated union is broken — stop and re-check `ButtonProps` in Task 2.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/components/Button.test.ts
git commit -m "test(ui): cover Button primitive variants, sizes, and TS gates (sub-spec #2 PR1)"
```

---

### Task 4: Build `UpdateAllButton.svelte` + `UpdateAllButton.test.ts`

**Files:**

- Create: `frontend/src/lib/components/UpdateAllButton.svelte`
- Create: `frontend/src/lib/components/UpdateAllButton.test.ts`

- [ ] **Step 1: Write the component file**

Write `frontend/src/lib/components/UpdateAllButton.svelte`:

```svelte
<script lang="ts" module>
  import type { Snippet } from 'svelte';
  import type { MouseEventHandler } from 'svelte/elements';

  export type UpdateAllState = 'idle' | 'dim';

  export type UpdateAllButtonProps = {
    state: UpdateAllState;
    count?: number;
    onclick: MouseEventHandler<HTMLButtonElement>;
    ariaLabel?: string;
    children?: Snippet;
    class?: string;
  };
</script>

<script lang="ts">
  const BASE =
    'inline-flex items-center gap-1.5 h-[19px] px-2 rounded-[3px] ' +
    'text-[8.5px] font-bold uppercase tracking-wide ' +
    'transition-[background,border-color,color] duration-[0.12s] ' +
    'active:opacity-[0.88]';

  const STATE_CLASSES: Record<UpdateAllState, string> = {
    idle:
      'bg-[rgba(var(--accent-rgb),0.06)] ' +
      'border border-[rgba(var(--accent-rgb),0.20)] ' +
      'text-[var(--accent)] ' +
      'hover:bg-[rgba(var(--accent-rgb),0.18)] ' +
      'hover:border-[rgba(var(--accent-rgb),0.45)] ' +
      'hover:text-[var(--accent-bright)]',
    dim:
      'bg-transparent ' +
      'border border-[var(--border-default)] ' +
      'text-[var(--text-muted)] ' +
      'pointer-events-none'
  };

  let {
    state,
    count,
    onclick,
    ariaLabel,
    children,
    class: className = ''
  }: UpdateAllButtonProps = $props();

  const computedClass = $derived(
    [BASE, STATE_CLASSES[state], className].filter(Boolean).join(' ')
  );

  const isDim = $derived(state === 'dim');

  function handleKeydown(e: KeyboardEvent) {
    if (isDim && (e.key === 'Enter' || e.key === ' ')) e.preventDefault();
  }
</script>

{#snippet defaultLabel()}↑ Update all{/snippet}

<button
  type="button"
  aria-disabled={isDim || undefined}
  aria-label={ariaLabel}
  class={computedClass}
  onclick={isDim ? undefined : onclick}
  onkeydown={handleKeydown}
>
  {#if children}
    {@render children()}
  {:else}
    {@render defaultLabel()}
  {/if}
  {#if count !== undefined}
    &nbsp;·&nbsp;{count}
  {/if}
</button>
```

- [ ] **Step 2: Write the test file**

Write `frontend/src/lib/components/UpdateAllButton.test.ts`:

```ts
import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import UpdateAllButton from './UpdateAllButton.svelte';

function noop() {}

describe('UpdateAllButton primitive', () => {
  it('renders <button type="button">', () => {
    const { container } = render(UpdateAllButton, { state: 'idle', onclick: noop });
    const btn = container.querySelector('button');
    expect(btn).not.toBeNull();
    expect(btn?.getAttribute('type')).toBe('button');
  });

  it('renders "↑ Update all" as default children', () => {
    const { container } = render(UpdateAllButton, { state: 'idle', onclick: noop });
    expect(container.querySelector('button')!.textContent).toContain('↑ Update all');
  });

  it('appends " · {count}" when count is provided', () => {
    const { container } = render(UpdateAllButton, { state: 'idle', onclick: noop, count: 7 });
    const text = container.querySelector('button')!.textContent!.replace(/\s+/g, ' ').trim();
    expect(text).toBe('↑ Update all · 7');
  });

  it('renders custom children when provided', () => {
    const children = createRawSnippet(() => ({ render: () => '<span>CUSTOM</span>' }));
    const { container } = render(UpdateAllButton, { state: 'idle', onclick: noop, children });
    expect(container.querySelector('button')!.textContent).toContain('CUSTOM');
  });

  it('applies idle-state classes including accent-rgb backgrounds', () => {
    const { container } = render(UpdateAllButton, { state: 'idle', onclick: noop });
    const cls = container.querySelector('button')!.className;
    expect(cls).toContain('bg-[rgba(var(--accent-rgb),0.06)]');
    expect(cls).toContain('border-[rgba(var(--accent-rgb),0.20)]');
    expect(cls).toContain('text-[var(--accent)]');
    expect(cls).toContain('hover:bg-[rgba(var(--accent-rgb),0.18)]');
    expect(cls).toContain('hover:text-[var(--accent-bright)]');
  });

  it('applies dim-state classes including pointer-events-none', () => {
    const { container } = render(UpdateAllButton, { state: 'dim', onclick: noop });
    const cls = container.querySelector('button')!.className;
    expect(cls).toContain('bg-transparent');
    expect(cls).toContain('border-[var(--border-default)]');
    expect(cls).toContain('text-[var(--text-muted)]');
    expect(cls).toContain('pointer-events-none');
  });

  it('sets aria-disabled="true" in dim state', () => {
    const { container } = render(UpdateAllButton, { state: 'dim', onclick: noop });
    expect(container.querySelector('button')!.getAttribute('aria-disabled')).toBe('true');
  });

  it('omits aria-disabled attr in idle state', () => {
    const { container } = render(UpdateAllButton, { state: 'idle', onclick: noop });
    expect(container.querySelector('button')!.hasAttribute('aria-disabled')).toBe(false);
  });

  it('passes ariaLabel through when provided', () => {
    const { container } = render(UpdateAllButton, {
      state: 'dim',
      onclick: noop,
      ariaLabel: 'No updates available'
    });
    expect(container.querySelector('button')!.getAttribute('aria-label')).toBe(
      'No updates available'
    );
  });

  it('does not fire onclick in dim state under pointer activation', async () => {
    const onclick = vi.fn();
    const { container } = render(UpdateAllButton, { state: 'dim', onclick });
    await fireEvent.click(container.querySelector('button')!);
    expect(onclick).not.toHaveBeenCalled();
  });

  it('preventDefaults Enter/Space keydown in dim state', () => {
    const { container } = render(UpdateAllButton, { state: 'dim', onclick: noop });
    const btn = container.querySelector('button')!;
    const enter = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
    btn.dispatchEvent(enter);
    expect(enter.defaultPrevented).toBe(true);
    const space = new KeyboardEvent('keydown', { key: ' ', bubbles: true, cancelable: true });
    btn.dispatchEvent(space);
    expect(space.defaultPrevented).toBe(true);
  });

  it('fires onclick in idle state', async () => {
    const onclick = vi.fn();
    const { container } = render(UpdateAllButton, { state: 'idle', onclick });
    await fireEvent.click(container.querySelector('button')!);
    expect(onclick).toHaveBeenCalledTimes(1);
  });

  it('concatenates consumer class after internal classes', () => {
    const { container } = render(UpdateAllButton, {
      state: 'idle',
      onclick: noop,
      class: 'extra-marker'
    });
    const cls = container.querySelector('button')!.className;
    expect(cls).toContain('extra-marker');
    expect(cls).toContain('h-[19px]');
  });
});
```

- [ ] **Step 3: Run the suite — expect all to pass**

Run: `cd frontend && npx vitest run src/lib/components/UpdateAllButton.test.ts`
Expected: all tests pass.

- [ ] **Step 4: Lint + format**

Run: `cd frontend && npx prettier --check src/lib/components/UpdateAllButton.svelte src/lib/components/UpdateAllButton.test.ts && npx eslint
src/lib/components/UpdateAllButton.svelte src/lib/components/UpdateAllButton.test.ts`
Expected: pass. Apply `--write` if format fails, re-run.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/components/UpdateAllButton.svelte frontend/src/lib/components/UpdateAllButton.test.ts
git commit -m "feat(ui): add UpdateAllButton primitive (sub-spec #2 PR1)"
```

---

### Task 5: Build `terminal-palette.ts` + `terminal-palette.test.ts`

**Files:**

- Create: `frontend/src/theme/terminal-palette.ts`
- Create: `frontend/src/theme/terminal-palette.test.ts`

- [ ] **Step 1: Write the palette module**

Write `frontend/src/theme/terminal-palette.ts`:

```ts
import type { ITheme } from '@xterm/xterm';
import { tokens } from './tokens';

const SUCCESS = tokens['--color-success'].dark;
const ACCENT_BRIGHT = tokens['--accent-bright'].dark;
const MUTED = tokens['--text-muted'].dark;
const PRIMARY = tokens['--text-primary'].dark;
const INVERTED = tokens['--text-inverted'].dark;
const INFO = tokens['--color-info'].dark;

// ANSI-only colors — not part of design language, kept local.
const TERM_BG = '#0c0c0e';
const TERM_FG = '#d4d4d8';
const SELECTION = '#3f3f46';
const ANSI_BLACK = '#18181b';
const ANSI_RED = '#f87171';
const ANSI_BLUE = '#60a5fa';
const ANSI_MAGENTA = '#c084fc';
const ANSI_BRIGHT_RED = '#fb7185';
const ANSI_BRIGHT_GREEN = '#86efac';
const ANSI_BRIGHT_YELLOW = '#fde68a';
const ANSI_BRIGHT_BLUE = '#93c5fd';
const ANSI_BRIGHT_MAGENTA = '#d8b4fe';

// Parent spec §6 pins terminal yellow at `#fcd34d` (progress / in-flight
// layers). This is distinct from `--color-warning` dark (`#fbbf24`) by
// design — terminal amber sits higher on the ramp for readability.
const TERMINAL_AMBER = '#fcd34d';

export const TERMINAL_THEME: ITheme = {
  background: TERM_BG,
  foreground: TERM_FG,
  cursor: TERM_FG,
  selectionBackground: SELECTION,
  black: ANSI_BLACK,
  red: ANSI_RED,
  green: SUCCESS,
  yellow: TERMINAL_AMBER,
  blue: ANSI_BLUE,
  magenta: ANSI_MAGENTA,
  cyan: ACCENT_BRIGHT,
  white: PRIMARY,
  brightBlack: MUTED,
  brightRed: ANSI_BRIGHT_RED,
  brightGreen: ANSI_BRIGHT_GREEN,
  brightYellow: ANSI_BRIGHT_YELLOW,
  brightBlue: ANSI_BRIGHT_BLUE,
  brightMagenta: ANSI_BRIGHT_MAGENTA,
  brightCyan: INFO,
  brightWhite: INVERTED
};
```

- [ ] **Step 2: Write the test file (expect failure until module exists — it already exists from step 1, so this test passes straight through)**

Write `frontend/src/theme/terminal-palette.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { TERMINAL_THEME } from './terminal-palette';
import { tokens } from './tokens';

describe('TERMINAL_THEME bindings to tokens.ts (parent spec §6)', () => {
  it('brightBlack — timestamps / layer IDs — uses --text-muted dark', () => {
    expect(TERMINAL_THEME.brightBlack).toBe(tokens['--text-muted'].dark);
  });

  it('cyan — uptrakit annotations — uses --accent-bright dark', () => {
    expect(TERMINAL_THEME.cyan).toBe(tokens['--accent-bright'].dark);
  });

  it('brightWhite — Docker status lines — uses --text-inverted dark', () => {
    expect(TERMINAL_THEME.brightWhite).toBe(tokens['--text-inverted'].dark);
  });

  it('green — success lines — uses --color-success dark', () => {
    expect(TERMINAL_THEME.green).toBe(tokens['--color-success'].dark);
  });

  it('white — default text — uses --text-primary dark', () => {
    expect(TERMINAL_THEME.white).toBe(tokens['--text-primary'].dark);
  });

  it('brightCyan — bright info — uses --color-info dark', () => {
    expect(TERMINAL_THEME.brightCyan).toBe(tokens['--color-info'].dark);
  });

  it('yellow — terminal amber — pins #fcd34d per §6 (distinct from --color-warning)', () => {
    expect(TERMINAL_THEME.yellow).toBe('#fcd34d');
    expect(TERMINAL_THEME.yellow).not.toBe(tokens['--color-warning'].dark);
  });

  it('background — #0c0c0e — always-dark body per §6', () => {
    expect(TERMINAL_THEME.background).toBe('#0c0c0e');
  });

  it('snapshot: full TERMINAL_THEME object', () => {
    expect(TERMINAL_THEME).toMatchInlineSnapshot(`
{
  "background": "#0c0c0e",
  "black": "#18181b",
  "blue": "#60a5fa",
  "brightBlack": "#52525b",
  "brightBlue": "#93c5fd",
  "brightCyan": "#67e8f9",
  "brightGreen": "#86efac",
  "brightMagenta": "#d8b4fe",
  "brightRed": "#fb7185",
  "brightWhite": "#fafafa",
  "brightYellow": "#fde68a",
  "cursor": "#d4d4d8",
  "cyan": "#22d3ee",
  "foreground": "#d4d4d8",
  "green": "#4ade80",
  "magenta": "#c084fc",
  "red": "#f87171",
  "selectionBackground": "#3f3f46",
  "white": "#e4e4e7",
  "yellow": "#fcd34d",
}
`);
  });
});
```

Note on snapshot ordering: Vitest's `toMatchInlineSnapshot` sorts object keys alphabetically when the input is a plain object. The snapshot above
reflects that alphabetical ordering. If the first run of vitest rewrites the snapshot with a different ordering, accept the rewrite (`npx vitest run
-u`) — source-of-truth is the `TERMINAL_THEME` literal in `terminal-palette.ts`.

- [ ] **Step 3: Run the suite**

Run: `cd frontend && npx vitest run src/theme/terminal-palette.test.ts`
Expected: all 8 binding assertions pass, inline snapshot matches (or is auto-written on first run).

- [ ] **Step 4: Lint + format**

Run: `cd frontend && npx prettier --check src/theme/terminal-palette.ts src/theme/terminal-palette.test.ts && npx eslint src/theme/terminal-palette.ts
src/theme/terminal-palette.test.ts`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/theme/terminal-palette.ts frontend/src/theme/terminal-palette.test.ts
git commit -m "feat(theme): extract terminal palette bound to tokens.ts (sub-spec #2 PR1)"
```

---

### Task 6: Run the full frontend gate + open PR1

**Files:** none (CI sanity).

- [ ] **Step 1: Full frontend gate**

Run:

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: every command exits 0. If `npm run check` flags the new components, fix reported errors before continuing — do not suppress warnings.

- [ ] **Step 2: Push and open PR1**

```bash
git push
gh pr create --title "feat(frontend): add Button + UpdateAllButton primitives + terminal-palette module (sub-spec #2 PR 1)" --body "$(cat <<'EOF'
## Summary

- Add two danger-hover tokens: `--color-error-bg-hover`, `--color-error-border-hover`.
- Add `Button.svelte` primitive with primary/ghost/danger variants, sm/md sizes, loading + disabled gating, polymorphic `href` branch.
- Add `UpdateAllButton.svelte` with idle/dim states (dim uses `pointer-events-none` + `aria-disabled`, blocks keyboard activation).
- Add `terminal-palette.ts` module binding xterm slots to `tokens.ts` dark values.

Part 1 of sub-spec #2. Pure addition — zero consumer touched. Canary migration ships in PR2.

## Test plan

- [x] `cd frontend && npm run lint`
- [x] `cd frontend && npm run format:check`
- [x] `cd frontend && npm run check`
- [x] `cd frontend && npm run test`
- [x] `cd frontend && npm run build`

Playwright e2e unchanged (no call site touched in this PR).

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Expected: PR URL returned. Record it for PR2 cross-linking.

---

## PR2 — Canary migration + terminal consumer switch

> **Important:** Do not start PR2 until PR1 is merged to main. PR2's canary
> migrations import the primitives from `$lib/components/Button` and
> `$lib/components/UpdateAllButton`, which only exist once PR1 lands.

### Task 7: Swap `TerminalOutput.svelte` to the palette module

**Files:**

- Modify: `frontend/src/lib/components/TerminalOutput.svelte`
- Modify: `frontend/src/lib/components/TerminalOutput.test.ts`

- [ ] **Step 1: Confirm current inline literal location**

Run: `cd frontend && grep -n "TERMINAL_THEME" src/lib/components/TerminalOutput.svelte`
Expected: matches on line 75 (`const TERMINAL_THEME = {`) and wherever the identifier is consumed by `new Terminal({ theme: TERMINAL_THEME })`.

- [ ] **Step 2: Replace the inline literal with an import**

Edit `frontend/src/lib/components/TerminalOutput.svelte`:

Near the top of the `<script lang="ts">` block (with the existing `import` lines), add:

```ts
import { TERMINAL_THEME } from '../../theme/terminal-palette';
```

Delete the entire inline literal (lines 75-96 of the current file):

```ts
const TERMINAL_THEME = {
  background: '#0c0c0e',
  foreground: '#d4d4d8',
  cursor: '#d4d4d8',
  selectionBackground: '#3f3f46',
  black: '#18181b',
  red: '#f87171',
  green: '#4ade80',
  yellow: '#fcd34d',
  blue: '#60a5fa',
  magenta: '#c084fc',
  cyan: '#22d3ee',
  white: '#e4e4e7',
  brightBlack: '#3f3f46',
  brightRed: '#fb7185',
  brightGreen: '#86efac',
  brightYellow: '#fde68a',
  brightBlue: '#93c5fd',
  brightMagenta: '#d8b4fe',
  brightCyan: '#67e8f9',
  brightWhite: '#fafafa'
};
```

Do not touch any other code in the file.

- [ ] **Step 3: Extend `TerminalOutput.test.ts` with the reference-identity assertion**

Locate the existing mock for `@xterm/xterm` in `frontend/src/lib/components/TerminalOutput.test.ts` (search for `vi.mock('@xterm/xterm'`). It captures
Terminal constructor arguments already (the existing tests verify lifecycle).

Add a new `it` block inside the existing `describe` that exercises xterm mounting, after the last existing assertion on the mocked constructor:

```ts
it('passes the same TERMINAL_THEME reference from the module to xterm', async () => {
  const { TERMINAL_THEME } = await import('../../theme/terminal-palette');
  // `terminalCtorMock` is the shared mock captured by the existing `vi.mock` block.
  // If the existing test file names it differently, use the local name.
  const ctorArg = terminalCtorMock.mock.calls.at(-1)?.[0];
  expect(ctorArg?.theme).toBe(TERMINAL_THEME);
});
```

If the existing test file does not already define `terminalCtorMock`, wire it: the mocked `Terminal` constructor is captured via `vi.fn()` at the top
of the mock. Rename the local capture to `terminalCtorMock` and reuse it.

If the existing tests name the mock variable `terminalMock` or similar, use that name without renaming — do not invent a new global.

- [ ] **Step 4: Run the extended TerminalOutput suite — expect pass**

Run: `cd frontend && npx vitest run src/lib/components/TerminalOutput.test.ts`
Expected: all existing tests still pass + the new `it` passes.

- [ ] **Step 5: Format + lint**

Run: `cd frontend && npx prettier --check src/lib/components/TerminalOutput.svelte src/lib/components/TerminalOutput.test.ts && npx eslint
src/lib/components/TerminalOutput.svelte src/lib/components/TerminalOutput.test.ts`
Expected: pass.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/components/TerminalOutput.svelte frontend/src/lib/components/TerminalOutput.test.ts
git commit -m "refactor(ui): consume TERMINAL_THEME from terminal-palette module (sub-spec #2 PR2)"
```

---

### Task 8: Migrate `/profile` "Save changes" button

**Files:**

- Modify: `frontend/src/routes/profile/+page.svelte`

**Rationale:** Canary for `variant="primary" type="submit" loading={…}`.

- [ ] **Step 1: Inspect the existing Save button**

Run: `cd frontend && grep -n 'Save changes\|preset-filled-primary\|preset-filled' src/routes/profile/+page.svelte`
Expected: finds the existing `<button class="btn preset-filled-primary-500"> Save changes </button>` around the form submit (exact line varies).

- [ ] **Step 2: Replace the inline `<button>` with `<Button>`**

Add to the top of the `<script lang="ts">` block:

```ts
import Button from '$lib/components/Button.svelte';
```

Locate the existing "Save changes" button — it uses `class="btn preset-filled-primary-500"` with `type="submit"` and is disabled during `loading`.
Replace exactly that markup with:

```svelte
<Button variant="primary" type="submit" loading={loading}>Save changes</Button>
```

Preserve surrounding markup (form, labels, layout).

- [ ] **Step 3: Run profile-related tests — expect pass**

Run: `cd frontend && npx vitest run src/routes/profile`
Expected: pass if tests exist; "no tests found" is acceptable — Playwright covers behaviour.

- [ ] **Step 4: Format + lint**

Run: `cd frontend && npx prettier --check src/routes/profile/+page.svelte && npx eslint src/routes/profile/+page.svelte`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/routes/profile/+page.svelte
git commit -m "refactor(profile): migrate Save button to Button primitive (sub-spec #2 PR2)"
```

---

### Task 9: Migrate `ConfirmDialog` Cancel button

**Files:**

- Modify: `frontend/src/lib/components/ConfirmDialog.svelte`

**Rationale:** Canary for `variant="ghost"` on a plain `<button>`.

- [ ] **Step 1: Inspect the existing Cancel button**

Run: `cd frontend && grep -n 'preset-tonal-surface\|Cancel\|onclick={oncancel}' src/lib/components/ConfirmDialog.svelte`
Expected: current line 59 is `<button class="btn preset-tonal-surface" onclick={oncancel}>Cancel</button>`.

- [ ] **Step 2: Replace the Cancel button**

Add to `<script lang="ts">` imports:

```ts
import Button from './Button.svelte';
```

Replace the existing line:

```svelte
<button class="btn preset-tonal-surface" onclick={oncancel}>Cancel</button>
```

with:

```svelte
<Button variant="ghost" onclick={oncancel}>Cancel</Button>
```

Do not touch the Confirm button — it stays inline for now (its variant is consumer-driven through `confirmClass` and will migrate in sub-spec #3).

- [ ] **Step 3: Run ConfirmDialog tests**

Run: `cd frontend && npx vitest run src/lib/components/ConfirmDialog`
Expected: existing tests pass. If a test asserts the exact class `preset-tonal-surface` on the Cancel button, adjust the assertion to check for
`variant="ghost"` rendered output (presence of `border-[var(--border-default)]`).

- [ ] **Step 4: Format + lint + commit**

```bash
cd frontend && npx prettier --check src/lib/components/ConfirmDialog.svelte && npx eslint src/lib/components/ConfirmDialog.svelte
cd ..
git add frontend/src/lib/components/ConfirmDialog.svelte
git commit -m "refactor(ui): migrate ConfirmDialog Cancel to Button primitive (sub-spec #2 PR2)"
```

---

### Task 10: Migrate `PublicEntryShell` "Back to login" link

**Files:**

- Modify: `frontend/src/lib/components/ui/PublicEntryShell.svelte`

**Rationale:** Canary for the `href` polymorphic branch.

- [ ] **Step 1: Inspect the existing link**

Run: `cd frontend && grep -n 'href\|Back to login\|/login' src/lib/components/ui/PublicEntryShell.svelte`
Expected: finds an `<a href="/login">` rendering "Back to login" (or similar label).

- [ ] **Step 2: Replace the link**

Add to `<script lang="ts">` imports:

```ts
import Button from '../Button.svelte';
```

Replace the existing anchor markup (the "Back to login" link specifically — there may be other links in the shell) with:

```svelte
<Button variant="ghost" href="/login">Back to login</Button>
```

- [ ] **Step 3: Format + lint + test run**

Run:

```bash
cd frontend && npx prettier --check src/lib/components/ui/PublicEntryShell.svelte && npx eslint src/lib/components/ui/PublicEntryShell.svelte
cd frontend && npx vitest run src/lib/components/ui/PublicEntryShell 2>&1 | tail -20
```

Expected: both pass; vitest reports "no tests found" or passes if tests exist.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/components/ui/PublicEntryShell.svelte
git commit -m "refactor(ui): migrate PublicEntryShell back-to-login to Button (sub-spec #2 PR2)"
```

---

### Task 11: Migrate `EnrollmentTokenSettings` first "Revoke" action

**Files:**

- Modify: `frontend/src/routes/settings/EnrollmentTokenSettings.svelte`

**Rationale:** Canary for `variant="danger" size="sm" leadingIcon={…}`.

- [ ] **Step 1: Inspect the existing Revoke button**

Run: `cd frontend && grep -n 'Revoke\|preset-tonal-error\|preset-filled-error' src/routes/settings/EnrollmentTokenSettings.svelte`
Expected: finds the per-row Revoke button using `class="btn btn-sm preset-tonal-error"`.

- [ ] **Step 2: Replace the first Revoke button**

Add to `<script lang="ts">` imports:

```ts
import Button from '$lib/components/Button.svelte';
```

The existing Revoke button markup is something like:

```svelte
<button class="btn btn-sm preset-tonal-error" onclick={() => revoke(token.id)}>
  Revoke
</button>
```

Replace with a Button call that includes a `leadingIcon` snippet. Place the snippet alongside the markup:

```svelte
{#snippet revokeIcon()}
  <span aria-hidden="true">×</span>
{/snippet}

<Button
  variant="danger"
  size="sm"
  leadingIcon={revokeIcon}
  onclick={() => revoke(token.id)}
>
  Revoke
</Button>
```

The leading-icon glyph `×` is intentional — parent spec §4.3 danger pattern uses a multiplication-sign close symbol for destructive row actions. A
proper icon component arrives in a later sub-spec; this canary validates the `leadingIcon` snippet slot compiles + renders.

Only migrate the first Revoke button. Other Revoke instances in the same file remain untouched — sub-spec #3 handles the remaining 244 sites.

- [ ] **Step 3: Format + lint + commit**

```bash
cd frontend && npx prettier --check src/routes/settings/EnrollmentTokenSettings.svelte && npx eslint src/routes/settings/EnrollmentTokenSettings.svelte
cd ..
git add frontend/src/routes/settings/EnrollmentTokenSettings.svelte
git commit -m "refactor(settings): migrate first Revoke to Button danger primitive (sub-spec #2 PR2)"
```

---

### Task 12: Migrate `software/+page.svelte` "Update all" header control

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte`

**Rationale:** Canary for `UpdateAllButton` — both `idle` and `dim` branches exercised through the reactive `pendingCount` expression.

- [ ] **Step 1: Find the existing Update-all header control**

Run: `cd frontend && grep -n 'Update all\|pendingCount\|update-all\|preset-tonal-primary\|preset-filled-primary' src/routes/software/+page.svelte |
head -20`
Expected: finds the header-row Update-all button. It likely already uses a pending/empty branching (two distinct markup paths) or a single button with
`disabled={pendingCount === 0}`.

- [ ] **Step 2: Determine the existing onclick + count expressions**

Inspect the block in an editor or via Read tool. Capture the exact `onclick` handler and the expression currently driving the disabled / empty state.
Typically the handler is `onclick={() => triggerUpdateAll(...)}` and the pending expression is `pendingCount > 0`.

- [ ] **Step 3: Replace the header control**

Add to imports:

```ts
import UpdateAllButton from '$lib/components/UpdateAllButton.svelte';
```

Replace the existing Update-all markup (including any `{#if pendingCount > 0}…{:else}…{/if}` split) with a single call:

```svelte
<UpdateAllButton
  state={pendingCount > 0 ? 'idle' : 'dim'}
  count={pendingCount > 0 ? pendingCount : undefined}
  ariaLabel={pendingCount > 0 ? undefined : 'No updates available'}
  onclick={triggerUpdateAll}
/>
```

If the local handler name is not `triggerUpdateAll`, use whichever name the existing onclick already references. If `pendingCount` is named
differently (e.g. `updatableCount`), use the existing name unchanged.

- [ ] **Step 4: Format + lint**

Run: `cd frontend && npx prettier --check src/routes/software/+page.svelte && npx eslint src/routes/software/+page.svelte`
Expected: pass.

- [ ] **Step 5: Run component test suite touching software page**

Run: `cd frontend && npx vitest run src/routes/software 2>&1 | tail -20`
Expected: pass. If any unit test asserts the inline Update-all markup contained specific classes like `preset-tonal-primary`, adapt the assertion to
the new component's rendered classes (e.g. `bg-[rgba(var(--accent-rgb),0.06)]`).

- [ ] **Step 6: Commit**

```bash
git add frontend/src/routes/software/+page.svelte
git commit -m "refactor(software): migrate Update-all header to UpdateAllButton (sub-spec #2 PR2)"
```

---

### Task 13: Add `/dev/button-preview` dev-only route

**Files:**

- Create: `frontend/src/routes/dev/button-preview/+page.svelte`

**Rationale:** Deterministic matrix page for Playwright visual coverage of every variant × size × state combination.

- [ ] **Step 1: Create the dev route**

Write `frontend/src/routes/dev/button-preview/+page.svelte`:

```svelte
<script lang="ts">
  import Button from '$lib/components/Button.svelte';
  import UpdateAllButton from '$lib/components/UpdateAllButton.svelte';
  import type { ButtonVariant, ButtonSize } from '$lib/components/Button.svelte';

  const VARIANTS: ButtonVariant[] = ['primary', 'ghost', 'danger'];
  const SIZES: ButtonSize[] = ['md', 'sm'];

  function noop() {}
</script>

<main class="flex flex-col gap-6 p-6" data-testid="button-preview-root">
  <section data-testid="button-variants">
    <h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Button — variants × sizes</h2>
    <div class="grid grid-cols-2 gap-3" style="width: 420px;">
      {#each VARIANTS as variant}
        {#each SIZES as size}
          <div
            class="flex items-center gap-2"
            data-testid="button-cell-{variant}-{size}"
          >
            <Button {variant} {size} onclick={noop}>Label</Button>
            <span class="text-[9px] uppercase text-[var(--text-muted)]">
              {variant}/{size}
            </span>
          </div>
        {/each}
      {/each}
    </div>
  </section>

  <section data-testid="button-states">
    <h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Button — disabled + loading</h2>
    <div class="flex flex-wrap gap-3">
      <Button variant="primary" disabled onclick={noop}>Disabled</Button>
      <Button variant="primary" loading onclick={noop}>Loading</Button>
      <Button variant="ghost" disabled onclick={noop}>Ghost disabled</Button>
      <Button variant="danger" loading onclick={noop}>Danger loading</Button>
    </div>
  </section>

  <section data-testid="button-link">
    <h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Button — link branch</h2>
    <div class="flex flex-wrap gap-3">
      <Button variant="ghost" href="/login">Href ghost</Button>
      <Button variant="primary" href="/register">Href primary</Button>
      <Button variant="primary" href="/disabled" disabled>Href disabled</Button>
    </div>
  </section>

  <section data-testid="updateall-states">
    <h2 class="mb-3 text-sm font-bold uppercase tracking-wide">UpdateAllButton</h2>
    <div class="flex flex-wrap gap-3">
      <UpdateAllButton state="idle" onclick={noop} />
      <UpdateAllButton state="idle" count={3} onclick={noop} />
      <UpdateAllButton state="dim" ariaLabel="No updates available" onclick={noop} />
    </div>
  </section>
</main>
```

- [ ] **Step 2: Verify the route renders in dev**

Run: `cd frontend && npm run build 2>&1 | tail -10`
Expected: build succeeds. The route ships to the production bundle too; that is acceptable — nothing in the route pulls server-only modules, and
parent spec notes a dev flag is deferred.

- [ ] **Step 3: Format + lint**

Run: `cd frontend && npx prettier --check src/routes/dev/button-preview/+page.svelte && npx eslint src/routes/dev/button-preview/+page.svelte`
Expected: pass.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/dev/button-preview/+page.svelte
git commit -m "chore(dev): add /dev/button-preview route for primitive snapshots (sub-spec #2 PR2)"
```

---

### Task 14: Add `button-primitive.spec.ts` Playwright suite

**Files:**

- Create: `frontend/tests/e2e/button-primitive.spec.ts`

**Rationale:** Visual baseline across both themes for every variant × state permutation.

- [ ] **Step 1: Inspect existing Playwright patterns for parity-style tests**

Run: `cd frontend && head -60 tests/e2e/ui-parity.test.ts`
Note the conventions: theme-toggle helper, screenshot tolerance `threshold: 0.005` (0.5 % per spec §3), locator-based region captures.

- [ ] **Step 2: Write the Playwright suite**

Write `frontend/tests/e2e/button-primitive.spec.ts`:

```ts
import { expect, test } from '@playwright/test';

const ROUTE = '/dev/button-preview';

const SECTIONS = [
  { id: 'button-variants', name: 'variants' },
  { id: 'button-states', name: 'states' },
  { id: 'button-link', name: 'link' },
  { id: 'updateall-states', name: 'updateall' }
];

async function setTheme(page: import('@playwright/test').Page, theme: 'dark' | 'light') {
  await page.addInitScript((t) => {
    if (t === 'dark') document.documentElement.classList.add('dark');
    else document.documentElement.classList.remove('dark');
    try {
      localStorage.setItem('theme', t);
    } catch {
      /* ignore */
    }
  }, theme);
}

test.describe('button primitive preview', () => {
  for (const theme of ['dark', 'light'] as const) {
    test.describe(theme, () => {
      test.beforeEach(async ({ page }) => {
        await setTheme(page, theme);
        await page.goto(ROUTE);
        await page.waitForSelector('[data-testid="button-preview-root"]');
      });

      for (const section of SECTIONS) {
        test(`${section.name} snapshot`, async ({ page }) => {
          const region = page.locator(`[data-testid="${section.id}"]`);
          await expect(region).toHaveScreenshot(
            `${theme}-${section.name}.png`,
            { threshold: 0.005 }
          );
        });
      }
    });
  }
});
```

- [ ] **Step 3: Install Chromium if missing (one-time)**

Run: `cd frontend && npx playwright install --with-deps chromium`
Expected: installation succeeds; re-running says "already installed".

- [ ] **Step 4: Generate baseline snapshots (macOS + Chromium only)**

Run: `cd frontend && npx playwright test tests/e2e/button-primitive.spec.ts --update-snapshots`
Expected: 8 snapshots written under `frontend/tests/e2e/button-primitive.spec.ts-snapshots/`. If the test host is not macOS + Chromium, stop and
switch hosts — parent spec pins snapshot regeneration to `macOS + Chromium` per `frontend/playwright.config.ts`.

- [ ] **Step 5: Re-run to confirm stability**

Run: `cd frontend && npx playwright test tests/e2e/button-primitive.spec.ts`
Expected: 8/8 pass.

- [ ] **Step 6: Commit**

```bash
git add frontend/tests/e2e/button-primitive.spec.ts frontend/tests/e2e/button-primitive.spec.ts-snapshots
git commit -m "test(e2e): baseline Button + UpdateAllButton preview snapshots (sub-spec #2 PR2)"
```

---

### Task 15: Re-baseline canary route Playwright snapshots + full gate + open PR2

**Files:**

- Modify: `frontend/tests/e2e/ui-parity.test.ts-snapshots/` (regenerated bytes)
- Modify: `frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots/` (regenerated bytes, if any touch the five canary scenes)

**Rationale:** The five canary call sites now render the primitives' gradient / ghost / danger / update-all classes. Existing snapshots assume the
preset-* classes and will fail visual diff. Parent spec §9 requires each delta be enumerated in the PR description.

- [ ] **Step 1: Run the full e2e suite to see the pre-regen failures**

Run: `cd frontend && npx playwright test 2>&1 | tail -30`
Expected: 5 failures (profile, confirm-dialog, public-entry shell back-to-login, settings enrollment-tokens first revoke, software update-all).
Capture the failing test ids for the PR body.

- [ ] **Step 2: Regenerate only the affected snapshots**

Run:

```bash
cd frontend && npx playwright test --update-snapshots --grep "profile|confirm|public-entry|enrollment|software|update all"
```

Expected: the failing snapshots are overwritten. Verify by `git status frontend/tests/e2e/` — only the expected five scenes (at most) should appear in
diff.

- [ ] **Step 3: Run the full e2e suite — expect all pass**

Run: `cd frontend && npx playwright test 2>&1 | tail -10`
Expected: 0 failures.

- [ ] **Step 4: Full frontend gate**

Run:

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: every command exits 0.

- [ ] **Step 5: Commit snapshot updates**

```bash
git add frontend/tests/e2e/ui-parity.test.ts-snapshots frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots
git commit -m "test(e2e): re-baseline five canary scenes after Button/UpdateAllButton migration (sub-spec #2 PR2)"
```

- [ ] **Step 6: Push + open PR2**

```bash
git push
gh pr create --title "feat(frontend): migrate canary call sites and terminal palette consumer (sub-spec #2 PR 2)" --body "$(cat <<'EOF'
## Summary

- `TerminalOutput.svelte` consumes `TERMINAL_THEME` from the palette module (one ref, no value change visible to users; `brightBlack` shifts from `#3f3f46` → `#52525b` as spec'd).
- Five canary call sites migrated to the new primitives:
  - `/profile` Save button → `<Button variant="primary" type="submit" loading>`
  - `ConfirmDialog` Cancel → `<Button variant="ghost">`
  - `PublicEntryShell` back-to-login → `<Button variant="ghost" href>`
  - `/settings` first Revoke → `<Button variant="danger" size="sm" leadingIcon>`
  - `/software` Update-all header → `<UpdateAllButton state count ariaLabel>`
- `/dev/button-preview` route added for deterministic primitive snapshots.
- `button-primitive.spec.ts` adds 8 Playwright baselines (4 sections × 2 themes).

## Deliberate visual deltas (per parent spec §9 waiver schema)

Each delta is the intended adoption of sub-spec #2 primitives. No waivers needed.

- Profile Save button adopts primary gradient: 9 px weight-700, 23 px height, `var(--accent-deep) → var(--accent)` gradient per §4.3.
- ConfirmDialog Cancel adopts ghost variant: transparent bg, `var(--border-default)` border.
- PublicEntryShell back-to-login adopts ghost `<a role="button">`, pointer-events blocked when disabled (n/a here).
- EnrollmentTokenSettings first Revoke adopts danger sm: `var(--color-error-bg)` idle, `var(--color-error-bg-hover)` hover.
- Software page Update-all adopts accent-rgb translucent idle / border-default dim states.
- `TerminalOutput` modal's `brightBlack` ANSI slot shifts `#3f3f46` → `#52525b` (§6 binding to `--text-muted`).

## Test plan

- [x] `cd frontend && npm run lint`
- [x] `cd frontend && npm run format:check`
- [x] `cd frontend && npm run check`
- [x] `cd frontend && npm run test`
- [x] `cd frontend && npm run build`
- [x] `cd frontend && npm run test:e2e` (macOS + Chromium)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Expected: PR URL returned.

---

## Definition of done (whole plan)

1. PR1 merged on main with two new tokens + three new primitives + six new test files. All `npm run *` gates green.
2. PR2 merged on main with terminal-palette consumer switch, five canary migrations, dev preview route, new Playwright baseline, re-baselined canary
   snapshots. All `npm run *` gates green including `test:e2e`.
3. Spec's Non-goals respected: no migration beyond the five canary sites; no `ariaLabel` prop on `Button`; no Storybook harness beyond
   `/dev/button-preview`.
4. `TerminalOutput.svelte` imports `TERMINAL_THEME` from `terminal-palette.ts`; the only value-level change observable in tests is the `brightBlack`
   hex shift, intentional per §6 binding table.
