import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import { Permission, type PaginatedResponse, type ServiceResponse, type UpdateHistoryResponse } from '$lib/types';

vi.mock('$lib/api', () => ({
	getHosts: vi.fn(),
	getServices: vi.fn(),
	getSoftwareItems: vi.fn(),
	listUpdateHistory: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null)
}));

vi.mock('$lib/stores/events.svelte', () => ({
	subscribeToEvent: vi.fn(() => () => {})
}));

import HomePage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';

const user = {
	id: '00000000-0000-0000-0000-000000000101',
	email: 'home@example.com',
	first_name: 'Home',
	last_name: 'User',
	permissions: [Permission.ViewHosts, Permission.ViewServices, Permission.ViewSoftware]
};

function makeServices(items: ServiceResponse[]): PaginatedResponse<ServiceResponse> {
	return { items, total: items.length, page: 1, per_page: 100, total_pages: 1 };
}

describe('Dashboard Route', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.getHosts).mockResolvedValue({ items: [], total: 3, page: 1, per_page: 1, total_pages: 1 });
		vi.mocked(api.getServices).mockResolvedValue(makeServices([{ status: 'pending' } as unknown as ServiceResponse]));
		vi.mocked(api.getSoftwareItems).mockImplementation(async (_page = 1, _perPage = 1, featured = true) => ({
			items: [],
			total: featured ? 7 : 2,
			page: 1,
			per_page: 1,
			total_pages: 1
		}));
		vi.mocked(api.listUpdateHistory).mockResolvedValue({
			items: [
				{
					id: 'hist-1',
					software_item_name: 'nginx',
					host_name: 'prod-01',
					status: 'failed',
					created_at: '2026-01-01T10:00:00Z'
				} as unknown as UpdateHistoryResponse
			],
			total: 1,
			page: 1,
			per_page: 5,
			total_pages: 1
		});
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('renders shared page shell primitives for dashboard content', async () => {
		render(HomePage);

		await waitFor(() => expect(screen.getByText('Dashboard')).toBeInTheDocument());
		await waitFor(() => expect(screen.getByText('Updates pending')).toBeInTheDocument());
		await waitFor(() => expect(screen.getByText('nginx')).toBeInTheDocument());
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="status-badge"]')).toBeInTheDocument();
	});

	it('renders shared callouts for attention items', async () => {
		render(HomePage);

		await waitFor(() => expect(screen.getByText('Attention Needed')).toBeInTheDocument());
		expect(document.querySelector('[data-ui="callout"]')).toBeInTheDocument();
	});
});
