import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import StatusBadge from './StatusBadge.svelte';

afterEach(() => {
	cleanup();
});

describe('StatusBadge', () => {
	it('renders the compact semantic tone shell and label', () => {
		const { container } = render(StatusBadge, {
			tone: 'success',
			label: 'Connected'
		});

		const badge = container.querySelector('[data-ui="status-badge"]');
		expect(badge).toHaveAttribute('data-tone', 'success');
		expect(badge?.className).toContain('min-h-badge');
		expect(badge?.className).toContain('rounded-badge');
		expect(badge?.className).toContain('text-badge');
		expect(screen.getByText('Connected')).toBeInTheDocument();
	});
});
