import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import type { HostResponse, PaginatedResponse, UpdateHistoryResponse } from '$lib/types';
import { Permission } from '$lib/types';

vi.mock('$lib/api', () => ({
	getHost: vi.fn(),
	listUpdateHistory: vi.fn(),
	updateHost: vi.fn(),
	deactivateHost: vi.fn(),
	triggerHostDiscovery: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null),
	getAccessToken: vi.fn(() => null)
}));

vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn()
}));

vi.mock('$lib/stores/events.svelte', () => ({
	subscribeToEvent: vi.fn(() => () => {})
}));

vi.mock('$lib/surfaces/registry.svelte', () => ({
	getSurfaceRuntimeStatus: vi.fn(() => ({ active: false })),
	getSurfacesBySlot: vi.fn(() => []),
	getSurfaceReadModel: vi.fn(() => undefined),
	loadSurfaceReadModels: vi.fn(() => Promise.resolve())
}));

import HostDetailPage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';
import * as surfaceRegistry from '$lib/surfaces/registry.svelte';
import { page } from '$app/state';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const adminUser = {
	id: '00000000-0000-0000-0000-000000000001',
	email: 'admin@example.com',
	first_name: 'Admin',
	last_name: 'User',
	permissions: [
		Permission.UpdateHosts,
		Permission.DeactivateHosts,
		Permission.CreateSoftware,
		Permission.UpdateSoftware,
		Permission.DeleteSoftware,
		Permission.TriggerChecks,
		Permission.TriggerUpdates
	]
};

function makeHistoryPage(items: UpdateHistoryResponse[]): PaginatedResponse<UpdateHistoryResponse> {
	return { items, total: items.length, page: 1, per_page: 5, total_pages: 1 };
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
	agents: [
		{ id: 'agent-001', friendly_name: 'Main Agent', status: 'approved' },
		{ id: 'agent-002', friendly_name: 'Backup Agent', status: 'pending' }
	],
	tags: [],
	software_status: {
		known: true,
		update_count: 0,
		error_count: 0
	}
};

