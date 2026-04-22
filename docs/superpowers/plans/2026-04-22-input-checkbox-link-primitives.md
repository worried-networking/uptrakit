# Input + Checkbox + Link Primitives Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Input, Checkbox, and Link primitives to `$lib/components/` implementing §4.10 field conventions and §2.6 focus-ring rules, with a
Playwright-baselined dev preview route.

**Architecture:** Three thin Svelte 5 wrappers around native elements using `$bindable()` for two-way binding, `$props()` for typed APIs, and CSS
class string composition matching `Button.svelte`'s established pattern. No runtime token imports — all `var(--*)` references are CSS literals
resolved by the browser.

**Tech Stack:** Svelte 5 (runes), TypeScript, Tailwind v4 arbitrary values, Vitest + @testing-library/svelte, Playwright

---

## File structure

| Path | Responsibility |
| --- | --- |
| `frontend/src/lib/components/Input.svelte` | Text-field primitive; bindable value, error/aria-invalid wiring, type matrix |
| `frontend/src/lib/components/Input.test.ts` | Type branch matrix, error/disabled state, event firing, class gates |
| `frontend/src/lib/components/Checkbox.svelte` | Checkbox primitive; bindable checked state |
| `frontend/src/lib/components/Checkbox.test.ts` | Checked/unchecked rendering, disabled state, event firing, class gates |
| `frontend/src/lib/components/Link.svelte` | Anchor primitive; variant × external attribute wiring, snippet children |
| `frontend/src/lib/components/Link.test.ts` | Variant class matrix, external attrs, children rendering, class gates |
| `frontend/src/routes/dev/form-primitive-preview/+page.svelte` | Dev-only gallery rendering every primitive × variant × state in both themes |
| `frontend/tests/e2e/form-primitive.spec.ts` | Playwright snapshots of `/dev/form-primitive-preview` (5 sections × 2 themes = 10 PNGs) |

## Global conventions

- **Svelte 5 runes:** module script exports types; runtime script uses `$props()`, `$derived()`. `$bindable()` is called at destructure time inside
`$props()`.
- **Class concatenation:** `[BASE, className].filter(Boolean).join(' ')` — no clsx, bare string per project convention.
- **Type mirroring in test files:** types used in `.test.ts` are defined locally, NOT imported from the `.svelte` source. `tsc --noEmit` does not
resolve `.svelte` module types in plain `.ts` files; only `svelte-check` does.
- **ESLint key rule:** all `{#each}` loops must carry a key expression: `{#each ITEMS as item (item)}`.
- **Working directory for all commands:** `frontend/` — commands below use `cd frontend &&` prefix.

---

## Task 1: Input.svelte primitive

**Files:**

- Create: `frontend/src/lib/components/Input.svelte`
- Create: `frontend/src/lib/components/Input.test.ts`

- [ ] **Step 1: Write the failing tests**

Create `frontend/src/lib/components/Input.test.ts`:

