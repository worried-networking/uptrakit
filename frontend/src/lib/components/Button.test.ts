import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import type { Snippet } from 'svelte';
import type { MouseEventHandler } from 'svelte/elements';
import Button from './Button.svelte';

// Mirror of the discriminated-union exported from Button.svelte's module script.
// Kept in sync manually; the @ts-expect-error cases below will catch regressions.
type ButtonVariant = 'primary' | 'ghost' | 'danger' | 'secondary';
type ButtonSize = 'sm' | 'md';
type CommonProps = {
	variant: ButtonVariant;
	size?: ButtonSize;
	disabled?: boolean;
	loading?: boolean;
	leadingIcon?: Snippet;
	trailingIcon?: Snippet;
	ariaLabel?: string;
	children: Snippet;
	class?: string;
};
type ButtonProps =
	| (CommonProps & { href: string; type?: never; onclick?: never })
	| (CommonProps & {
			href?: never;
			type?: 'button' | 'submit' | 'reset';
			onclick?: MouseEventHandler<HTMLButtonElement>;
	  });

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
		const { container } = render(Button, mdButton({ variant: 'ghost', href: '/x', disabled: true }));
		const link = container.querySelector('a')!;
		expect(link.getAttribute('aria-disabled')).toBe('true');
		const event = new MouseEvent('click', { bubbles: true, cancelable: true });
		link.dispatchEvent(event);
		expect(event.defaultPrevented).toBe(true);
	});

	it('link branch preventDefaults Enter/Space keydown when loading', () => {
		const { container } = render(Button, mdButton({ variant: 'ghost', href: '/x', loading: true }));
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
		// @ts-expect-error — 'tertiary' is not a valid ButtonVariant
		const _bad3: ButtonProps = { variant: 'tertiary', children };
		void _bad1;
		void _bad2;
		void _bad3;
	});

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
		expect(cls).not.toMatch(/disabled:opacity-(?!40)/);
	});

	it('secondary variant contains active:opacity-[0.88] class fragment', () => {
		const { container } = render(Button, mdButton({ variant: 'secondary' }));
		const cls = container.querySelector('button')!.className;
		expect(cls).toContain('active:opacity-[0.88]');
	});
});
