import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/svelte';
import Checkbox from './Checkbox.svelte';

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
		expect(cls).toContain('rounded-badge');
		expect(cls).toContain('border-[var(--border-default)]');
		expect(cls).toContain('text-[var(--accent)]');
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

	it('has data-ui="checkbox" attribute', () => {
		const { container } = render(Checkbox, baseCheckbox());
		expect(container.querySelector('input')!.getAttribute('data-ui')).toBe('checkbox');
	});

	it('fires onchange callback on change event', async () => {
		const onchange = vi.fn();
		const { container } = render(Checkbox, baseCheckbox({ onchange }));
		await fireEvent.change(container.querySelector('input')!);
		expect(onchange).toHaveBeenCalledTimes(1);
	});

	it('bind:checked round-trip: DOM checked reflects updated state after change event', async () => {
		const { container } = render(Checkbox, { id: 'rt', checked: false });
		const input = container.querySelector('input[type="checkbox"]')!;
		expect((input as HTMLInputElement).checked).toBe(false);
		await fireEvent.click(input);
		expect((input as HTMLInputElement).checked).toBe(true);
	});

	it('concatenates consumer class after internal classes', () => {
		const { container } = render(Checkbox, baseCheckbox({ class: 'extra-marker' }));
		const cls = container.querySelector('input')!.className;
		expect(cls).toContain('extra-marker');
		expect(cls).toContain('h-4');
	});
});