```ts
import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/svelte';
import Input from './Input.svelte';

// Mirror of InputProps from Input.svelte's module script.
// Defined locally — tsc --noEmit does not resolve svelte module types in plain .ts files.
type InputType = 'text' | 'email' | 'password' | 'url' | 'number' | 'search';
type InputProps = {
	id: string;
	type: InputType;
	value: string;
	name?: string;
	placeholder?: string;
	autocomplete?: string;
	disabled?: boolean;
	required?: boolean;
	error?: string;
	oninput?: (e: Event) => void;
	onblur?: (e: FocusEvent) => void;
	'aria-describedby'?: string;
	class?: string;
};

function baseInput(extra: Partial<InputProps> = {}): InputProps {
	return { id: 'test-input', type: 'text', value: '', ...extra };
}

describe('Input primitive', () => {
	it('renders an <input> element', () => {
		const { container } = render(Input, baseInput());
		expect(container.querySelector('input')).not.toBeNull();
	});

	it('forwards the id prop to the underlying input', () => {
		const { container } = render(Input, baseInput({ id: 'my-field' }));
		expect(container.querySelector('input')!.getAttribute('id')).toBe('my-field');
	});

	it('sets type="text" when type prop is text', () => {
		const { container } = render(Input, baseInput({ type: 'text' }));
		expect(container.querySelector('input')!.getAttribute('type')).toBe('text');
	});

	it('sets type="email" when type prop is email', () => {
		const { container } = render(Input, baseInput({ type: 'email' }));
		expect(container.querySelector('input')!.getAttribute('type')).toBe('email');
	});

	it('sets type="password" when type prop is password', () => {
		const { container } = render(Input, baseInput({ type: 'password' }));
		expect(container.querySelector('input')!.getAttribute('type')).toBe('password');
	});

	it('sets type="url" when type prop is url', () => {
		const { container } = render(Input, baseInput({ type: 'url' }));
		expect(container.querySelector('input')!.getAttribute('type')).toBe('url');
	});

	it('sets type="number" when type prop is number', () => {
		const { container } = render(Input, baseInput({ type: 'number' }));
		expect(container.querySelector('input')!.getAttribute('type')).toBe('number');
	});

	it('sets type="search" when type prop is search', () => {
		const { container } = render(Input, baseInput({ type: 'search' }));
		expect(container.querySelector('input')!.getAttribute('type')).toBe('search');
	});

	it('applies base class tokens including bg-surface and border-default', () => {
		const { container } = render(Input, baseInput());
		const cls = container.querySelector('input')!.className;
		expect(cls).toContain('h-8');
		expect(cls).toContain('w-full');
		expect(cls).toContain('bg-[var(--bg-surface)]');
		expect(cls).toContain('border-[var(--border-default)]');
		expect(cls).toContain('text-sm');
		expect(cls).toContain('text-[var(--text-primary)]');
	});

	it('applies focus-visible ring class', () => {
		const { container } = render(Input, baseInput());
		const cls = container.querySelector('input')!.className;
		expect(cls).toContain('focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]');
	});

	it('applies disabled utility classes in class string', () => {
		const { container } = render(Input, baseInput());
		const cls = container.querySelector('input')!.className;
		expect(cls).toContain('disabled:opacity-40');
		expect(cls).toContain('disabled:cursor-not-allowed');
	});

	it('applies aria-invalid error classes in class string', () => {
		const { container } = render(Input, baseInput());
		const cls = container.querySelector('input')!.className;
		expect(cls).toContain('aria-[invalid=true]:border-[var(--color-error-border)]');
		expect(cls).toContain('aria-[invalid=true]:bg-[var(--color-error-bg)]');
	});

	it('sets aria-invalid="true" when error prop is non-empty', () => {
		const { container } = render(Input, baseInput({ error: 'Required field' }));
		expect(container.querySelector('input')!.getAttribute('aria-invalid')).toBe('true');
	});

	it('omits aria-invalid attribute when error is undefined', () => {
		const { container } = render(Input, baseInput());
		expect(container.querySelector('input')!.hasAttribute('aria-invalid')).toBe(false);
	});

	it('omits aria-invalid attribute when error is empty string', () => {
		const { container } = render(Input, baseInput({ error: '' }));
		expect(container.querySelector('input')!.hasAttribute('aria-invalid')).toBe(false);
	});

	it('sets disabled attribute when disabled prop is true', () => {
		const { container } = render(Input, baseInput({ disabled: true }));
		expect(container.querySelector('input')!.hasAttribute('disabled')).toBe(true);
	});

	it('sets required attribute when required prop is true', () => {
		const { container } = render(Input, baseInput({ required: true }));
		expect(container.querySelector('input')!.hasAttribute('required')).toBe(true);
	});

	it('forwards placeholder to the input element', () => {
		const { container } = render(Input, baseInput({ placeholder: 'Enter email' }));
		expect(container.querySelector('input')!.getAttribute('placeholder')).toBe('Enter email');
	});

	it('forwards name to the input element', () => {
		const { container } = render(Input, baseInput({ name: 'email' }));
		expect(container.querySelector('input')!.getAttribute('name')).toBe('email');
	});

	it('forwards autocomplete to the input element', () => {
		const { container } = render(Input, baseInput({ autocomplete: 'email' }));
		expect(container.querySelector('input')!.getAttribute('autocomplete')).toBe('email');
	});

	it('forwards aria-describedby to the input element', () => {
		const { container } = render(Input, baseInput({ 'aria-describedby': 'hint-id' }));
		expect(container.querySelector('input')!.getAttribute('aria-describedby')).toBe('hint-id');
	});

	it('omits aria-describedby when not provided', () => {
		const { container } = render(Input, baseInput());
		expect(container.querySelector('input')!.hasAttribute('aria-describedby')).toBe(false);
	});

	it('fires oninput callback on input event', async () => {
		const oninput = vi.fn();
		const { container } = render(Input, baseInput({ oninput }));
		await fireEvent.input(container.querySelector('input')!);
		expect(oninput).toHaveBeenCalledTimes(1);
	});

	it('fires onblur callback on blur event', async () => {
		const onblur = vi.fn();
		const { container } = render(Input, baseInput({ onblur }));
		await fireEvent.blur(container.querySelector('input')!);
		expect(onblur).toHaveBeenCalledTimes(1);
	});

	it('concatenates consumer class after internal classes', () => {
		const { container } = render(Input, baseInput({ class: 'extra-marker' }));
		const cls = container.querySelector('input')!.className;
		expect(cls).toContain('extra-marker');
		expect(cls).toContain('h-8');
	});

	it('rejects invalid type at the TS level', () => {
		// @ts-expect-error — 'date' is not a valid InputType
		const _bad: InputProps = { id: 'x', type: 'date', value: '' };
		void _bad;
	});
});
```

