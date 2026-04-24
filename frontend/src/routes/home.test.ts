import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import { Permission, type PaginatedResponse, type ServiceResponse, type UpdateHistoryResponse } from '$lib/types';
import homeSource from './+page.svelte?raw';

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
	has_pending_email_change: false,
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

	it('Retry button renders as primary Button with mt-3 class', async () => {
		vi.mocked(api.getHosts).mockRejectedValue(new Error('fail'));
		vi.mocked(api.getServices).mockRejectedValue(new Error('fail'));
		vi.mocked(api.getSoftwareItems).mockRejectedValue(new Error('fail'));
		vi.mocked(api.listUpdateHistory).mockRejectedValue(new Error('fail'));
		render(HomePage);
		await waitFor(() => expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument());

		const retryBtn = screen.getByRole('button', { name: /retry/i });
		expect(retryBtn.className).toContain('h-[23px]');
		expect(retryBtn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
		expect(retryBtn.className).toContain('mt-3');
	});

	it('"Review" action link renders as ghost Button href', async () => {
		render(HomePage);
		await waitFor(() => expect(screen.getByText('Attention Needed')).toBeInTheDocument());

		const reviewAnchor = document.querySelector('a[href="/services?status=pending"]') as HTMLElement;
		expect(reviewAnchor).not.toBeNull();
		expect(reviewAnchor.className).toContain('h-[19px]'); // size="sm"
		expect(reviewAnchor.className).toContain('bg-transparent');
		expect(reviewAnchor.className).not.toContain('preset-tonal');
	});

	it('"Investigate" action link renders as ghost Button href', async () => {
		render(HomePage);
		await waitFor(() => expect(screen.getByText('Attention Needed')).toBeInTheDocument());

		// Use role="button" to target the Button component anchor, not the stat card anchor
		const investigateAnchor = document.querySelector('a[href="/history?status=failed"][role="button"]') as HTMLElement;
		expect(investigateAnchor).not.toBeNull();
		expect(investigateAnchor.className).toContain('h-[19px]'); // size="sm"
		expect(investigateAnchor.className).toContain('bg-transparent');
	});

	it('"View all" action link renders as ghost Button href', async () => {
		// total > 5 is required for View all to render
		vi.mocked(api.listUpdateHistory).mockResolvedValue({
			items: Array.from({ length: 5 }, (_, i) => ({
				id: `hist-${i}`,
				software_item_name: `pkg-${i}`,
				host_name: 'host',
				status: 'completed',
				created_at: '2026-01-01T10:00:00Z'
			})) as unknown as UpdateHistoryResponse[],
			total: 10,
			page: 1,
			per_page: 5,
			total_pages: 2
		});
		render(HomePage);
		await waitFor(() => expect(screen.getByText('Recent Updates')).toBeInTheDocument());
		await waitFor(() => expect(document.querySelector('a[href="/history"]')).not.toBeNull());

		const viewAllAnchor = document.querySelector('a[href="/history"]') as HTMLElement;
		expect(viewAllAnchor).not.toBeNull();
		expect(viewAllAnchor.className).toContain('h-[19px]'); // size="sm"
		expect(viewAllAnchor.className).toContain('bg-transparent');
	});

	it('home page source contains no preset-filled-* or preset-tonal-* class strings', () => {
		expect(homeSource).not.toMatch(/preset-filled-/);
		expect(homeSource).not.toMatch(/preset-tonal-/);
	});
});
