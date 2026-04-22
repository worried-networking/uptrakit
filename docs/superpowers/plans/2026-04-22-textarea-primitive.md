# Textarea Primitive Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `Textarea.svelte` primitive to `$lib/components/` implementing §4.10 field conventions and §2.6
focus-ring rules, with `default`/`mono` variant, `error` prop, `bind:value`, and Playwright snapshot coverage
via the `/dev/form-primitive-preview` route.

**Architecture:** Thin Svelte 5 wrapper around `<textarea>` using `$bindable()`, `$props()`, `$derived()`. Class
string composition identical to `Input.svelte` from sub-spec #2b. The dev preview route and e2e spec — both
created by #2b — are extended with a Textarea section; this plan modifies those two files.

**Tech Stack:** Svelte 5 (runes), TypeScript, Tailwind v4 arbitrary values, Vitest + @testing-library/svelte, Playwright

---

## File structure

| Path | Action | Responsibility |
| --- | --- | --- |
| `frontend/src/lib/components/Textarea.svelte` | Create | Textarea primitive; bindable value, variant × error class composition |
| `frontend/src/lib/components/Textarea.test.ts` | Create | Variant/state matrix, event callbacks, attribute forwarding |
| `frontend/src/routes/dev/form-primitive-preview/+page.svelte` | Modify | Add Textarea section (file created by sub-spec #2b) |
| `frontend/tests/e2e/form-primitive.spec.ts` | Modify | Add textarea snapshot entries (file created by sub-spec #2b) |

**Prerequisite:** sub-spec #2b must be merged first — it creates the preview route and e2e spec that this plan extends.

## Global conventions

- **Svelte 5 runes:** module script exports types; runtime script uses `$props()`, `$derived()`. `$bindable()` is called at destructure time inside `$props()`.
- **Class concatenation:** `[BASE, VARIANT_CLASSES[variant], className].filter(Boolean).join(' ')` — no clsx.
- **Type mirroring in test files:** types used in `.test.ts` are defined locally, NOT imported from the `.svelte`
  source. `tsc --noEmit` does not resolve `.svelte` module types in plain `.ts` files.
- **Working directory for all commands:** `frontend/` — all commands below assume `cd frontend && ...` context.

---

## Task 1: Textarea.svelte primitive

**Files:**

- Create: `frontend/src/lib/components/Textarea.svelte`
- Create: `frontend/src/lib/components/Textarea.test.ts`

- [ ] **Step 1: Write the failing tests**

Create `frontend/src/lib/components/Textarea.test.ts`:

```ts
import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/svelte';
import Textarea from './Textarea.svelte';

// Mirror of TextareaProps from Textarea.svelte's module script.
// Defined locally — tsc --noEmit does not resolve svelte module types in plain .ts files.
type TextareaVariant = 'default' | 'mono';
type TextareaProps = {
	id: string;
	value: string;
	name?: string;
	placeholder?: string;
	rows?: number;
	disabled?: boolean;
	required?: boolean;
	error?: string;
	variant?: TextareaVariant;
	oninput?: (e: Event) => void;
	onblur?: (e: FocusEvent) => void;
	'aria-describedby'?: string;
	class?: string;
};

function base(extra: Partial<TextareaProps> = {}): TextareaProps {
	return { id: 'test-textarea', value: '', ...extra };
}

describe('Textarea primitive', () => {
	it('renders a <textarea> element', () => {
		const { container } = render(Textarea, base());
		expect(container.querySelector('textarea')).not.toBeNull();
	});

	it('forwards the id prop to the underlying textarea', () => {
		const { container } = render(Textarea, base({ id: 'my-field' }));
		expect(container.querySelector('textarea')!.getAttribute('id')).toBe('my-field');
	});

	// --- Base class contract ---

	it('applies base classes: w-full, rounded, padding, bg-surface, border-default', () => {
		const { container } = render(Textarea, base());
		const cls = container.querySelector('textarea')!.className;
		expect(cls).toContain('w-full');
		expect(cls).toContain('rounded-[3px]');
		expect(cls).toContain('p-2');
		expect(cls).toContain('bg-[var(--bg-surface)]');
		expect(cls).toContain('border-[var(--border-default)]');
		expect(cls).toContain('text-sm');
		expect(cls).toContain('text-[var(--text-primary)]');
	});

	it('applies resize-y and min-h in base class', () => {
		const { container } = render(Textarea, base());
		const cls = container.querySelector('textarea')!.className;
		expect(cls).toContain('resize-y');
		expect(cls).toContain('min-h-[4rem]');
	});

	it('applies focus-visible ring class', () => {
		const { container } = render(Textarea, base());
		const cls = container.querySelector('textarea')!.className;
		expect(cls).toContain('focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]');
	});

	it('applies disabled utility classes in class string', () => {
		const { container } = render(Textarea, base());
		const cls = container.querySelector('textarea')!.className;
		expect(cls).toContain('disabled:opacity-40');
		expect(cls).toContain('disabled:cursor-not-allowed');
	});

	it('applies aria-invalid error classes in class string', () => {
		const { container } = render(Textarea, base());
		const cls = container.querySelector('textarea')!.className;
		expect(cls).toContain('aria-[invalid=true]:border-[var(--color-error-border)]');
		expect(cls).toContain('aria-[invalid=true]:bg-[var(--color-error-bg)]');
	});

	// --- Variant matrix ---

	it('default variant: does NOT add font-mono', () => {
		const { container } = render(Textarea, base({ variant: 'default' }));
		expect(container.querySelector('textarea')!.className).not.toContain('font-mono');
	});

	it('mono variant: adds font-mono class', () => {
		const { container } = render(Textarea, base({ variant: 'mono' }));
		expect(container.querySelector('textarea')!.className).toContain('font-mono');
	});

	it('mono variant: adds text-[13px] class', () => {
		const { container } = render(Textarea, base({ variant: 'mono' }));
		expect(container.querySelector('textarea')!.className).toContain('text-[13px]');
	});

	it('variant defaults to default when omitted', () => {
		const { container } = render(Textarea, base());
		expect(container.querySelector('textarea')!.className).not.toContain('font-mono');
	});

	// --- Error / aria-invalid ---

	it('sets aria-invalid="true" when error prop is non-empty', () => {
		const { container } = render(Textarea, base({ error: 'Required' }));
		expect(container.querySelector('textarea')!.getAttribute('aria-invalid')).toBe('true');
	});

	it('omits aria-invalid when error is undefined', () => {
		const { container } = render(Textarea, base());
		expect(container.querySelector('textarea')!.hasAttribute('aria-invalid')).toBe(false);
	});

	it('omits aria-invalid when error is empty string', () => {
		const { container } = render(Textarea, base({ error: '' }));
		expect(container.querySelector('textarea')!.hasAttribute('aria-invalid')).toBe(false);
	});

	// --- Variant × error permutation checks ---
	// 8 permutations: default|mono × error|no-error × disabled|enabled

	it('default + no-error + enabled: base classes present, no font-mono, no aria-invalid', () => {
		const { container } = render(Textarea, base({ variant: 'default', disabled: false }));
		const ta = container.querySelector('textarea')!;
		expect(ta.className).toContain('w-full');
		expect(ta.className).not.toContain('font-mono');
		expect(ta.hasAttribute('aria-invalid')).toBe(false);
		expect(ta.hasAttribute('disabled')).toBe(false);
	});

	it('default + error + enabled: aria-invalid set, no font-mono', () => {
		const { container } = render(Textarea, base({ variant: 'default', error: 'bad' }));
		const ta = container.querySelector('textarea')!;
		expect(ta.getAttribute('aria-invalid')).toBe('true');
		expect(ta.className).not.toContain('font-mono');
	});

	it('default + no-error + disabled: disabled attr set, opacity-40 in class', () => {
		const { container } = render(Textarea, base({ variant: 'default', disabled: true }));
		const ta = container.querySelector('textarea')!;
		expect(ta.hasAttribute('disabled')).toBe(true);
		expect(ta.className).toContain('disabled:opacity-40');
	});

	it('default + error + disabled: both aria-invalid and disabled set', () => {
		const { container } = render(Textarea, base({ variant: 'default', error: 'err', disabled: true }));
		const ta = container.querySelector('textarea')!;
		expect(ta.getAttribute('aria-invalid')).toBe('true');
		expect(ta.hasAttribute('disabled')).toBe(true);
	});

	it('mono + no-error + enabled: font-mono present, no aria-invalid', () => {
		const { container } = render(Textarea, base({ variant: 'mono', disabled: false }));
		const ta = container.querySelector('textarea')!;
		expect(ta.className).toContain('font-mono');
		expect(ta.hasAttribute('aria-invalid')).toBe(false);
	});

	it('mono + error + enabled: font-mono + aria-invalid both present', () => {
		const { container } = render(Textarea, base({ variant: 'mono', error: 'bad' }));
		const ta = container.querySelector('textarea')!;
		expect(ta.className).toContain('font-mono');
		expect(ta.getAttribute('aria-invalid')).toBe('true');
	});

	it('mono + no-error + disabled: font-mono + disabled attr', () => {
		const { container } = render(Textarea, base({ variant: 'mono', disabled: true }));
		const ta = container.querySelector('textarea')!;
		expect(ta.className).toContain('font-mono');
		expect(ta.hasAttribute('disabled')).toBe(true);
	});

	it('mono + error + disabled: font-mono + aria-invalid + disabled all set', () => {
		const { container } = render(Textarea, base({ variant: 'mono', error: 'err', disabled: true }));
		const ta = container.querySelector('textarea')!;
		expect(ta.className).toContain('font-mono');
		expect(ta.getAttribute('aria-invalid')).toBe('true');
		expect(ta.hasAttribute('disabled')).toBe(true);
	});

	// --- Attribute forwarding ---

	it('forwards rows to the textarea element', () => {
		const { container } = render(Textarea, base({ rows: 6 }));
		expect(container.querySelector('textarea')!.getAttribute('rows')).toBe('6');
	});

	it('omits rows attribute when not provided', () => {
		const { container } = render(Textarea, base());
		expect(container.querySelector('textarea')!.hasAttribute('rows')).toBe(false);
	});

	it('forwards required attribute when required prop is true', () => {
		const { container } = render(Textarea, base({ required: true }));
		expect(container.querySelector('textarea')!.hasAttribute('required')).toBe(true);
	});

	it('forwards placeholder to the textarea element', () => {
		const { container } = render(Textarea, base({ placeholder: 'Enter config JSON' }));
		expect(container.querySelector('textarea')!.getAttribute('placeholder')).toBe('Enter config JSON');
	});

	it('forwards name to the textarea element', () => {
		const { container } = render(Textarea, base({ name: 'config' }));
		expect(container.querySelector('textarea')!.getAttribute('name')).toBe('config');
	});

	// --- aria-describedby ---

	it('forwards aria-describedby to the textarea element', () => {
		const { container } = render(Textarea, base({ 'aria-describedby': 'hint-id' }));
		expect(container.querySelector('textarea')!.getAttribute('aria-describedby')).toBe('hint-id');
	});

	it('omits aria-describedby when not provided', () => {
		const { container } = render(Textarea, base());
		expect(container.querySelector('textarea')!.hasAttribute('aria-describedby')).toBe(false);
	});

	// --- Events ---

	it('fires oninput callback on input event', async () => {
		const oninput = vi.fn();
		const { container } = render(Textarea, base({ oninput }));
		await fireEvent.input(container.querySelector('textarea')!);
		expect(oninput).toHaveBeenCalledTimes(1);
	});

	it('fires onblur callback on blur event', async () => {
		const onblur = vi.fn();
		const { container } = render(Textarea, base({ onblur }));
		await fireEvent.blur(container.querySelector('textarea')!);
		expect(onblur).toHaveBeenCalledTimes(1);
	});

	it('bind:value round-trip: DOM value reflects updated value after input event', async () => {
		const { container } = render(Textarea, base({ value: 'initial' }));
		const ta = container.querySelector('textarea')!;
		expect(ta.value).toBe('initial');
		await fireEvent.input(ta, { target: { value: 'updated' } });
		expect(ta.value).toBe('updated');
	});

	// --- Consumer class passthrough ---

	it('concatenates consumer class after internal classes', () => {
		const { container } = render(Textarea, base({ class: 'extra-marker' }));
		const cls = container.querySelector('textarea')!.className;
		expect(cls).toContain('extra-marker');
		expect(cls).toContain('w-full');
	});

	// --- TypeScript gate ---

	it('rejects invalid variant at the TS level', () => {
		// @ts-expect-error — 'compact' is not a valid TextareaVariant
		const _bad: TextareaProps = { id: 'x', value: '', variant: 'compact' };
		void _bad;
	});
});
```

- [ ] **Step 2: Run the tests — expect failures**

```bash
cd frontend && npx vitest run src/lib/components/Textarea.test.ts
```

Expected: FAIL — module not found.

- [ ] **Step 3: Create Textarea.svelte**

Create `frontend/src/lib/components/Textarea.svelte`:

```svelte
<script lang="ts" module>
	export type TextareaVariant = 'default' | 'mono';

	export type TextareaProps = {
		id: string;
		value: string;
		name?: string;
		placeholder?: string;
		rows?: number;
		disabled?: boolean;
		required?: boolean;
		error?: string;
		variant?: TextareaVariant;
		oninput?: (e: Event) => void;
		onblur?: (e: FocusEvent) => void;
		'aria-describedby'?: string;
		class?: string;
	};
</script>

<script lang="ts">
	const BASE =
		'w-full rounded-[3px] p-2 ' +
		'bg-[var(--bg-surface)] border border-[var(--border-default)] ' +
		'text-sm text-[var(--text-primary)] ' +
		'placeholder:text-[var(--text-muted)] ' +
		'focus-visible:outline-none ' +
		'focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] ' +
		'disabled:opacity-40 disabled:cursor-not-allowed ' +
		'aria-[invalid=true]:border-[var(--color-error-border)] ' +
		'aria-[invalid=true]:bg-[var(--color-error-bg)] ' +
		'transition-[background,border-color] duration-[0.12s] ' +
		'resize-y min-h-[4rem]';

	const VARIANT_CLASSES: Record<TextareaVariant, string> = {
		default: '',
		mono: 'font-mono text-[13px]'
	};

	let {
		id,
		value = $bindable(''),
		name,
		placeholder,
		rows,
		disabled = false,
		required = false,
		error,
		variant = 'default',
		oninput,
		onblur,
		'aria-describedby': ariaDescribedby,
		class: className = ''
	}: TextareaProps = $props();

	const computedClass = $derived(
		[BASE, VARIANT_CLASSES[variant], className].filter(Boolean).join(' ')
	);
	const hasError = $derived(!!error);
</script>

<textarea
	{id}
	bind:value
	{name}
	{placeholder}
	{rows}
	{disabled}
	{required}
	{oninput}
	{onblur}
	aria-invalid={hasError ? 'true' : undefined}
	aria-describedby={ariaDescribedby}
	class={computedClass}
></textarea>
```

- [ ] **Step 4: Run the tests — expect all to pass**

```bash
cd frontend && npx vitest run src/lib/components/Textarea.test.ts
```

Expected: all assertions pass.

- [ ] **Step 5: Svelte type-check**

```bash
cd frontend && npm run check 2>&1 | grep -E 'Textarea\.svelte'
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/components/Textarea.svelte frontend/src/lib/components/Textarea.test.ts
git commit -m "feat(ui): add Textarea primitive with variant matrix, error/aria-invalid wiring (sub-spec #2d)"
```

---

## Task 2: Extend preview route + Playwright e2e

**Files:**

- Modify: `frontend/src/routes/dev/form-primitive-preview/+page.svelte`
- Modify: `frontend/tests/e2e/form-primitive.spec.ts`

> Both files are created by sub-spec #2b. They must exist before executing this task.
> Read each file before editing — the exact content depends on the #2b implementer's output.
> The replacements below are based on the content defined in the #2b plan (Task 4).
> If the #2b implementer deviated from the plan, adjust accordingly.

- [ ] **Step 1: Extend the preview route**

Read `frontend/src/routes/dev/form-primitive-preview/+page.svelte` first. Then add:

1. `import Textarea from '$lib/components/Textarea.svelte';` in the `<script>` block alongside the other imports.
2. A `<section data-testid="textarea-states">` block appended at the bottom of `<main>`, before `</main>`.

The section to append:

```svelte
	<section data-testid="textarea-states">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Textarea — states</h2>
		<div class="flex flex-col gap-3" style="width: 480px;">
			<div data-testid="textarea-cell-default">
				<label class="mb-1 block text-xs text-[var(--text-muted)]" for="preview-ta-default">
					default / normal
				</label>
				<Textarea id="preview-ta-default" value="" placeholder="Default textarea" rows={4} />
			</div>
			<div data-testid="textarea-cell-error">
				<label class="mb-1 block text-xs text-[var(--text-muted)]" for="preview-ta-error">
					default / error
				</label>
				<Textarea
					id="preview-ta-error"
					value=""
					placeholder="Error textarea"
					rows={4}
					error="This field is required"
				/>
			</div>
			<div data-testid="textarea-cell-mono">
				<label class="mb-1 block text-xs text-[var(--text-muted)]" for="preview-ta-mono">
					mono / normal
				</label>
				<Textarea
					id="preview-ta-mono"
					value=""
					placeholder='{ "key": "value" }'
					rows={4}
					variant="mono"
				/>
			</div>
			<div data-testid="textarea-cell-mono-error">
				<label class="mb-1 block text-xs text-[var(--text-muted)]" for="preview-ta-mono-error">
					mono / error
				</label>
				<Textarea
					id="preview-ta-mono-error"
					value=""
					placeholder='{ "key": "value" }'
					rows={4}
					variant="mono"
					error="Invalid JSON"
				/>
			</div>
			<div data-testid="textarea-cell-disabled">
				<label class="mb-1 block text-xs text-[var(--text-muted)]" for="preview-ta-disabled">
					disabled
				</label>
				<Textarea id="preview-ta-disabled" value="" placeholder="Disabled textarea" rows={4} disabled />
			</div>
		</div>
	</section>
```

- [ ] **Step 2: Verify route builds**

```bash
cd frontend && npm run build 2>&1 | tail -10
```

Expected: build succeeds, no errors.

- [ ] **Step 3: Extend the Playwright e2e spec**

Read `frontend/tests/e2e/form-primitive.spec.ts` first. Add `{ id: 'textarea-states', name: 'textarea' }` to the
`SECTIONS` array. That increases the baseline count by 2 (1 new section × 2 themes).

The updated `SECTIONS` array (based on the 5 sections defined in the #2b plan):

```ts
const SECTIONS = [
	{ id: 'input-types', name: 'input-types' },
	{ id: 'input-states', name: 'input-states' },
	{ id: 'checkbox-states', name: 'checkbox-states' },
	{ id: 'link-variants', name: 'link-variants' },
	{ id: 'link-external', name: 'link-external' },
	{ id: 'textarea-states', name: 'textarea' }
];
```

- [ ] **Step 4: Install Chromium if not yet installed (one-time)**

```bash
cd frontend && npx playwright install --with-deps chromium
```

Expected: installs or reports already present.

- [ ] **Step 5: Re-baseline all form-primitive snapshots (macOS + Chromium)**

> `--update-snapshots` regenerates all PNGs in the spec, including the 10 from #2b and the 2 new
> textarea ones. This is required because the route now renders a taller page — existing section
> snapshots are region-scoped (not full-page) so they should be unchanged, but regenerating
> ensures no stale reference mismatch.

```bash
cd frontend && npx playwright test tests/e2e/form-primitive.spec.ts --update-snapshots
```

Expected: 12 PNGs written (6 sections × 2 themes).

- [ ] **Step 6: Re-run to confirm stability**

```bash
cd frontend && npx playwright test tests/e2e/form-primitive.spec.ts
```

Expected: 12/12 pass with no diff failures.

- [ ] **Step 7: Commit**

```bash
git add \
  frontend/src/routes/dev/form-primitive-preview/+page.svelte \
  frontend/tests/e2e/form-primitive.spec.ts \
  "frontend/tests/e2e/form-primitive.spec.ts-snapshots"
git commit -m "chore(dev): extend form-primitive preview + Playwright baselines with Textarea (sub-spec #2d)"
```

---

## Task 3: Full frontend gate

- [ ] **Step 1: Run full gate**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: every command exits 0.

- [ ] **Step 2: Run full e2e suite**

```bash
cd frontend && npx playwright test 2>&1 | tail -20
```

Expected: 0 failures. New `textarea` snapshots pass; existing `button-primitive` and `form-primitive`
(non-textarea sections) unaffected — no consumer migrations in this plan.

- [ ] **Step 3: Commit if any lint/format auto-fixes were applied**

Only if `npm run lint` or `format:check` required fixes:

```bash
git add -p
git commit -m "chore(frontend): lint/format fixes after Textarea primitive addition (sub-spec #2d)"
```