- [ ] **Step 2: Run the tests — expect failures (Input.svelte not yet created)**

```bash
cd frontend && npx vitest run src/lib/components/Input.test.ts
```

Expected: FAIL — module not found.

- [ ] **Step 3: Create Input.svelte**

Create `frontend/src/lib/components/Input.svelte`:

```svelte
<script lang="ts" module>
	export type InputType = 'text' | 'email' | 'password' | 'url' | 'number' | 'search';

	export type InputProps = {
		id: string;
		type: InputType;
		value: string;
		name?: string;
		placeholder?: string;
		autocomplete?: string;
		disabled?: boolean;
		required?: boolean;
		error?: string;
		oninput?: (e: Event) => void;
		onblur?: (e: FocusEvent) => void;
		'aria-describedby'?: string;
		class?: string;
	};
</script>

<script lang="ts">
	const BASE =
		'h-8 w-full px-[10px] rounded-[3px] ' +
		'bg-[var(--bg-surface)] border border-[var(--border-default)] ' +
		'text-sm text-[var(--text-primary)] ' +
		'placeholder:text-[var(--text-muted)] ' +
		'focus-visible:outline-none ' +
		'focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] ' +
		'disabled:opacity-40 disabled:cursor-not-allowed ' +
		'aria-[invalid=true]:border-[var(--color-error-border)] ' +
		'aria-[invalid=true]:bg-[var(--color-error-bg)] ' +
		'transition-[background,border-color] duration-[0.12s]';

	let {
		id,
		type,
		value = $bindable(''),
		name,
		placeholder,
		autocomplete,
		disabled = false,
		required = false,
		error,
		oninput,
		onblur,
		'aria-describedby': ariaDescribedby,
		class: className = ''
	}: InputProps = $props();

	const computedClass = $derived([BASE, className].filter(Boolean).join(' '));
	const hasError = $derived(!!error);
</script>

<input
	{id}
	{type}
	bind:value
	{name}
	{placeholder}
	{autocomplete}
	{disabled}
	{required}
	{oninput}
	{onblur}
	aria-invalid={hasError ? 'true' : undefined}
	aria-describedby={ariaDescribedby}
	class={computedClass}
/>
```

