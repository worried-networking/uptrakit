# Button Primitive Updates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `--bg-hover` token, `secondary` `ButtonVariant`, and `ariaLabel` prop to the Button primitive; re-baseline the button-preview
Playwright snapshot.

**Architecture:** Purely additive changes — extend the existing `TokenName` union and `tokens` record, extend `VARIANT_CLASSES` and `CommonProps`,
wire `aria-label` to both render branches. `UpdateAllButton` requires no change (already emits `aria-label` natively on its standalone `<button>`).

**Tech Stack:** Svelte 5, TypeScript, Tailwind v4, Vitest, Playwright

---

## File structure

> **Path note:** The design spec incorrectly references these files under `src/lib/theme/`. The actual paths are under `src/theme/` (no `lib/`
> segment). The plan below uses the correct paths. The spec also mentions `adapter-manifest.test.ts` — this file does not exist in the codebase; skip
> that step.

| Path | Change |
| --- | --- |
| `frontend/src/theme/tokens.ts` | Add `'--bg-hover'` to `TokenName` union and `tokens` record |
| `frontend/src/theme/tokens.test.ts` | Add `'--bg-hover'` entry to `EXPECTED` constant |
| `frontend/src/lib/components/Button.svelte` | Add `secondary` to `ButtonVariant`; add `ariaLabel?: string` to `CommonProps`; add `VARIANT_CLASSES.secondary`; wire `aria-label` to both render branches |
| `frontend/src/lib/components/Button.test.ts` | Mirror updated types locally; add 6 new tests (secondary class fragments, secondary+disabled, secondary active, ariaLabel on button + link branches, omit when absent, `@ts-expect-error` for tertiary) |
| `frontend/src/lib/components/UpdateAllButton.test.ts` | No change — `ariaLabel` non-regression test already exists at line 65; verify it still passes |
| `frontend/src/routes/dev/button-preview/+page.svelte` | Add `'secondary'` to `VARIANTS`; add `<section data-testid="button-arialabel">` |
| `frontend/tests/e2e/button-primitive.spec.ts` | Add `button-arialabel` to `SECTIONS`; re-baseline affected snapshots |

## Commit plan

| # | Files | Message |
| --- | --- | --- |
| 1 | `tokens.ts`, `tokens.test.ts` | `feat(tokens): add --bg-hover token for secondary button hover state` |
| 2 | `Button.svelte`, `Button.test.ts` | `feat(button): add secondary variant and ariaLabel prop` |
| 3 | `+page.svelte` (preview), `button-primitive.spec.ts`, snapshot PNGs | `feat(button-preview): add secondary row + arialabel section; re-baseline snapshots` |

---

## Task 1: Add `--bg-hover` token (TDD — test first)

**Files:**

- Modify: `frontend/src/theme/tokens.ts`
- Modify: `frontend/src/theme/tokens.test.ts`

### Current `tokens.test.ts` structure

The file defines `const EXPECTED: Record<TokenName, Record<Theme, string>>` — a compile-time exhaustive map. Adding `'--bg-hover'` to `TokenName`
will make the TypeScript compiler error on this constant until the entry is added.

- [ ] **Step 1: Extend `EXPECTED` in `tokens.test.ts` (RED)**

In `frontend/src/theme/tokens.test.ts`, add `'--bg-hover'` after the `'--bg-raised'` entry (line 7):

```ts
'--bg-raised': { dark: '#18181b', light: '#f1f5f9' },
'--bg-hover':  { dark: '#1e1e22', light: '#eef1f5' },   // ← new
'--border-subtle': { dark: '#1c1c1f', light: '#e2e8f0' },
```

Run tests — expect TypeScript compile error (`'--bg-hover'` not yet in `TokenName`):

```bash
cd frontend && npx vitest run src/theme/tokens.test.ts
```

Expected: type error or test failure.

- [ ] **Step 2: Add `--bg-hover` to `tokens.ts` (GREEN)**

In `frontend/src/theme/tokens.ts`:

**Change 1 — `TokenName` union** (after `'--bg-raised'`):

