import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { createRawSnippet } from 'svelte';
import FilterBar from './FilterBar.svelte';

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

describe('FilterBar', () => {
	it('renders data-ui="filter-bar" attribute', () => {
		const { container } = render(FilterBar, {
			filters: makeSnippet('<span>filter content</span>')
		});
		expect(container.querySelector('[data-ui="filter-bar"]')).toBeInTheDocument();
	});

	it('renders filters snippet on the left', () => {
		render(FilterBar, { filters: makeSnippet('<span data-testid="f">filter</span>') });
		expect(screen.getByTestId('f')).toBeInTheDocument();
	});

	it('renders actions snippet on the right when provided', () => {
		const { container } = render(FilterBar, {
			filters: makeSnippet('<span>f</span>'),
			actions: makeSnippet('<button type="button">Add</button>')
		});
		expect(screen.getByRole('button', { name: 'Add' })).toBeInTheDocument();
		expect(container.querySelector('.shrink-0')).toBeInTheDocument();
	});

	it('omits actions container when actions snippet not provided', () => {
		const { container } = render(FilterBar, {
			filters: makeSnippet('<span>f</span>')
		});
		expect(container.querySelector('.shrink-0')).not.toBeInTheDocument();
	});

	it('filters are in a flex-wrap container before actions', () => {
		const { container } = render(FilterBar, {
			filters: makeSnippet('<span data-testid="f">f</span>'),
			actions: makeSnippet('<button>Add</button>')
		});
		const header = container.querySelector('[data-ui="filter-bar"]')!;
		const children = Array.from(header.children);
		expect(children[0].querySelector('[data-testid="f"]')).toBeInTheDocument();
		expect(children[1].querySelector('button')).toBeInTheDocument();
	});
});
