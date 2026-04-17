import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { createRawSnippet } from 'svelte';
import SectionCard from './SectionCard.svelte';

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

describe('SectionCard', () => {
	it('renders a titled panel with optional actions and body content', () => {
		const { container } = render(SectionCard, {
			title: 'Runtime state',
			description: 'Latest provider response',
			actions: makeSnippet('<button type="button">Reload</button>'),
			children: makeSnippet('<p>Rendered payload</p>')
		});

		expect(container.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
		expect(screen.getByRole('heading', { name: 'Runtime state' })).toBeInTheDocument();
		expect(screen.getByText('Latest provider response')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Reload' })).toBeInTheDocument();
		expect(screen.getByText('Rendered payload')).toBeInTheDocument();
	});
});
