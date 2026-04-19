import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import type { PaginatedResponse, SoftwareIgnoreResponse } from '$lib/types';
import { Permission } from '$lib/types';

vi.mock('$lib/api', () => ({
	getSoftwareIgnores: vi.fn(),
	createSoftwareIgnore: vi.fn(),
	deleteSoftwareIgnore: vi.fn(),
	batchSoftwareIgnores: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null)
}));

vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn()
}));

import IgnoreRulesTab from './IgnoreRulesTab.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';

const managerUser = {
	id: '00000000-0000-0000-0000-000000000042',
	email: 'ignores@example.com',
	first_name: 'Ignore',
	last_name: 'Manager',
	permissions: [Permission.ManageIgnores]
};

function makePage(items: SoftwareIgnoreResponse[]): PaginatedResponse<SoftwareIgnoreResponse> {
	return {
		items,
		total: items.length,
		page: 1,
		per_page: 25,
		total_pages: 1
	};
}

function deferred<T>() {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((r) => {
		resolve = r;
	});
	return { promise, resolve };
}

describe('Software Ignore Rules Tab', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(managerUser);
		vi.mocked(api.getSoftwareIgnores).mockResolvedValue(
			makePage([
				{
					id: 'ignore-1',
					name: 'Plex',
					created_at: '2026-04-01T10:00:00Z'
				}
			])
		);
		vi.mocked(api.batchSoftwareIgnores).mockResolvedValue({ succeeded: [], failed: [] });
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('renders shared table/footer primitives for the ignore-rules list', async () => {
		render(IgnoreRulesTab);

		await waitFor(() => expect(screen.getByText('Plex')).toBeInTheDocument());
		expect(document.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="table-footer-bar"]')).toBeInTheDocument();
		expect(screen.getByText('1 total')).toBeInTheDocument();
		expect(document.querySelector('.table-wrap')).not.toBeInTheDocument();
	});

	it('uses shared loading and empty-state treatments', async () => {
		const ignoresDeferred = deferred<PaginatedResponse<SoftwareIgnoreResponse>>();
		vi.mocked(api.getSoftwareIgnores).mockReturnValue(ignoresDeferred.promise);

		render(IgnoreRulesTab);

		expect(screen.getByText('Loading...')).toBeInTheDocument();
		ignoresDeferred.resolve(makePage([]));

		await waitFor(() => {
			expect(document.querySelector('[data-ui="empty-state"]')).toBeInTheDocument();
		});
		expect(screen.getByRole('heading', { name: 'No ignore rules' })).toBeInTheDocument();
	});

	it('opens create flow in the shared modal shell with shared footer ordering', async () => {
		render(IgnoreRulesTab);
		await waitFor(() => expect(screen.getByText('Plex')).toBeInTheDocument());

		await fireEvent.click(screen.getByRole('button', { name: 'Add Ignore Rule' }));

		const title = await screen.findByRole('heading', { name: 'Add Ignore Rule' });
		expect(title.closest('[data-ui="modal-shell"]')).toBeInTheDocument();
		const cancelButton = screen.getByRole('button', { name: 'Cancel' });
		const createButton = screen.getByRole('button', { name: 'Create' });
		expect(cancelButton.compareDocumentPosition(createButton) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
	});

	it('keeps batch-delete confirmations on shared batch + confirm primitives', async () => {
		render(IgnoreRulesTab);
		await waitFor(() => expect(screen.getByText('Plex')).toBeInTheDocument());

		await fireEvent.click(screen.getByRole('checkbox', { name: 'Select Plex' }));
		expect(screen.getByRole('toolbar', { name: 'Batch actions' })).toBeInTheDocument();

		const toolbar = screen.getByRole('toolbar', { name: 'Batch actions' });
		await fireEvent.click(within(toolbar).getByRole('button', { name: 'Delete' }));

		const title = await screen.findByText('Batch Delete Ignore Rules');
		const dialog = title.closest('[data-ui="modal-shell"]') as HTMLElement;
		expect(dialog).toBeInTheDocument();
		expect(within(dialog).getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
		expect(within(dialog).getByRole('button', { name: 'Delete' })).toBeInTheDocument();
	});
});