```ts
export type TokenName =
  | '--bg-base'
  | '--bg-surface'
  | '--bg-raised'
  | '--bg-hover'         // ← new
  | '--border-subtle'
  | '--border-default'
  // ... rest unchanged
```

**Change 2 — `tokens` record** (after the `'--bg-raised'` entry):

```ts
'--bg-raised': { dark: '#18181b', light: '#f1f5f9' },
'--bg-hover':  { dark: '#1e1e22', light: '#eef1f5' },   // ← new
'--border-subtle': { dark: '#1c1c1f', light: '#e2e8f0' },
```

Run tests — all pass:

```bash
cd frontend && npx vitest run src/theme/tokens.test.ts
```

Expected: all 6 tests pass.

- [ ] **Step 3: Commit 1**

```bash
git add frontend/src/theme/tokens.ts frontend/src/theme/tokens.test.ts
git commit -m "feat(tokens): add --bg-hover token for secondary button hover state"
```

---

## Task 2: Add `secondary` variant and `ariaLabel` prop to Button (TDD — tests first)

**Files:**

- Modify: `frontend/src/lib/components/Button.svelte`
- Modify: `frontend/src/lib/components/Button.test.ts`

### Current Button.svelte (reference)

```svelte
<script lang="ts" module>
  export type ButtonVariant = 'primary' | 'ghost' | 'danger';
  // CommonProps has: variant, size, disabled, loading, leadingIcon, trailingIcon, children, class
  // ButtonProps is a discriminated union: href branch | button/onclick branch
</script>

<script lang="ts">
  const BASE = 'inline-flex items-center gap-1.5 rounded-[3px] font-bold uppercase tracking-wide ' +
    'transition-[background,border-color,color] duration-[0.12s] ' +
    'disabled:opacity-40 disabled:pointer-events-none ' +
    'aria-disabled:opacity-40 aria-disabled:pointer-events-none ' +
    'active:opacity-[0.88]';
  // VARIANT_CLASSES: Record<ButtonVariant, string> = { primary, ghost, danger }
  // $props() destructures: variant, size, disabled, loading, leadingIcon, trailingIcon,
  //   children, class:className, href, type, onclick
  // Both render branches: <a> and <button>
</script>
```

- [ ] **Step 1: Extend `Button.test.ts` (RED)**

In `frontend/src/lib/components/Button.test.ts` make four changes:

**A — Update local `ButtonVariant` type (line 10):**

```ts
type ButtonVariant = 'primary' | 'ghost' | 'danger' | 'secondary';
```

**B — Add `ariaLabel?: string` to local `CommonProps`** (before `children: Snippet`):

```ts
type CommonProps = {
	variant: ButtonVariant;
	size?: ButtonSize;
	disabled?: boolean;
	loading?: boolean;
	leadingIcon?: Snippet;
	trailingIcon?: Snippet;
	ariaLabel?: string;   // ← new
	children: Snippet;
	class?: string;
};
```

**C — Add four new tests** at the end of the `describe` block, before the closing `});`:

```ts
it('secondary variant uses bg-raised + bg-hover on hover', () => {
	const { container } = render(Button, mdButton({ variant: 'secondary' }));
	const cls = container.querySelector('button')!.className;
	expect(cls).toContain('bg-[var(--bg-raised)]');
	expect(cls).toContain('border-[var(--border-default)]');
	expect(cls).toContain('text-[var(--text-primary)]');
	expect(cls).toContain('hover:bg-[var(--bg-hover)]');
});

it('ariaLabel prop sets aria-label on the button branch', () => {
	const { container } = render(Button, mdButton({ ariaLabel: 'Close dialog' }));
	expect(container.querySelector('button')!.getAttribute('aria-label')).toBe('Close dialog');
});

it('ariaLabel prop sets aria-label on the link branch', () => {
	const { container } = render(Button, mdButton({ variant: 'ghost', href: '/x', ariaLabel: 'Go home' }));
	expect(container.querySelector('a')!.getAttribute('aria-label')).toBe('Go home');
});

it('omits aria-label when ariaLabel prop is not provided', () => {
	const { container } = render(Button, mdButton());
	expect(container.querySelector('button')!.hasAttribute('aria-label')).toBe(false);
});

it('secondary variant + disabled carries opacity-40 (no per-variant override sneaking in)', () => {
	const { container } = render(Button, mdButton({ variant: 'secondary', disabled: true }));
	const cls = container.querySelector('button')!.className;
	expect(cls).toContain('disabled:opacity-40');
	// Regression guard: secondary VARIANT_CLASSES must not add a conflicting disabled rule
	expect(cls).not.toMatch(/disabled:opacity-(?!40)/);
});

it('secondary variant contains active:opacity-[0.88] class fragment', () => {
	const { container } = render(Button, mdButton({ variant: 'secondary' }));
	const cls = container.querySelector('button')!.className;
	expect(cls).toContain('active:opacity-[0.88]');
});
```