const sampleHistoryEntry: UpdateHistoryResponse = {
	id: 'hist-001',
	host_id: 'host-001',
	host_name: 'prod-server',
	software_item_id: 'sw-001',
	software_item_name: 'nginx',
	from_version: '1.24.0',
	to_version: '1.25.0',
	status: 'completed',
	actor_type: 'user',
	actor_id: 'user-001',
	started_at: '2024-06-01T11:55:00Z',
	completed_at: '2024-06-01T12:00:00Z',
	output: null,
	created_at: '2024-06-01T11:54:00Z',
	interactive: false,
	output_truncated: false
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('Host Detail Page', () => {
	beforeEach(() => {
		page.params.id = 'host-001';
		vi.mocked(auth.getUser).mockReturnValue(adminUser);
		vi.mocked(api.listUpdateHistory).mockResolvedValue(makeHistoryPage([]));
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('renders the host name and hostname from the API response', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		expect(screen.getByText('prod-server')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
	});

	it('shows a loading indicator before data arrives', () => {
		vi.mocked(api.getHost).mockImplementation(() => new Promise(() => {}));
		render(HostDetailPage);
		expect(screen.getByText('Loading...')).toBeInTheDocument();
	});

	it('shows an error message and Retry button when getHost fails', async () => {
		vi.mocked(api.getHost).mockRejectedValue(new Error('Host not found'));
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByText('Host not found')).toBeInTheDocument());
		expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument();
	});

	it('renders nothing when no user is logged in', () => {
		vi.mocked(auth.getUser).mockReturnValue(null);
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		render(HostDetailPage);
		expect(screen.queryByText('Loading...')).not.toBeInTheDocument();
		expect(screen.queryByText('Production Server')).not.toBeInTheDocument();
	});

	it('renders the back link to the hosts list', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		const backLink = screen.getByRole('link', { name: /back to hosts/i });
		expect(backLink).toHaveAttribute('href', '/hosts');
	});

	it('renders host metadata fields in the info grid', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		expect(screen.getByText('Ubuntu 24.04')).toBeInTheDocument();
		expect(screen.getByText('x86_64')).toBeInTheDocument();
		expect(screen.getByText('10.0.0.5')).toBeInTheDocument();
		expect(screen.getByText('machine-abc')).toBeInTheDocument();
	});

	it('shows dashes for missing optional fields', async () => {
		vi.mocked(api.getHost).mockResolvedValue({
			...sampleHost,
			os_type: null,
			os_version: null,
			architecture: null,
			ip_address: null
		});
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		// os_version and os_type combine into one cell, so nulling all four optional
		// fields (os_type, os_version, architecture, ip_address) produces 3 dashes.
		const dashes = screen.getAllByText('—');
		expect(dashes.length).toBeGreaterThanOrEqual(3);
	});

	it('renders connected agents with their status badges', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByText('Main Agent')).toBeInTheDocument());
		expect(screen.getByText('Backup Agent')).toBeInTheDocument();
		expect(screen.getByText('approved')).toBeInTheDocument();
		expect(screen.getByText('pending')).toBeInTheDocument();
	});

	it('shows empty-state message when host has no agents', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ ...sampleHost, agents: [] });
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByText(/no agents connected/i)).toBeInTheDocument());
	});

	it('renders recent update history rows', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		vi.mocked(api.listUpdateHistory).mockResolvedValue(makeHistoryPage([sampleHistoryEntry]));
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByText('nginx')).toBeInTheDocument());
		expect(screen.getByText('1.24.0')).toBeInTheDocument();
		expect(screen.getByText('1.25.0')).toBeInTheDocument();
		expect(screen.getByText('Done')).toBeInTheDocument();
	});

	it('shows empty-state message when there is no update history', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		vi.mocked(api.listUpdateHistory).mockResolvedValue(makeHistoryPage([]));
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByText(/no update history/i)).toBeInTheDocument());
	});

	it('renders a "View all" history link pointing to /history?host_id={id}', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		const link = screen.getByRole('link', { name: /view all/i });
		expect(link).toHaveAttribute('href', '/history?host_id=host-001');
	});

	it('shows Edit Name and Deactivate buttons when user has ManageHosts permission', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		expect(screen.getByRole('button', { name: /edit name/i })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: /deactivate/i })).toBeInTheDocument();
	});

	it('hides Edit Name and Deactivate buttons when user lacks host management permissions', async () => {
		vi.mocked(auth.getUser).mockReturnValue({
			...adminUser,
			permissions: [Permission.TriggerChecks]
		});
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		expect(screen.queryByRole('button', { name: /edit name/i })).not.toBeInTheDocument();
		expect(screen.queryByRole('button', { name: /deactivate/i })).not.toBeInTheDocument();
	});

	it('shows Trigger Discovery button when user has software management permissions', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		expect(screen.getByRole('button', { name: /trigger discovery/i })).toBeInTheDocument();
	});

	it('hides Trigger Discovery button when user lacks software management permissions', async () => {
		vi.mocked(auth.getUser).mockReturnValue({
			...adminUser,
			permissions: [Permission.UpdateHosts]
		});
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		expect(screen.queryByRole('button', { name: /trigger discovery/i })).not.toBeInTheDocument();
	});

	it('calls triggerHostDiscovery and shows a success notification when plugins are queued', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		vi.mocked(api.triggerHostDiscovery).mockResolvedValue({ plugins_queued: 3, message: 'ok' });
		const { showSuccess } = await import('$lib/notifications.svelte');

		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('button', { name: /trigger discovery/i })).toBeInTheDocument());

		fireEvent.click(screen.getByRole('button', { name: /trigger discovery/i }));

		await waitFor(() => expect(vi.mocked(api.triggerHostDiscovery)).toHaveBeenCalledWith('host-001'));
		await waitFor(() => expect(vi.mocked(showSuccess)).toHaveBeenCalledWith(expect.stringContaining('3 plugin(s)')));
	});

	it('renders host-detail shared surfaces and preloads their read models', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		vi.mocked(surfaceRegistry.getSurfaceRuntimeStatus).mockReturnValue({ active: true });
		vi.mocked(surfaceRegistry.getSurfacesBySlot).mockImplementation((slot: string) => {
			if (slot !== 'host_detail.tabs') {
				return [];
			}
			return [
				{
					surface_id: 'proxmox.host-info',
					label: 'Proxmox VE Info',
					priority: 100,
					slot: 'host_detail.tabs',
					scope: 'tenant',
					targeting: 'universal',
					provider_kind: 'plugin',
					required_capabilities: [],
					root_node: {
						kind: 'key_value',
						data_source_id: 'data.remote'
					},
					provider_count: 1
				}
			];
		});

		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		expect(screen.getByText('Proxmox VE Info')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
		expect(vi.mocked(surfaceRegistry.loadSurfaceReadModels)).toHaveBeenCalledWith(['proxmox.host-info']);
	});
});
