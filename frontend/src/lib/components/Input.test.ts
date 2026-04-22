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