**D — Add `@ts-expect-error` for `'tertiary'`** inside the existing `'rejects invalid prop combinations at the TS level'` test, after the two
existing `@ts-expect-error` lines:

```ts
// @ts-expect-error — 'tertiary' is not a valid ButtonVariant
const _bad3: ButtonProps = { variant: 'tertiary', children };
void _bad3;
```

Run — expect 6 new test failures:

```bash
cd frontend && npx vitest run src/lib/components/Button.test.ts
```

Expected: 6 new tests FAIL; all 18 existing tests PASS.

- [ ] **Step 2: Update `Button.svelte` (GREEN)**

Make five changes to `frontend/src/lib/components/Button.svelte`:

**Change 1 — `ButtonVariant` in module script:**

```ts
export type ButtonVariant = 'primary' | 'ghost' | 'danger' | 'secondary';
```

**Change 2 — `ariaLabel?: string` in `CommonProps`** (before `children: Snippet`):

```ts
type CommonProps = {
	variant: ButtonVariant;
	size?: ButtonSize;
	disabled?: boolean;
	loading?: boolean;
	leadingIcon?: Snippet;
	trailingIcon?: Snippet;
	ariaLabel?: string;   // ← new
	children: Snippet;
	class?: string;
};
```

**Change 3 — Add `secondary` to `VARIANT_CLASSES`** (full record after adding the entry):

```ts
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
		'hover:border-[var(--color-error-border-hover)]',
	secondary:
		'bg-[var(--bg-raised)] border border-[var(--border-default)] ' +
		'text-[var(--text-primary)] ' +
		'hover:bg-[var(--bg-hover)] ' +
		'active:opacity-[0.88]'
};
```

> `active:opacity-[0.88]` is already in `BASE` so it applies to all variants — including it in `secondary` makes the variant's full class contract
> self-documenting.

**Change 4 — Add `ariaLabel` to `$props()` destructure** (before `children`, mirroring `CommonProps` field order):

```ts
let {
	variant,
	size = 'md',
	disabled = false,
	loading = false,
	leadingIcon,
	trailingIcon,
	ariaLabel,      // ← new — before children, matching CommonProps order
	children,
	class: className = '',
	href,
	type,
	onclick
}: ButtonProps = $props();
```

**Change 5 — Wire `aria-label` to both render branches:**

In the `<a>` branch, add `aria-label={ariaLabel ?? undefined}`:

```svelte
<a
	{href}
	role="button"
	aria-disabled={inert || undefined}
	aria-busy={loading || undefined}
	aria-label={ariaLabel ?? undefined}
	onclick={handleLinkClick}
	onkeydown={handleLinkKeydown}
	class={computedClass}
>
```

In the `<button>` branch, add `aria-label={ariaLabel ?? undefined}`:

```svelte
<button
	type={type ?? 'button'}
	disabled={inert}
	aria-busy={loading || undefined}
	aria-label={ariaLabel ?? undefined}
	class={computedClass}
	onclick={inert ? undefined : onclick}
>
```

Run — all 24 tests pass (18 existing + 6 new):

```bash
cd frontend && npx vitest run src/lib/components/Button.test.ts
```

- [ ] **Step 3: Verify UpdateAllButton non-regression test still passes**