- [ ] **Step 4: Run the tests — expect all to pass**

```bash
cd frontend && npx vitest run src/lib/components/Input.test.ts
```

Expected: all assertions pass.

- [ ] **Step 5: Svelte type-check**

```bash
cd frontend && npm run check 2>&1 | grep -E 'Input\.svelte'
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/components/Input.svelte frontend/src/lib/components/Input.test.ts
git commit -m "feat(ui): add Input primitive with type matrix, error/aria-invalid wiring (sub-spec #2b)"
```

---

## Task 2: Checkbox.svelte primitive

**Files:**

- Create: `frontend/src/lib/components/Checkbox.svelte`
- Create: `frontend/src/lib/components/Checkbox.test.ts`

- [ ] **Step 1: Write the failing tests**

Create `frontend/src/lib/components/Checkbox.test.ts`:

```ts
import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/svelte';
import Checkbox from './Checkbox.svelte';

// Mirror of CheckboxProps from Checkbox.svelte's module script.
// Defined locally — tsc --noEmit does not resolve svelte module types in plain .ts files.
type CheckboxProps = {
	id: string;
	checked: boolean;
	name?: string;
	disabled?: boolean;
	onchange?: (e: Event) => void;
	class?: string;
};

function baseCheckbox(extra: Partial<CheckboxProps> = {}): CheckboxProps {
	return { id: 'test-checkbox', checked: false, ...extra };
}

describe('Checkbox primitive', () => {
	it('renders an <input type="checkbox"> element', () => {
		const { container } = render(Checkbox, baseCheckbox());
		expect(container.querySelector('input[type="checkbox"]')).not.toBeNull();
	});

	it('forwards the id prop to the underlying input', () => {
		const { container } = render(Checkbox, baseCheckbox({ id: 'accept-terms' }));
		expect(container.querySelector('input')!.getAttribute('id')).toBe('accept-terms');
	});

	it('is unchecked when checked=false', () => {
		const { container } = render(Checkbox, baseCheckbox({ checked: false }));
		expect((container.querySelector('input') as HTMLInputElement).checked).toBe(false);
	});

	it('is checked when checked=true', () => {
		const { container } = render(Checkbox, baseCheckbox({ checked: true }));
		expect((container.querySelector('input') as HTMLInputElement).checked).toBe(true);
	});

	it('applies base class tokens including size, border, and accent', () => {
		const { container } = render(Checkbox, baseCheckbox());
		const cls = container.querySelector('input')!.className;
		expect(cls).toContain('h-4');
		expect(cls).toContain('w-4');
		expect(cls).toContain('rounded-[2px]');
		expect(cls).toContain('border-[var(--border-default)]');
		expect(cls).toContain('accent-[var(--accent)]');
	});

	it('applies focus-visible ring class', () => {
		const { container } = render(Checkbox, baseCheckbox());
		const cls = container.querySelector('input')!.className;
		expect(cls).toContain('focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]');
	});

	it('applies disabled utility classes in class string', () => {
		const { container } = render(Checkbox, baseCheckbox());
		const cls = container.querySelector('input')!.className;
		expect(cls).toContain('disabled:opacity-40');
		expect(cls).toContain('disabled:cursor-not-allowed');
	});

	it('sets disabled attribute when disabled prop is true', () => {
		const { container } = render(Checkbox, baseCheckbox({ disabled: true }));
		expect(container.querySelector('input')!.hasAttribute('disabled')).toBe(true);
	});

	it('forwards name to the input element', () => {
		const { container } = render(Checkbox, baseCheckbox({ name: 'accept' }));
		expect(container.querySelector('input')!.getAttribute('name')).toBe('accept');
	});

	it('omits name when not provided', () => {
		const { container } = render(Checkbox, baseCheckbox());
		expect(container.querySelector('input')!.hasAttribute('name')).toBe(false);
	});

	it('fires onchange callback on change event', async () => {
		const onchange = vi.fn();
		const { container } = render(Checkbox, baseCheckbox({ onchange }));
		await fireEvent.change(container.querySelector('input')!);
		expect(onchange).toHaveBeenCalledTimes(1);
	});

	it('concatenates consumer class after internal classes', () => {
		const { container } = render(Checkbox, baseCheckbox({ class: 'extra-marker' }));
		const cls = container.querySelector('input')!.className;
		expect(cls).toContain('extra-marker');
		expect(cls).toContain('h-4');
	});
});
```

