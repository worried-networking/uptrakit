import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/svelte';
import Select from './Select.svelte';

type SelectOption = { value: string; label: string };
type SelectProps = {
	id: string;
	value: string;
	options: SelectOption[];
	name?: string;
	placeholder?: string;
	disabled?: boolean;
	required?: boolean;
	error?: string;
	onchange?: (e: Event) => void;
	onblur?: (e: FocusEvent) => void;
	'aria-describedby'?: string;
	'aria-label'?: string;
	class?: string;
};

const OPTS: SelectOption[] = [
	{ value: 'a', label: 'Option A' },
	{ value: 'b', label: 'Option B' }
];

function baseSelect(extra: Partial<SelectProps> = {}): SelectProps {
	return { id: 'test-select', value: 'a', options: OPTS, ...extra };
}

describe('Select primitive', () => {
	it('renders a <select> element', () => {
		const { container } = render(Select, baseSelect());
		expect(container.querySelector('select')).not.toBeNull();
	});

	it('forwards the id prop to the underlying select', () => {
		const { container } = render(Select, baseSelect({ id: 'my-field' }));
		expect(container.querySelector('select')!.getAttribute('id')).toBe('my-field');
	});

	it('renders option elements from options prop', () => {
		const { container } = render(Select, baseSelect());
		const opts = container.querySelectorAll('option');
		expect(opts.length).toBe(2);
		expect(opts[0].value).toBe('a');
		expect(opts[0].textContent).toBe('Option A');
		expect(opts[1].value).toBe('b');
		expect(opts[1].textContent).toBe('Option B');
	});

	it('renders placeholder as first disabled option when provided', () => {
		const { container } = render(Select, baseSelect({ placeholder: 'Pick one' }));
		const opts = container.querySelectorAll('option');
		expect(opts.length).toBe(3);
		expect(opts[0].value).toBe('');
		expect(opts[0].textContent).toBe('Pick one');
		expect(opts[0].disabled).toBe(true);
	});

	it('does not render placeholder option when placeholder is not provided', () => {
		const { container } = render(Select, baseSelect());
		const opts = container.querySelectorAll('option');
		expect(opts.length).toBe(2);
	});

	it('applies base class tokens', () => {
		const { container } = render(Select, baseSelect());
		const cls = container.querySelector('select')!.className;
		expect(cls).toContain('h-8');
		expect(cls).toContain('w-full');
		expect(cls).toContain('bg-[var(--bg-surface)]');
		expect(cls).toContain('border-[var(--border-default)]');
		expect(cls).toContain('text-sm');
		expect(cls).toContain('text-[var(--text-primary)]');
	});

	it('does not include placeholder: pseudo-class token in class string', () => {
		const { container } = render(Select, baseSelect());
		expect(container.querySelector('select')!.className).not.toContain('placeholder:');
	});

	it('applies focus-visible ring class', () => {
		const { container } = render(Select, baseSelect());
		expect(container.querySelector('select')!.className).toContain(
			'focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]'
		);
	});

	it('applies disabled utility classes in class string', () => {
		const { container } = render(Select, baseSelect());
		const cls = container.querySelector('select')!.className;
		expect(cls).toContain('disabled:opacity-40');
		expect(cls).toContain('disabled:cursor-not-allowed');
	});

	it('applies aria-invalid error classes in class string', () => {
		const { container } = render(Select, baseSelect());
		const cls = container.querySelector('select')!.className;
		expect(cls).toContain('aria-[invalid=true]:border-[var(--color-danger-border)]');
		expect(cls).toContain('aria-[invalid=true]:bg-[var(--color-danger-bg)]');
	});

	it('sets aria-invalid="true" when error prop is non-empty', () => {
		const { container } = render(Select, baseSelect({ error: 'Required' }));
		expect(container.querySelector('select')!.getAttribute('aria-invalid')).toBe('true');
	});

	it('omits aria-invalid attribute when error is undefined', () => {
		const { container } = render(Select, baseSelect());
		expect(container.querySelector('select')!.hasAttribute('aria-invalid')).toBe(false);
	});

	it('omits aria-invalid attribute when error is empty string', () => {
		const { container } = render(Select, baseSelect({ error: '' }));
		expect(container.querySelector('select')!.hasAttribute('aria-invalid')).toBe(false);
	});

	it('sets disabled attribute when disabled prop is true', () => {
		const { container } = render(Select, baseSelect({ disabled: true }));
		expect(container.querySelector('select')!.hasAttribute('disabled')).toBe(true);
	});

	it('sets required attribute when required prop is true', () => {
		const { container } = render(Select, baseSelect({ required: true }));
		expect(container.querySelector('select')!.hasAttribute('required')).toBe(true);
	});

	it('forwards name to the select element', () => {
		const { container } = render(Select, baseSelect({ name: 'sort' }));
		expect(container.querySelector('select')!.getAttribute('name')).toBe('sort');
	});

	it('forwards aria-describedby to the select element', () => {
		const { container } = render(Select, baseSelect({ 'aria-describedby': 'hint-id' }));
		expect(container.querySelector('select')!.getAttribute('aria-describedby')).toBe('hint-id');
	});

	it('omits aria-describedby when not provided', () => {
		const { container } = render(Select, baseSelect());
		expect(container.querySelector('select')!.hasAttribute('aria-describedby')).toBe(false);
	});

	it('fires onchange callback on change event', async () => {
		const onchange = vi.fn();
		const { container } = render(Select, baseSelect({ onchange }));
		await fireEvent.change(container.querySelector('select')!);
		expect(onchange).toHaveBeenCalledTimes(1);
	});

	it('fires onblur callback on blur event', async () => {
		const onblur = vi.fn();
		const { container } = render(Select, baseSelect({ onblur }));
		await fireEvent.blur(container.querySelector('select')!);
		expect(onblur).toHaveBeenCalledTimes(1);
	});

	it('concatenates consumer class after internal classes', () => {
		const { container } = render(Select, baseSelect({ class: 'extra-marker' }));
		const cls = container.querySelector('select')!.className;
		expect(cls).toContain('extra-marker');
		expect(cls).toContain('h-8');
	});
});