The test at line 65 of `UpdateAllButton.test.ts` already asserts `ariaLabel` emits `aria-label` on the standalone `<button>`. No change needed — just
verify:

```bash
cd frontend && npx vitest run src/lib/components/UpdateAllButton.test.ts
```

Expected: all 13 tests pass.

- [ ] **Step 4: Full unit suite**

```bash
cd frontend && npx vitest run src/theme/tokens.test.ts src/lib/components/Button.test.ts src/lib/components/UpdateAllButton.test.ts
```

Expected: all pass.

- [ ] **Step 5: Commit 2**

```bash
git add frontend/src/lib/components/Button.svelte frontend/src/lib/components/Button.test.ts
git commit -m "feat(button): add secondary variant and ariaLabel prop"
```

---

## Task 3: Update preview route and re-baseline Playwright snapshots

**Files:**

- Modify: `frontend/src/routes/dev/button-preview/+page.svelte`
- Modify: `frontend/tests/e2e/button-primitive.spec.ts`

- [ ] **Step 1: Add `'secondary'` to `VARIANTS` (line 6 of +page.svelte)**

```ts
const VARIANTS: ButtonVariant[] = ['primary', 'ghost', 'danger', 'secondary'];
```

- [ ] **Step 2: Add `<section data-testid="button-arialabel">` after the `button-link` section**

```svelte
<section data-testid="button-arialabel">
	<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Button — ariaLabel prop</h2>
	<div class="flex flex-wrap gap-3">
		<Button variant="secondary" size="md" onclick={noop} ariaLabel="Confirm action">Confirm</Button>
		<Button variant="secondary" size="sm" onclick={noop} ariaLabel="Cancel action">Cancel</Button>
		<Button variant="ghost" href="/dev/button-preview" ariaLabel="Navigate home">Home</Button>
	</div>
</section>
```

- [ ] **Step 3: Add `button-arialabel` to `SECTIONS` in `button-primitive.spec.ts`**

In `frontend/tests/e2e/button-primitive.spec.ts`, add `button-arialabel` to the `SECTIONS` array after `button-link`:

```ts
const SECTIONS = [
	{ id: 'button-variants', name: 'variants' },
	{ id: 'button-states', name: 'states' },
	{ id: 'button-link', name: 'link' },
	{ id: 'button-arialabel', name: 'arialabel' },   // ← new
	{ id: 'updateall-states', name: 'updateall' }
];
```

- [ ] **Step 4: Delete stale `button-variants` snapshots and re-baseline**

The `button-variants` section now renders 4 rows instead of 3 (secondary added). Delete the stale PNGs then run with `--update-snapshots`:

```bash
rm "frontend/tests/e2e/button-primitive.spec.ts-snapshots/dark-variants.png" \
   "frontend/tests/e2e/button-primitive.spec.ts-snapshots/light-variants.png"
cd frontend && npx playwright test tests/e2e/button-primitive.spec.ts --update-snapshots
```

Expected: regenerates `dark-variants.png`, `light-variants.png`; creates `dark-arialabel.png`, `light-arialabel.png`. Other sections (`states`,
`link`, `updateall`) are unchanged and pass within threshold.

- [ ] **Step 5: Run without `--update-snapshots` to confirm stable**

```bash
cd frontend && npx playwright test tests/e2e/button-primitive.spec.ts
```

Expected: all 10 tests pass (5 sections × 2 themes).

- [ ] **Step 6: Commit 3**

```bash
git add \
  frontend/src/routes/dev/button-preview/+page.svelte \
  frontend/tests/e2e/button-primitive.spec.ts \
  "frontend/tests/e2e/button-primitive.spec.ts-snapshots"
git commit -m "feat(button-preview): add secondary row + arialabel section; re-baseline snapshots"
```

---

## Task 4: Full frontend gate

- [ ] **Step 1: Run full gate**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: every command exits 0.

- [ ] **Step 2: Run full e2e suite**

```bash
cd frontend && npx playwright test 2>&1 | tail -20
```

Expected: 0 failures. `button-primitive.spec.ts` updated baselines pass; all other suites unaffected.