- [ ] **Step 2: Run the tests — expect failures (Checkbox.svelte not yet created)**

```bash
cd frontend && npx vitest run src/lib/components/Checkbox.test.ts
```

Expected: FAIL — module not found.

- [ ] **Step 3: Create Checkbox.svelte**

Create `frontend/src/lib/components/Checkbox.svelte`:

```svelte
<script lang="ts" module>
	export type CheckboxProps = {
		id: string;
		checked: boolean;
		name?: string;
		disabled?: boolean;
		onchange?: (e: Event) => void;
		class?: string;
	};
</script>

<script lang="ts">
	const BASE =
		'h-4 w-4 rounded-[2px] ' +
		'border border-[var(--border-default)] ' +
		'accent-[var(--accent)] ' +
		'focus-visible:outline-none ' +
		'focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] ' +
		'disabled:opacity-40 disabled:cursor-not-allowed';

	let {
		id,
		checked = $bindable(false),
		name,
		disabled = false,
		onchange,
		class: className = ''
	}: CheckboxProps = $props();

	const computedClass = $derived([BASE, className].filter(Boolean).join(' '));
</script>

<input
	type="checkbox"
	{id}
	bind:checked
	{name}
	{disabled}
	{onchange}
	class={computedClass}
/>
```

- [ ] **Step 4: Run the tests — expect all to pass**

```bash
cd frontend && npx vitest run src/lib/components/Checkbox.test.ts
```

Expected: all assertions pass.

- [ ] **Step 5: Svelte type-check**

```bash
cd frontend && npm run check 2>&1 | grep -E 'Checkbox\.svelte'
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/components/Checkbox.svelte frontend/src/lib/components/Checkbox.test.ts
git commit -m "feat(ui): add Checkbox primitive with bindable checked, disabled gating (sub-spec #2b)"
```

---

## Task 3: Link.svelte primitive

**Files:**

- Create: `frontend/src/lib/components/Link.svelte`
- Create: `frontend/src/lib/components/Link.test.ts`

- [ ] **Step 1: Write the failing tests**

