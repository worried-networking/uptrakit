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

	it('forwards aria-describedby to the textarea element', () => {
		const { container } = render(Textarea, base({ 'aria-describedby': 'hint-id' }));
		expect(container.querySelector('textarea')!.getAttribute('aria-describedby')).toBe('hint-id');
	});

	it('omits aria-describedby when not provided', () => {
		const { container } = render(Textarea, base());
		expect(container.querySelector('textarea')!.hasAttribute('aria-describedby')).toBe(false);
	});

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

	it('concatenates consumer class after internal classes', () => {
		const { container } = render(Textarea, base({ class: 'extra-marker' }));
		const cls = container.querySelector('textarea')!.className;
		expect(cls).toContain('extra-marker');
		expect(cls).toContain('w-full');
	});

	it('rejects invalid variant at the TS level', () => {
		// @ts-expect-error — 'compact' is not a valid TextareaVariant
		const _bad: TextareaProps = { id: 'x', value: '', variant: 'compact' };
		void _bad;
	});
});
