import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { createRawSnippet } from 'svelte';
import DataTable from './DataTable.svelte';

function makeRowActions() {
	return createRawSnippet<[Record<string, unknown>]>((getRow) => ({
		render() {
			const row = getRow();
			return `<button type="button">Inspect ${String(row.name)}</button>`;
		}
	}));
}

function makeHeaderSnippet() {
	return createRawSnippet(() => ({
		render() {
			return '<tr><th scope="col">Custom Name</th><th scope="col">Custom Status</th></tr>';
		}
	}));
}

function makeRowSnippet() {
	return createRawSnippet<[Record<string, unknown>]>((getRow) => ({
		render() {
			const row = getRow();
			return `<tr><td><strong>${String(row.name)}</strong></td><td>${String(row.status)}</td></tr>`;
		}
	}));
}

function makeErrorActionsSnippet() {
	return createRawSnippet(() => ({
		render() {
			return '<button type="button">Retry load</button>';
		}
	}));
}

function makeFooterSnippet() {
	return createRawSnippet(() => ({
		render() {
			return '<div data-testid="table-footer">42 total</div>';
		}
	}));
}

afterEach(() => {
	cleanup();
});

describe('DataTable', () => {
	it('renders headers, row values, and row actions through a shared shell', () => {
		const { container } = render(DataTable, {
			columns: [
				{ key: 'name', label: 'Name' },
				{ key: 'status', label: 'Status' }
			],
			rows: [
				{ name: 'alpha', status: 'ready' },
				{ name: 'beta', status: 'pending' }
			],
			rowActions: makeRowActions()
		});

		expect(screen.getByRole('columnheader', { name: 'Name' })).toBeInTheDocument();
		expect(screen.getByRole('columnheader', { name: 'Status' })).toBeInTheDocument();
		expect(screen.getByText('alpha')).toBeInTheDocument();
		expect(screen.getByText('pending')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Inspect alpha' })).toBeInTheDocument();
		expect(container.querySelector('table')).toHaveClass('text-table-body');
	});

	it('falls back to the shared empty state when there are no rows', () => {
		render(DataTable, {
			columns: [{ key: 'name', label: 'Name' }],
			rows: [],
			emptyTitle: 'No rows available',
			emptyDescription: 'Try reloading the provider response.'
		});

		expect(screen.getByText('No rows available')).toBeInTheDocument();
		expect(screen.getByText('Try reloading the provider response.')).toBeInTheDocument();
		expect(screen.queryByRole('table')).not.toBeInTheDocument();
	});

	it('supports custom header and row snippets for rich table rendering', () => {
		render(DataTable, {
			columns: [],
			rows: [{ name: 'gamma', status: 'ok' }],
			header: makeHeaderSnippet(),
			row: makeRowSnippet()
		});

		expect(screen.getByRole('columnheader', { name: 'Custom Name' })).toBeInTheDocument();
		expect(screen.getByText('gamma')).toBeInTheDocument();
		expect(screen.getByText('ok')).toBeInTheDocument();
	});

	it('renders custom error actions when an error is present', () => {
		render(DataTable, {
			columns: [{ key: 'name', label: 'Name' }],
			rows: [],
			error: 'Foreground load failed',
			errorActions: makeErrorActionsSnippet()
		});

		expect(screen.getByText('Foreground load failed')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Retry load' })).toBeInTheDocument();
	});

	it('renders the shared table footer slot inside the table shell', () => {
		const { container } = render(DataTable, {
			columns: [{ key: 'name', label: 'Name' }],
			rows: [{ name: 'alpha' }],
			footer: makeFooterSnippet()
		});

		expect(screen.getByTestId('table-footer')).toBeInTheDocument();
		expect(container.querySelector('[data-ui="data-table"]')).toContainElement(screen.getByTestId('table-footer'));
	});

	describe('responsive mode', () => {
		it('renders data-table-cards container with role=list when mobileMode is cards', () => {
			const { container } = render(DataTable, {
				columns: [
					{ key: 'name', label: 'Name', mobileTitle: true },
					{ key: 'status', label: 'Status' }
				],
				rows: [{ name: 'alpha', status: 'ready' }],
				mobileMode: 'cards'
			});

			const cardsEl = container.querySelector('[data-ui="data-table-cards"]');
			expect(cardsEl).toBeInTheDocument();
			expect(cardsEl).toHaveAttribute('role', 'list');
		});

		it('renders both table and cards layouts in DOM for cards mode (dual-DOM)', () => {
			const { container } = render(DataTable, {
				columns: [{ key: 'name', label: 'Name', mobileTitle: true }],
				rows: [{ name: 'alpha' }],
				mobileMode: 'cards'
			});

			expect(container.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
			expect(container.querySelector('[data-ui="data-table-cards"]')).toBeInTheDocument();
		});

		it('auto-generates a card with title from mobileTitle column and dl key/value pairs', () => {
			const { container } = render(DataTable, {
				columns: [
					{ key: 'name', label: 'Name', mobileTitle: true },
					{ key: 'status', label: 'Status' }
				],
				rows: [{ name: 'alpha', status: 'ready' }],
				mobileMode: 'cards'
			});

			const cardsEl = container.querySelector('[data-ui="data-table-cards"]');
			const listItem = cardsEl?.querySelector('[role="listitem"]');
			expect(listItem).toBeInTheDocument();
			// Title column renders as <p>
			const titleEl = listItem?.querySelector('p');
			expect(titleEl).toHaveTextContent('alpha');
			// Value column renders in <dl>
			expect(listItem?.querySelector('dt')).toHaveTextContent('Status');
			expect(listItem?.querySelector('dd')).toHaveTextContent('ready');
		});

		it('uses the first visible column as implicit title when no mobileTitle is set', () => {
			const { container } = render(DataTable, {
				columns: [
					{ key: 'name', label: 'Name' },
					{ key: 'status', label: 'Status' }
				],
				rows: [{ name: 'alpha', status: 'ready' }],
				mobileMode: 'cards'
			});

			const cardsEl = container.querySelector('[data-ui="data-table-cards"]');
			const listItem = cardsEl?.querySelector('[role="listitem"]');
			const titleEl = listItem?.querySelector('p');
			expect(titleEl).toHaveTextContent('alpha');
			// 'Name' column used as title so only 'Status' appears in dl
			expect(listItem?.querySelector('dt')).toHaveTextContent('Status');
		});

		it('excludes mobileHide columns from auto-generated cards', () => {
			const { container } = render(DataTable, {
				columns: [
					{ key: 'name', label: 'Name', mobileTitle: true },
					{ key: 'internal', label: 'Internal', mobileHide: true },
					{ key: 'status', label: 'Status' }
				],
				rows: [{ name: 'alpha', internal: 'hidden', status: 'ready' }],
				mobileMode: 'cards'
			});

			const cardsEl = container.querySelector('[data-ui="data-table-cards"]');
			const dts = cardsEl?.querySelectorAll('dt');
			// Only 'Status' dt — 'Internal' is hidden, 'Name' is the title
			expect(dts).toHaveLength(1);
			expect(dts?.[0]).toHaveTextContent('Status');
		});

		it('renders mobileRow snippet content in cards mode when provided', () => {
			const mobileRowSnippet = createRawSnippet<[Record<string, unknown>]>((getRow) => ({
				render() {
					const row = getRow();
					return `<div role="listitem" data-testid="custom-mobile-card">${String(row.name)}</div>`;
				}
			}));

			const { container } = render(DataTable, {
				columns: [{ key: 'name', label: 'Name' }],
				rows: [{ name: 'custom' }],
				mobileMode: 'cards',
				mobileRow: mobileRowSnippet
			});

			expect(container.querySelector('[data-testid="custom-mobile-card"]')).toBeInTheDocument();
			expect(container.querySelector('[data-testid="custom-mobile-card"]')).toHaveTextContent('custom');
		});

		it('renders auto-generated cards even when custom row snippet is provided without mobileRow', () => {
			const { container } = render(DataTable, {
				columns: [{ key: 'name', label: 'Name' }],
				rows: [{ name: 'alpha' }],
				mobileMode: 'cards',
				row: makeRowSnippet()
				// no mobileRow: auto-generated cards are the normal path
			});

			// Cards DOM present — row snippet does not suppress cards mode
			expect(container.querySelector('[data-ui="data-table-cards"]')).toBeInTheDocument();
			expect(container.querySelector('[data-ui="data-table-cards"] [role="listitem"]')).toBeInTheDocument();
		});

		it('applies w-max class to table when mobileMode is scroll', () => {
			const { container } = render(DataTable, {
				columns: [{ key: 'name', label: 'Name' }],
				rows: [{ name: 'alpha' }],
				mobileMode: 'scroll'
			});

			expect(container.querySelector('table')).toHaveClass('w-max');
			expect(container.querySelector('table')).not.toHaveClass('min-w-full');
		});

		it('keeps min-w-full on table when no mobileMode is provided', () => {
			const { container } = render(DataTable, {
				columns: [{ key: 'name', label: 'Name' }],
				rows: [{ name: 'alpha' }]
			});

			expect(container.querySelector('table')).toHaveClass('min-w-full');
			expect(container.querySelector('table')).not.toHaveClass('w-max');
		});

		it('renders rowActions in a group element inside cards auto-layout', () => {
			const { container } = render(DataTable, {
				columns: [{ key: 'name', label: 'Name', mobileTitle: true }],
				rows: [{ name: 'alpha' }],
				mobileMode: 'cards',
				rowActions: makeRowActions(),
				rowActionsLabel: 'Row actions'
			});

			const cardsEl = container.querySelector('[data-ui="data-table-cards"]');
			const actionsGroup = cardsEl?.querySelector('[role="group"]');
			expect(actionsGroup).toBeInTheDocument();
			expect(actionsGroup).toHaveAttribute('aria-label', 'Row actions');
			expect(actionsGroup?.querySelector('button')).toHaveTextContent('Inspect alpha');
		});
	});
});
