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
		render(DataTable, {
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
});
