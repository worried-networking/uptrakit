import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import type { PaginatedResponse, ServiceResponse } from '$lib/types';
import { Permission } from '$lib/types';

// vi.mock calls are hoisted before imports by vitest — set them up first.
vi.mock('$lib/api', () => ({
	getServices: vi.fn(),
	approveService: vi.fn(),
	rejectService: vi.fn(),
	deleteService: vi.fn(),
	mergeService: vi.fn(),
	updateService: vi.fn(),
	batchServices: vi.fn(),
	executeBatchChunked: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null),
	getAccessToken: vi.fn(() => null)
}));

vi.mock('$lib/stores/events.svelte', () => ({
	subscribeToEvent: vi.fn(() => () => {})
}));

import ServicesPage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const adminUser = {
	id: '00000000-0000-0000-0000-000000000001',
	email: 'admin@example.com',
	first_name: 'Admin',
	last_name: 'User',
	permissions: [
		Permission.ApproveServices,
		Permission.RejectServices,
		Permission.RemoveServices,
		Permission.UpdateServices
	]
};

function makePage(items: ServiceResponse[]): PaginatedResponse<ServiceResponse> {
	return { items, total: items.length, page: 1, per_page: 25, total_pages: 1 };
}

const approvedAgent: ServiceResponse = {
	id: 'svc-001',
	friendly_name: 'prod-agent',
	capabilities: ['software_discovery', 'update_hooks', 'graceful_shutdown'],
	service_label: 'Agent',
	hostname: 'prod-host',
	is_embedded: false,
	ip_address: '10.0.0.1',
	status: 'approved',
	client_version: '1.2.0',
	last_seen_at: '2024-06-01T12:00:00Z',
	created_at: '2024-01-01T00:00:00Z',
	updated_at: '2024-01-01T00:00:00Z',
	cert_lifetime_hours: null,
	yielded_to: null
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('Services Page', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(adminUser);
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('renders the page heading when a user is logged in', async () => {
		vi.mocked(api.getServices).mockResolvedValue(makePage([]));
		render(ServicesPage);
		await waitFor(() => expect(screen.getByText('Services')).toBeInTheDocument());
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
	});

	it('renders a service row after a successful API response', async () => {
		vi.mocked(api.getServices).mockResolvedValue(makePage([approvedAgent]));
		render(ServicesPage);
		await waitFor(() => expect(screen.getByText('prod-agent')).toBeInTheDocument());
		expect(screen.getByText('prod-host')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="table-footer-bar"]')).toBeInTheDocument();
		expect(screen.getByText('1 total')).toBeInTheDocument();
	});

	it('shows the empty-state message when the service list is empty', async () => {
		vi.mocked(api.getServices).mockResolvedValue(makePage([]));
		render(ServicesPage);
		await waitFor(() => expect(screen.getByText(/No services registered yet/)).toBeInTheDocument());
	});

	it('shows an error message and a Retry button when the API call fails', async () => {
		vi.mocked(api.getServices).mockRejectedValue(new Error('Connection refused'));
		render(ServicesPage);
		await waitFor(() => expect(screen.getByText('Connection refused')).toBeInTheDocument());
		expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument();
		expect(document.querySelector('[data-ui="callout"]')).toBeInTheDocument();
	});

	it('renders nothing when no user is logged in', () => {
		vi.mocked(auth.getUser).mockReturnValue(null);
		render(ServicesPage);
		expect(screen.queryByText('Services')).not.toBeInTheDocument();
	});

	it('calls getServices with the software_discovery capability when the Agents filter button is clicked', async () => {
		vi.mocked(api.getServices).mockResolvedValue(makePage([]));
		render(ServicesPage);
		// Wait for the initial load triggered by $effect
		await waitFor(() => expect(vi.mocked(api.getServices)).toHaveBeenCalledTimes(1));

		fireEvent.click(screen.getByRole('button', { name: 'Agents' }));

		await waitFor(() =>
			expect(vi.mocked(api.getServices)).toHaveBeenCalledWith(
				expect.objectContaining({ capability: 'software_discovery' })
			)
		);
	});

	it('displays the Pending status badge for a pending service', async () => {
		vi.mocked(api.getServices).mockResolvedValue(makePage([{ ...approvedAgent, status: 'pending' }]));
		render(ServicesPage);
		await waitFor(() => expect(screen.getByText('Pending')).toBeInTheDocument());
		expect(document.querySelector('[data-ui="status-badge"]')).toBeInTheDocument();
	});

	it('displays the Approved status badge for an approved service', async () => {
		vi.mocked(api.getServices).mockResolvedValue(makePage([approvedAgent]));
		render(ServicesPage);
		await waitFor(() => expect(screen.getByText('Approved')).toBeInTheDocument());
	});

	it('shows embedded badges and hides delete for embedded services', async () => {
		vi.mocked(api.getServices).mockResolvedValue(
			makePage([
				{
					...approvedAgent,
					id: 'svc-embedded',
					friendly_name: 'embedded-agent',
					is_embedded: true,
					yielded_to: ['00000000-0000-0000-0000-000000000123']
				}
			])
		);
		render(ServicesPage);

		await waitFor(() => expect(screen.getByText('embedded-agent')).toBeInTheDocument());
		expect(screen.getByText('Embedded')).toBeInTheDocument();
		expect(screen.getByText('Yielded (1)')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="status-badge-stack"]')).toBeInTheDocument();

		await fireEvent.click(screen.getByRole('button', { name: /actions for embedded-agent/i }));
		expect(document.querySelector('[data-ui="context-menu-item"]')).toBeInTheDocument();
		expect(screen.queryByRole('menuitem', { name: /delete/i })).not.toBeInTheDocument();
	});
});
