import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import { Permission, type HostTagResponse, type PaginatedResponse } from '$lib/types';

vi.mock('$lib/api', () => ({
	getHostTags: vi.fn(),
	createHostTag: vi.fn(),
	updateHostTag: vi.fn(),
	deleteHostTag: vi.fn(),
	batchHostTags: vi.fn(),
	executeBatchChunked: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null)
}));

vi.mock('$lib/stores/events.svelte', () => ({
	subscribeToEvent: vi.fn(() => () => {})
}));

vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn()
}));

import HostTagsPage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';

const user = {
	id: '00000000-0000-0000-0000-000000000104',
	email: 'host-tags@example.com',
	first_name: 'Host',
	last_name: 'Tags',
	permissions: [Permission.UpdateHosts, Permission.DeactivateHosts]
};

function makePage(items: HostTagResponse[]): PaginatedResponse<HostTagResponse> {
	return { items, total: items.length, page: 1, per_page: 25, total_pages: 1 };
}

describe('Host Tags Route', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.getHostTags).mockResolvedValue(
			makePage([
				{
					id: 'tag-1',
					name: 'production',
					color: '#16A34A',
					description: 'Production hosts',
					host_count: 8,
					created_at: '2026-03-01T10:00:00Z'
				} as unknown as HostTagResponse
			])
		);
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('renders shared shell primitives for tag management table', async () => {
		render(HostTagsPage);

		await waitFor(() => expect(screen.getByText('Host Tags')).toBeInTheDocument());
		expect(screen.getByText('production')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
	});
});