Create `frontend/src/lib/components/Link.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { render } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import Link from './Link.svelte';

// Mirror of LinkProps from Link.svelte's module script.
// Defined locally — tsc --noEmit does not resolve svelte module types in plain .ts files.
type LinkVariant = 'default' | 'muted' | 'danger';
type LinkProps = {
	href: string;
	variant?: LinkVariant;
	external?: boolean;
	children: ReturnType<typeof createRawSnippet>;
	class?: string;
};

function textSnippet(value: string) {
	return createRawSnippet(() => ({ render: () => `<span>${value}</span>` }));
}

function baseLink(extra: Partial<LinkProps> = {}): LinkProps {
	return { href: '/test', children: textSnippet('Click me'), ...extra };
}

describe('Link primitive', () => {
	it('renders an <a> element', () => {
		const { container } = render(Link, baseLink());
		expect(container.querySelector('a')).not.toBeNull();
	});

	it('forwards the href prop to the anchor', () => {
		const { container } = render(Link, baseLink({ href: '/dashboard' }));
		expect(container.querySelector('a')!.getAttribute('href')).toBe('/dashboard');
	});

	it('renders children snippet content', () => {
		const { container } = render(Link, baseLink({ children: textSnippet('Go there') }));
		expect(container.querySelector('a')!.textContent).toContain('Go there');
	});

	it('defaults to the "default" variant', () => {
		const { container } = render(Link, baseLink());
		const cls = container.querySelector('a')!.className;
		expect(cls).toContain('text-[var(--accent)]');
		expect(cls).toContain('hover:text-[var(--accent-bright)]');
	});

	it('applies base classes on all variants', () => {
		const { container } = render(Link, baseLink());
		const cls = container.querySelector('a')!.className;
		expect(cls).toContain('font-medium');
		expect(cls).toContain('underline');
		expect(cls).toContain('underline-offset-4');
		expect(cls).toContain('focus-visible:outline-none');
		expect(cls).toContain('focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]');
		expect(cls).toContain('transition-colors');
		expect(cls).toContain('duration-[0.12s]');
	});

	it('default variant uses accent color tokens', () => {
		const { container } = render(Link, baseLink({ variant: 'default' }));
		const cls = container.querySelector('a')!.className;
		expect(cls).toContain('text-[var(--accent)]');
		expect(cls).toContain('hover:text-[var(--accent-bright)]');
	});

	it('muted variant uses text-muted and text-primary tokens', () => {
		const { container } = render(Link, baseLink({ variant: 'muted' }));
		const cls = container.querySelector('a')!.className;
		expect(cls).toContain('text-[var(--text-muted)]');
		expect(cls).toContain('hover:text-[var(--text-primary)]');
	});

	it('danger variant uses color-error token', () => {
		const { container } = render(Link, baseLink({ variant: 'danger' }));
		const cls = container.querySelector('a')!.className;
		expect(cls).toContain('text-[var(--color-error)]');
		expect(cls).toContain('hover:opacity-80');
	});

	it('omits target and rel when external is false (default)', () => {
		const { container } = render(Link, baseLink({ external: false }));
		const a = container.querySelector('a')!;
		expect(a.hasAttribute('target')).toBe(false);
		expect(a.hasAttribute('rel')).toBe(false);
	});

	it('sets target="_blank" and rel="noopener noreferrer" when external=true', () => {
		const { container } = render(Link, baseLink({ external: true }));
		const a = container.querySelector('a')!;
		expect(a.getAttribute('target')).toBe('_blank');
		expect(a.getAttribute('rel')).toBe('noopener noreferrer');
	});

	it('concatenates consumer class after internal classes', () => {
		const { container } = render(Link, baseLink({ class: 'extra-marker' }));
		const cls = container.querySelector('a')!.className;
		expect(cls).toContain('extra-marker');
		expect(cls).toContain('font-medium');
	});

	it('rejects invalid variant at the TS level', () => {
		// @ts-expect-error — 'info' is not a valid LinkVariant
		const _bad: LinkProps = { href: '/x', variant: 'info', children: textSnippet('x') };
		void _bad;
	});
});
```

- [ ] **Step 2: Run the tests — expect failures (Link.svelte not yet created)**

```bash
cd frontend && npx vitest run src/lib/components/Link.test.ts
```

Expected: FAIL — module not found.

- [ ] **Step 3: Create Link.svelte**

Create `frontend/src/lib/components/Link.svelte`:

```svelte
<script lang="ts" module>
	import type { Snippet } from 'svelte';

	export type LinkVariant = 'default' | 'muted' | 'danger';

	export type LinkProps = {
		href: string;
		variant?: LinkVariant;
		external?: boolean;
		children: Snippet;
		class?: string;
	};
</script>

<script lang="ts">
	const BASE =
		'font-medium underline underline-offset-4 ' +
		'focus-visible:outline-none ' +
		'focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] ' +
		'transition-colors duration-[0.12s]';

	const VARIANT_CLASSES: Record<LinkVariant, string> = {
		default: 'text-[var(--accent)] hover:text-[var(--accent-bright)]',
		muted: 'text-[var(--text-muted)] hover:text-[var(--text-primary)]',
		danger: 'text-[var(--color-error)] hover:text-[var(--color-error)] hover:opacity-80'
	};

	let {
		href,
		variant = 'default',
		external = false,
		children,
		class: className = ''
	}: LinkProps = $props();

	const computedClass = $derived([BASE, VARIANT_CLASSES[variant], className].filter(Boolean).join(' '));
</script>

<a
	{href}
	target={external ? '_blank' : undefined}
	rel={external ? 'noopener noreferrer' : undefined}
	class={computedClass}
>
	{@render children()}
</a>
```

