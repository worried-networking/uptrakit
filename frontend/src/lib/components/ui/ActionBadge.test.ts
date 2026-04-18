import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import ActionBadge from './ActionBadge.svelte';

afterEach(() => {
	cleanup();
});

describe('ActionBadge', () => {
	it('renders the interactive navigation badge contract', () => {
		render(ActionBadge, {
			variant: 'navigation',
			tone: 'info',
			idleLabel: '2 updates',
			hoverLabel: '→ Software'
		});

		const badge = screen.getByRole('button', { name: '2 updates' });
		expect(badge).toHaveAttribute('data-ui', 'action-badge');
		expect(badge).toHaveAttribute('data-variant', 'navigation');
		expect(badge).toHaveAttribute('data-tone', 'info');
		expect(badge.className).toContain('min-w-max');
		expect(badge.className).toContain('hover:bg-[color-mix(in_srgb,var(--color-info-bg)_60%,var(--color-info)_40%)]');
		expect(badge.className).toContain(
			'hover:border-[color-mix(in_srgb,var(--color-info-border)_70%,var(--color-info)_30%)]'
		);
		expect(badge.className).toContain('transition-[background,border-color,color]');
		expect(badge.className).toContain('focus-visible:outline-none');
		expect(badge.className).toContain('focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]');
		expect(screen.getByText('→ Software')).toHaveClass('hov');
	});

	it('supports the danger tone for history navigation badges', () => {
		render(ActionBadge, {
			variant: 'navigation',
			tone: 'danger',
			idleLabel: '1 error',
			hoverLabel: '→ History'
		});

		expect(screen.getByRole('button', { name: '1 error' })).toHaveAttribute('data-tone', 'danger');
	});

	it('supports disabled bulk-update badges without firing clicks', async () => {
		const onclick = vi.fn();
		render(ActionBadge, {
			variant: 'bulk-update',
			tone: 'accent',
			idleLabel: '↑ Update all',
			hoverLabel: '↑ Update all',
			disabled: true,
			onclick
		});

		const badge = screen.getByRole('button', { name: '↑ Update all' });
		expect(badge).toBeDisabled();
		expect(badge).toHaveAttribute('data-variant', 'bulk-update');
		expect(badge.className).toContain('disabled:pointer-events-none');
		expect(badge.className).toContain('disabled:opacity-40');
		await fireEvent.click(badge);
		expect(onclick).not.toHaveBeenCalled();
	});

	it('pins the in-flow overlap structure that reserves width for the hover label', () => {
		render(ActionBadge, {
			variant: 'navigation',
			tone: 'accent',
			idleLabel: 'Up',
			hoverLabel: 'Open Software Updates'
		});

		const badge = screen.getByRole('button', { name: 'Up' });
		const idle = screen.getByText('Up');
		const hover = screen.getByText('Open Software Updates');
		const labelStack = idle.parentElement as HTMLElement;

		expect(badge.className).toContain('relative');
		expect(badge.className).toContain('min-w-max');
		expect(labelStack.className).toContain('grid');
		expect(labelStack.className).toContain('grid-cols-1');
		expect(idle.className).toContain('group-hover:invisible');
		expect(idle.className).not.toContain('group-hover:hidden');
		expect(idle.className).toContain('col-start-1');
		expect(idle.className).toContain('row-start-1');
		expect(hover.className).toContain('col-start-1');
		expect(hover.className).toContain('row-start-1');
		expect(hover.className).not.toContain('absolute');
		expect(hover.className).not.toContain('inset-0');
		expect(hover.className).toContain('group-hover:visible');
	});
});
