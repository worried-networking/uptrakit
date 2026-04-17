import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import StatusBadge from './StatusBadge.svelte';

afterEach(() => {
	cleanup();
});

describe('StatusBadge', () => {
	it('renders the semantic tone and label', () => {
		const { container } = render(StatusBadge, {
			tone: 'success',
			label: 'Connected'
		});

		const badge = container.querySelector('[data-ui="status-badge"]');
		expect(badge).toHaveAttribute('data-tone', 'success');
		expect(screen.getByText('Connected')).toBeInTheDocument();
	});
});
