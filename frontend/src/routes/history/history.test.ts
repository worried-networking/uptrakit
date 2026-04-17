import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { Permission, type UpdateHistoryResponse } from '$lib/types';

vi.mock('$lib/api', () => ({
	listUpdateHistory: vi.fn(),
	triggerSoftwareUpdate: vi.fn(),
	getSoftwareItems: vi.fn(),
	getUpdateHistoryEntry: vi.fn(),
	getSoftwareItem: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null)
}));

vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn()
}));

vi.mock('$lib/interactive', () => ({
	connectInteractiveSession: vi.fn(() => ({
		disconnect: vi.fn(),
		sendSignal: vi.fn(),
		sendInput: vi.fn()
	}))
}));

vi.mock('$lib/sse', () => ({
	connectEventStream: vi.fn(() => () => {})
}));

import HistoryPage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';

const user = {
	id: '00000000-0000-0000-0000-000000000102',
	email: 'history@example.com',
	first_name: 'History',
	last_name: 'User',
	permissions: [Permission.ViewSoftware, Permission.TriggerUpdates]
};

const queuedItem: UpdateHistoryResponse = {
	id: 'hist-queued',
	host_name: 'host-a',
	software_item_name: 'nginx',
	from_version: '1.24.0',
	to_version: '1.25.0',
	status: 'queued',
	started_at: '2026-02-01T10:00:00Z',
	completed_at: null,
	output: '',
	output_truncated: true,
	interactive: false,
	actor_type: 'user',
	actor_id: 'actor-1'
} as unknown as UpdateHistoryResponse;

describe('History Route', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.listUpdateHistory).mockResolvedValue({
			items: [queuedItem],
			total: 1,
			page: 1,
			per_page: 25,
			total_pages: 1
		});
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('uses shared page shell and table primitives', async () => {
		render(HistoryPage);

		await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="status-badge"]')).toBeInTheDocument();
	});

	it('uses shared callouts for queued output and truncation metadata in expanded rows', async () => {
		render(HistoryPage);

		const rowCell = await screen.findByText('host-a');
		await fireEvent.click(rowCell);

		await waitFor(() => expect(screen.getByText(/waiting for another update/i)).toBeInTheDocument());
		expect(document.querySelectorAll('[data-ui="callout"]').length).toBeGreaterThanOrEqual(1);
	});
});
