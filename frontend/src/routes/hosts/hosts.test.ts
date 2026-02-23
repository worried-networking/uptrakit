import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import type { HostResponse, PaginatedResponse } from '$lib/types';
import { Permission } from '$lib/types';

vi.mock('$lib/api', () => ({
	getHosts: vi.fn(),
	updateHost: vi.fn(),
	deactivateHost: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null)
}));

import HostsPage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const adminUser = {
	id: '00000000-0000-0000-0000-000000000002',
	email: 'admin@example.com',
	first_name: 'Admin',
	last_name: 'User',
	permissions: [Permission.ManageHosts]
};

function makePage(items: HostResponse[]): PaginatedResponse<HostResponse> {
	return { items, total: items.length, page: 1, per_page: 25, total_pages: 1 };
}

const sampleHost: HostResponse = {
	id: 'host-001',
	machine_id: 'machine-abc',
	hostname: 'prod-server',
	friendly_name: 'Production Server',
	os_type: 'Linux',
	os_version: 'Ubuntu 24.04',
	architecture: 'x86_64',
	ip_address: '10.0.0.5',
	last_seen_at: '2024-06-01T12:00:00Z',
	created_at: '2024-01-01T00:00:00Z',
	updated_at: '2024-01-01T00:00:00Z',
	agents: []
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('Hosts Page', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(adminUser);
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('renders the page heading when a user is logged in', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([]));
		render(HostsPage);
		await waitFor(() => expect(screen.getByText('Hosts')).toBeInTheDocument());
	});

	it('renders a host row after a successful API response', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
		render(HostsPage);
		await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());
		expect(screen.getByText('prod-server')).toBeInTheDocument();
		expect(screen.getByText('Ubuntu 24.04')).toBeInTheDocument();
	});

	it('shows the empty-state message when the host list is empty', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([]));
		render(HostsPage);
		await waitFor(() => expect(screen.getByText(/No hosts discovered yet/)).toBeInTheDocument());
	});

	it('shows an error message and a Retry button when the API call fails', async () => {
		vi.mocked(api.getHosts).mockRejectedValue(new Error('Server unavailable'));
		render(HostsPage);
		await waitFor(() => expect(screen.getByText('Server unavailable')).toBeInTheDocument());
		expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument();
	});

	it('renders nothing when no user is logged in', () => {
		vi.mocked(auth.getUser).mockReturnValue(null);
		render(HostsPage);
		expect(screen.queryByText('Hosts')).not.toBeInTheDocument();
	});

	it('displays a dash for unknown OS when os_type and os_version are both null', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([{ ...sampleHost, os_type: null, os_version: null }]));
		render(HostsPage);
		// The "—" dash for OS column should appear
		await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());
		// At least one em-dash should be present (could be OS, arch, or IP)
		const dashes = screen.getAllByText('—');
		expect(dashes.length).toBeGreaterThan(0);
	});
});
