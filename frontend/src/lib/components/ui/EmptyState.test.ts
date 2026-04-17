import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { createRawSnippet } from 'svelte';
import EmptyState from './EmptyState.svelte';

function makeSnippet(html: string) {
	return createRawSnippet(() => ({
		render() {
			return html;
		}
	}));
}

afterEach(() => {
	cleanup();
});

describe('EmptyState', () => {
	it('renders the title, description, and optional actions', () => {
		const { container } = render(EmptyState, {
			title: 'No providers connected',
			description: 'Connect a provider to continue.',
			actions: makeSnippet('<button type="button">Retry</button>')
		});

		expect(container.querySelector('[data-ui="empty-state"]')).toBeInTheDocument();
		expect(screen.getByText('No providers connected')).toBeInTheDocument();
		expect(screen.getByText('Connect a provider to continue.')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument();
	});
});
