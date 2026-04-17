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
		await fireEvent.click(badge);
		expect(onclick).not.toHaveBeenCalled();
	});
});