- [ ] **Step 4: Run the tests — expect all to pass**

```bash
cd frontend && npx vitest run src/lib/components/Link.test.ts
```

Expected: all assertions pass.

- [ ] **Step 5: Svelte type-check**

```bash
cd frontend && npm run check 2>&1 | grep -E 'Link\.svelte'
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/components/Link.svelte frontend/src/lib/components/Link.test.ts
git commit -m "feat(ui): add Link primitive with variant matrix, external attr wiring (sub-spec #2b)"
```

---

## Task 4: /dev/form-primitive-preview route + Playwright e2e

**Files:**

- Create: `frontend/src/routes/dev/form-primitive-preview/+page.svelte`
- Create: `frontend/tests/e2e/form-primitive.spec.ts`

Read `frontend/src/routes/dev/button-preview/+page.svelte` and `frontend/tests/e2e/button-primitive.spec.ts` before writing these files — follow the
exact same structural patterns (data-testid sections, `{#each}` key expressions, `mockAuthApi` helper).

- [ ] **Step 1: Create the preview route**

Create `frontend/src/routes/dev/form-primitive-preview/+page.svelte`:

```svelte
<script lang="ts">
	import Input from '$lib/components/Input.svelte';
	import Checkbox from '$lib/components/Checkbox.svelte';
	import Link from '$lib/components/Link.svelte';
	import type { InputType } from '$lib/components/Input.svelte';
	import type { LinkVariant } from '$lib/components/Link.svelte';

	const INPUT_TYPES: InputType[] = ['text', 'email', 'password', 'url', 'number', 'search'];
	const LINK_VARIANTS: LinkVariant[] = ['default', 'muted', 'danger'];

	let checkedA = $state(false);
	let checkedB = $state(true);
</script>

<main class="flex flex-col gap-6 p-6" data-testid="form-primitive-preview-root">
	<section data-testid="input-types">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Input — type matrix</h2>
		<div class="flex flex-col gap-3" style="width: 320px;">
			{#each INPUT_TYPES as type (type)}
				<div data-testid="input-cell-{type}">
					<label class="mb-1 block text-xs text-[var(--text-muted)]" for="preview-{type}">
						{type}
					</label>
					<Input id="preview-{type}" {type} value="" placeholder={type} />
				</div>
			{/each}
		</div>
	</section>

	<section data-testid="input-states">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Input — states</h2>
		<div class="flex flex-col gap-3" style="width: 320px;">
			<div data-testid="input-cell-normal">
				<label class="mb-1 block text-xs text-[var(--text-muted)]" for="preview-normal">normal</label>
				<Input id="preview-normal" type="text" value="" placeholder="Normal input" />
			</div>
			<div data-testid="input-cell-disabled">
				<label class="mb-1 block text-xs text-[var(--text-muted)]" for="preview-disabled">disabled</label>
				<Input id="preview-disabled" type="text" value="" placeholder="Disabled input" disabled />
			</div>
			<div data-testid="input-cell-error">
				<label class="mb-1 block text-xs text-[var(--text-muted)]" for="preview-error">error</label>
				<Input id="preview-error" type="text" value="" placeholder="Error input" error="This field is required" />
			</div>
		</div>
	</section>

	<section data-testid="checkbox-states">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Checkbox — states</h2>
		<div class="flex flex-col gap-3">
			<div class="flex items-center gap-2" data-testid="checkbox-cell-unchecked">
				<Checkbox id="preview-checkbox-unchecked" bind:checked={checkedA} />
				<label class="text-sm text-[var(--text-primary)]" for="preview-checkbox-unchecked">Unchecked</label>
			</div>
			<div class="flex items-center gap-2" data-testid="checkbox-cell-checked">
				<Checkbox id="preview-checkbox-checked" bind:checked={checkedB} />
				<label class="text-sm text-[var(--text-primary)]" for="preview-checkbox-checked">Checked</label>
			</div>
			<div class="flex items-center gap-2" data-testid="checkbox-cell-disabled">
				<Checkbox id="preview-checkbox-disabled" checked={false} disabled />
				<label class="text-sm text-[var(--text-primary)]" for="preview-checkbox-disabled">Disabled unchecked</label>
			</div>
			<div class="flex items-center gap-2" data-testid="checkbox-cell-disabled-checked">
				<Checkbox id="preview-checkbox-disabled-checked" checked={true} disabled />
				<label class="text-sm text-[var(--text-primary)]" for="preview-checkbox-disabled-checked">Disabled checked</label>
			</div>
		</div>
	</section>

	<section data-testid="link-variants">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Link — variants</h2>
		<div class="flex flex-wrap gap-4">
			{#each LINK_VARIANTS as variant (variant)}
				<div data-testid="link-cell-{variant}">
					<Link href="/dev/form-primitive-preview" {variant}>{variant} link</Link>
				</div>
			{/each}
		</div>
	</section>

	<section data-testid="link-external">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Link — external</h2>
		<div class="flex flex-wrap gap-4">
			<div data-testid="link-cell-external">
				<Link href="https://example.com" external>External link</Link>
			</div>
			<div data-testid="link-cell-internal">
				<Link href="/dev/form-primitive-preview">Internal link</Link>
			</div>
		</div>
	</section>
</main>
```

