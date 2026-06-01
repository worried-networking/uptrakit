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

	it('renders filterBar snippet below title without border-b on title row', () => {
		const { container } = render(SectionCard, {
			title: 'My Table',
			children: makeSnippet('<p>body</p>'),
			filterBar: makeSnippet('<div data-testid="fb">FilterBar</div>')
		});
		expect(screen.getByTestId('fb')).toBeInTheDocument();
		const header = container.querySelector('header');
		expect(header?.className).not.toContain('border-b');
	});

	it('renders border-b on title row when filterBar not provided', () => {
		const { container } = render(SectionCard, {
			title: 'My Table',
			children: makeSnippet('<p>body</p>')
		});
		const header = container.querySelector('header');
		expect(header?.className).toContain('border-b');
	});
});
