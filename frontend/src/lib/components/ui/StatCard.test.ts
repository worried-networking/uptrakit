import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import type { StatCardTone } from './StatCard.svelte';
import StatCard from './StatCard.svelte';

afterEach(() => {
	cleanup();
});

describe('StatCard', () => {
	it('renders as a navigable link with label, value, and sub-label', () => {
		render(StatCard, {
			href: '/hosts',
			label: 'Hosts',
			value: 42,
			valueTone: 'success',
			subLabel: 'registered hosts'
		});

		const link = screen.getByRole('link');
		expect(link).toHaveAttribute('href', '/hosts');
		expect(link).toHaveAttribute('data-ui', 'stat-card');
		expect(link).toHaveAttribute('data-tone', 'success');
		expect(screen.getByText('Hosts')).toBeInTheDocument();
		expect(screen.getByText('42')).toBeInTheDocument();
		expect(screen.getByText('registered hosts')).toBeInTheDocument();
	});

	it('applies required structural and interactive classes', () => {
		const { container } = render(StatCard, {
			href: '/x',
			label: 'X',
			value: 0,
			subLabel: 'y'
		});

		const card = container.querySelector('[data-ui="stat-card"]') as HTMLElement;
		expect(card.tagName).toBe('A');
		expect(card.className).toContain('block');
		expect(card.className).toContain('rounded-[3px]');
		expect(card.className).toContain('bg-[var(--bg-surface)]');
		expect(card.className).toContain('border-[var(--border-subtle)]');
		expect(card.className).toContain('transition-[background,border-color,color]');
		expect(card.className).toContain('duration-[120ms]');
		expect(card.className).toContain('hover:border-[var(--accent)]');
		expect(card.className).toContain('focus-visible:outline-none');
		expect(card.className).toContain('focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]');
	});

	it('defaults valueTone to muted', () => {
		const { container } = render(StatCard, {
			href: '/services',
			label: 'Services',
			value: 5,
			subLabel: 'active'
		});

		expect(container.querySelector('[data-ui="stat-card"]')).toHaveAttribute('data-tone', 'muted');
		expect(screen.getByText('5').getAttribute('style') ?? '').toContain('--text-muted');
	});

	it('maps each tone to the correct CSS variable in the value style', () => {
		const cases: Array<[StatCardTone, string]> = [
			['success', '--color-success'],
			['info', '--color-info'],
			['warning', '--color-warning'],
			['danger', '--color-error']
		];

		for (const [tone, token] of cases) {
			const { container } = render(StatCard, {
				href: '/x',
				label: 'Label',
				value: 99,
				valueTone: tone,
				subLabel: 'sub'
			});

			expect(container.querySelector('[data-ui="stat-card"]')).toHaveAttribute('data-tone', tone);
			expect(screen.getByText('99').getAttribute('style') ?? '').toContain(token);
			cleanup();
		}
	});
});