- [ ] **Step 2: Verify route builds**

```bash
cd frontend && npm run build 2>&1 | tail -10
```

Expected: build succeeds, no errors.

- [ ] **Step 3: Create the Playwright e2e spec**

Read `frontend/tests/e2e/button-primitive.spec.ts` for the exact `mockAuthApi` pattern. Copy it verbatim into the new spec (all routes require auth —
the mock prevents redirect). Create `frontend/tests/e2e/form-primitive.spec.ts` following the same structure: `SECTIONS` array, two-theme loop,
`toHaveScreenshot` with `threshold: 0.005`.

The 5 sections to snapshot: `input-types`, `input-states`, `checkbox-states`, `link-variants`, `link-external`. That yields 10 baseline PNGs (5 × 2
themes).

- [ ] **Step 4: Install Chromium if not yet installed (one-time)**

```bash
cd frontend && npx playwright install --with-deps chromium
```

Expected: installs or reports already present.

- [ ] **Step 5: Generate baseline snapshots (macOS + Chromium only)**

```bash
cd frontend && npx playwright test tests/e2e/form-primitive.spec.ts --update-snapshots
```

Expected: 10 PNGs written to `frontend/tests/e2e/form-primitive.spec.ts-snapshots/`.

- [ ] **Step 6: Re-run to confirm stability**

```bash
cd frontend && npx playwright test tests/e2e/form-primitive.spec.ts
```

Expected: 10/10 pass with no diff failures.

- [ ] **Step 7: Commit**

```bash
git add \
  frontend/src/routes/dev/form-primitive-preview/+page.svelte \
  frontend/tests/e2e/form-primitive.spec.ts \
  "frontend/tests/e2e/form-primitive.spec.ts-snapshots"
git commit -m "chore(dev): add form-primitive preview route and Playwright baselines (sub-spec #2b)"
```

---

## Task 5: Full frontend gate

- [ ] **Step 1: Run full gate**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: every command exits 0.

- [ ] **Step 2: Run full e2e suite**

```bash
cd frontend && npx playwright test 2>&1 | tail -20
```

Expected: 0 failures. New `form-primitive.spec.ts` passes; existing `button-primitive.spec.ts` and ui-parity suites unaffected (no consumer
migrations in this plan).
