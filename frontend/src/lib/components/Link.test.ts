import { describe, expect, it } from 'vitest';
import { render } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import Link from './Link.svelte';

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
