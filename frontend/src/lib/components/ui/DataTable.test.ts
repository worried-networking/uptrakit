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
});
