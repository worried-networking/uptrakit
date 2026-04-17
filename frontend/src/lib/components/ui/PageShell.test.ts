import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { createRawSnippet } from 'svelte';
import PageShell from './PageShell.svelte';

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

describe('PageShell', () => {
	it('renders the title, supporting copy, actions, and content in one shell', () => {
		const { container } = render(PageShell, {
			title: 'Plugin Settings',
			description: 'Manage shared provider defaults.',
			actions: makeSnippet('<button type="button">Refresh</button>'),
			children: makeSnippet('<p>Body content</p>')
		});

		expect(container.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
		expect(screen.getByRole('heading', { name: 'Plugin Settings' })).toBeInTheDocument();
		expect(screen.getByText('Manage shared provider defaults.')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Refresh' })).toBeInTheDocument();
		expect(screen.getByText('Body content')).toBeInTheDocument();
	});
});
