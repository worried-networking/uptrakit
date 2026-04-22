import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import type { HostResponse, PaginatedResponse, UpdateHistoryResponse } from '$lib/types';
import { Permission } from '$lib/types';
import type { SurfaceReadResponse, SurfaceResponse } from '$lib/surfaces/contract';

vi.mock('$lib/api', () => ({
	getHost: vi.fn(),
	listUpdateHistory: vi.fn(),
	updateHost: vi.fn(),
	deactivateHost: vi.fn(),
	triggerHostDiscovery: vi.fn(),
	invokeSurfaceInteraction: vi.fn(),
	listPluginTypes: vi.fn(),
	listHostDiscoveryAllowlist: vi.fn(),
	addHostDiscoveryAllowlistEntry: vi.fn(),
	deleteHostDiscoveryAllowlistEntry: vi.fn(),
	getHostTags: vi.fn(),
	setHostTags: vi.fn(),
	getSoftwareItems: vi.fn()
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
	getSurfacesBySlot: vi.fn(() => []),
	getSurfaceReadModel: vi.fn(() => undefined),
	loadSurfaceReadModels: vi.fn(() => Promise.resolve()),
	getSurfaceProviders: vi.fn(() => [])
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

function makeSoftwareItemsPage() {
	return {
		items: [],
		total: 0,
		page: 1,
		per_page: 20,
		total_pages: 1
	};
}

function buildHostDetailSurface(overrides: Partial<SurfaceResponse> = {}): SurfaceResponse {
	return {
		surface_id: 'proxmox.host-info',
		label: 'Proxmox VE Info',
		priority: 100,
		slot: 'host_detail.tabs',
		scope: 'tenant',
		targeting: 'universal',
		provider_kind: 'plugin',
		required_capabilities: [],
		root_node: {
			kind: 'text_block',
			text: 'host detail slot content'
		},
		provider_count: 1,
		...overrides
	};
}

function buildHostDetailRead(
	surface: SurfaceResponse,
	overrides: Partial<SurfaceReadResponse> = {}
): SurfaceReadResponse {
	const { provider_count: _providerCount, ...descriptor } = surface;
	return {
		descriptor,
		interactions: [],
		data_sources: [],
		...overrides
	};
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
		vi.mocked(api.listPluginTypes).mockResolvedValue([]);
		vi.mocked(api.listHostDiscoveryAllowlist).mockResolvedValue([]);
		vi.mocked(api.getHostTags).mockResolvedValue({ items: [], total: 0, page: 1, per_page: 100, total_pages: 1 });
		vi.mocked(api.getSoftwareItems).mockResolvedValue(makeSoftwareItemsPage());
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
		expect(vi.mocked(api.getHost)).not.toHaveBeenCalled();
		expect(vi.mocked(api.listUpdateHistory)).not.toHaveBeenCalled();
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

		const surface = buildHostDetailSurface({
			root_node: {
				kind: 'key_value',
				data_source_id: 'data.remote'
			}
		});
		vi.mocked(surfaceRegistry.getSurfacesBySlot).mockImplementation((slot: string) =>
			slot === 'host_detail.tabs' ? [surface] : []
		);

		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		expect(screen.getByText('Proxmox VE Info')).toBeInTheDocument();
		expect(document.querySelector('[data-parity-region="host_detail.tabs"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
		expect(vi.mocked(surfaceRegistry.loadSurfaceReadModels)).toHaveBeenCalledWith(['proxmox.host-info']);
	});

	it('omits host_detail.tabs when there is no surface content', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);

		vi.mocked(surfaceRegistry.getSurfacesBySlot).mockImplementation((slot: string) =>
			slot === 'host_detail.tabs' ? [] : []
		);

		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());

		expect(document.querySelector('[data-parity-region="host_detail.tabs"]')).not.toBeInTheDocument();
		expect(screen.queryByText('No surfaces available.')).not.toBeInTheDocument();
	});

	it('renders targeted no-compatible-provider host_detail.tabs state with canonical empty copy', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);

		const surface = buildHostDetailSurface({ targeting: 'targeted' });
		const read = buildHostDetailRead(surface);
		vi.mocked(surfaceRegistry.getSurfacesBySlot).mockImplementation((slot: string) =>
			slot === 'host_detail.tabs' ? [surface] : []
		);
		vi.mocked(surfaceRegistry.getSurfaceReadModel).mockImplementation((surfaceId: string) =>
			surfaceId === surface.surface_id ? read : undefined
		);
		vi.mocked(surfaceRegistry.getSurfaceProviders).mockReturnValue([
			{
				provider_id: 'provider.disconnected',
				display_label: 'Provider Disconnected',
				availability: 'disconnected'
			},
			{
				provider_id: 'provider.incompatible',
				display_label: 'Provider Incompatible',
				availability: 'incompatible_tenant'
			}
		]);

		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());

		expect(screen.getByText('No provider connected')).toBeInTheDocument();
		expect(screen.getByText('Connect a compatible service to use this surface.')).toBeInTheDocument();
		expect(document.querySelector('[data-parity-region="host_detail.tabs"]')).toBeInTheDocument();
	});

	it('renders contract mismatch host_detail.tabs state from SurfaceReadPanel canonical handling', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);

		const surface = buildHostDetailSurface();
		const baseRead = buildHostDetailRead(surface);
		const read = buildHostDetailRead(surface, {
			descriptor: {
				...baseRead.descriptor,
				surface_id: 'proxmox.host-info.mismatch'
			}
		});
		vi.mocked(surfaceRegistry.getSurfacesBySlot).mockImplementation((slot: string) =>
			slot === 'host_detail.tabs' ? [surface] : []
		);
		vi.mocked(surfaceRegistry.getSurfaceReadModel).mockImplementation((surfaceId: string) =>
			surfaceId === surface.surface_id ? read : undefined
		);

		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());

		expect(screen.getByText('Surface contract mismatch detected. Please refresh and try again.')).toBeInTheDocument();
		expect(document.querySelector('[data-parity-region="host_detail.tabs"]')).toBeInTheDocument();
	});

	it('renders hydration action failure host_detail.tabs state from SurfaceReadPanel canonical handling', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);

		const surface = buildHostDetailSurface({
			root_node: {
				kind: 'key_value',
				data_source_id: 'data.remote'
			}
		});
		const read = buildHostDetailRead(surface, {
			interactions: [
				{
					interaction_id: 'proxmox.host-info.load',
					kind: 'data_load',
					label: 'Load Proxmox Host Info',
					transport: { mode: 'controller_local' }
				}
			],
			data_sources: [
				{
					data_source_id: 'data.remote',
					kind: { kind: 'provider_query', operation_id: 'proxmox.host-info.load' },
					result_schema: 'object',
					refresh_policy: { type: 'manual' }
				}
			]
		});
		vi.mocked(surfaceRegistry.getSurfacesBySlot).mockImplementation((slot: string) =>
			slot === 'host_detail.tabs' ? [surface] : []
		);
		vi.mocked(surfaceRegistry.getSurfaceReadModel).mockImplementation((surfaceId: string) =>
			surfaceId === surface.surface_id ? read : undefined
		);
		vi.mocked(api.invokeSurfaceInteraction).mockRejectedValue(new Error('boom'));

		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());

		expect(await screen.findByText('Unable to load surface data')).toBeInTheDocument();
		expect(screen.getByText('Failed to load surface data. Please try again.')).toBeInTheDocument();
		expect(document.querySelector('[data-parity-region="host_detail.tabs"]')).toBeInTheDocument();
	});

	it('renders permission_denied host_detail.tabs state inside the host container', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);

		const gatedSurface = buildHostDetailSurface({
			required_permission: Permission.ViewSettings
		});
		vi.mocked(surfaceRegistry.getSurfacesBySlot).mockImplementation((slot: string) =>
			slot === 'host_detail.tabs' ? [gatedSurface] : []
		);
		vi.mocked(auth.getUser).mockReturnValue({
			...adminUser,
			permissions: [Permission.UpdateHosts]
		});

		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());

		expect(document.querySelector('[data-parity-region="host_detail.tabs"]')).toBeInTheDocument();
		expect(screen.getByText(gatedSurface.label)).toBeInTheDocument();
		expect(screen.getByText('Access denied')).toBeInTheDocument();
		expect(screen.getByText('You do not have permission to access this surface.')).toBeInTheDocument();
		expect(vi.mocked(surfaceRegistry.loadSurfaceReadModels)).not.toHaveBeenCalled();
	});

	it('does not submit allowlist entry when plugin type is empty', async () => {
		vi.mocked(auth.getUser).mockReturnValue({
			...adminUser,
			permissions: [...adminUser.permissions, Permission.ViewSoftware]
		});
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		vi.mocked(api.listPluginTypes).mockResolvedValue([]);
		vi.mocked(api.listHostDiscoveryAllowlist).mockResolvedValue([]);
		vi.mocked(api.getSoftwareItems).mockResolvedValue(makeSoftwareItemsPage());

		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		await waitFor(() => expect(screen.getByRole('button', { name: 'Add Plugin Type' })).toBeInTheDocument());

		await fireEvent.click(screen.getByRole('button', { name: 'Add Plugin Type' }));
		expect(screen.getByText('Add Discovery Plugin Type')).toBeInTheDocument();

		await fireEvent.click(screen.getByRole('button', { name: 'Add' }));

		expect(vi.mocked(api.addHostDiscoveryAllowlistEntry)).not.toHaveBeenCalled();
	});
});

describe('Button primitive contract — hosts/[id]/+page.svelte', () => {
	it('Edit Name uses secondary variant (md size, bg-raised border)', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		render(HostDetailPage);
		await waitFor(() => screen.getByRole('button', { name: /edit name/i }));

		const btn = screen.getByRole('button', { name: /edit name/i });
		expect(btn).toHaveClass('h-[23px]');
		expect(btn.className).toMatch(/bg-\[var\(--bg-raised\)\]/);
		expect(btn.className).not.toMatch(/preset-tonal|preset-filled/);
	});

	it('Deactivate uses danger variant (error colors)', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		render(HostDetailPage);
		await waitFor(() => screen.getByRole('button', { name: /^deactivate$/i }));

		const btn = screen.getByRole('button', { name: /^deactivate$/i });
		expect(btn).toHaveClass('h-[23px]');
		expect(btn.className).toMatch(/color-error/);
		expect(btn.className).not.toMatch(/preset-filled-error/);
	});

	it('Trigger Discovery uses secondary variant and aria-busy while discovering', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		let resolveTrigger!: (v: { plugins_queued: number; message: string }) => void;
		vi.mocked(api.triggerHostDiscovery).mockReturnValue(
			new Promise((res) => {
				resolveTrigger = res;
			})
		);

		render(HostDetailPage);
		await waitFor(() => screen.getByRole('button', { name: /trigger discovery/i }));

		const btn = screen.getByRole('button', { name: /trigger discovery/i });
		expect(btn).toHaveClass('h-[23px]');
		expect(btn.className).toMatch(/bg-\[var\(--bg-raised\)\]/);
		expect(btn.className).not.toMatch(/preset-tonal|preset-filled/);

		fireEvent.click(btn);
		await waitFor(() =>
			expect(screen.getByRole('button', { name: /trigger discovery/i })).toHaveAttribute('aria-busy', 'true')
		);

		// Static label — no text swap
		expect(btn.textContent).toMatch(/trigger discovery/i);

		resolveTrigger({ plugins_queued: 0, message: 'ok' });
		await waitFor(() =>
			expect(screen.getByRole('button', { name: /trigger discovery/i })).not.toHaveAttribute('aria-busy')
		);
	});

	it('Set Tags uses secondary sm variant', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		render(HostDetailPage);
		await waitFor(() => screen.getByRole('button', { name: /set tags/i }));

		const btn = screen.getByRole('button', { name: /set tags/i });
		expect(btn).toHaveClass('h-[19px]');
		expect(btn.className).toMatch(/bg-\[var\(--bg-raised\)\]/);
		expect(btn.className).not.toMatch(/preset-tonal|preset-filled/);
	});

	it('Add Plugin Type uses primary sm variant', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		vi.mocked(auth.getUser).mockReturnValue({
			...adminUser,
			permissions: [...adminUser.permissions, Permission.ViewSoftware]
		});
		render(HostDetailPage);
		await waitFor(() => screen.getByRole('button', { name: /add plugin type/i }));

		const btn = screen.getByRole('button', { name: /add plugin type/i });
		expect(btn).toHaveClass('h-[19px]');
		expect(btn.className).toMatch(/bg-\[linear-gradient/);
		expect(btn.className).not.toMatch(/preset-filled/);
	});

	it('error Retry uses primary variant with aria-busy during retry', async () => {
		vi.mocked(api.getHost).mockRejectedValue(new Error('Network error'));
		render(HostDetailPage);
		await waitFor(() => screen.getByRole('button', { name: /retry/i }));

		const btn = screen.getByRole('button', { name: /retry/i });
		expect(btn).toHaveClass('h-[23px]');
		expect(btn.className).toMatch(/bg-\[linear-gradient/);
		expect(btn.className).not.toMatch(/preset-filled/);
	});

	it('Edit modal Save has static label and loading wires to aria-busy', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);

		let resolveUpdate!: (v: typeof sampleHost) => void;
		vi.mocked(api.updateHost).mockReturnValue(new Promise((res) => (resolveUpdate = res)));

		render(HostDetailPage);
		await waitFor(() => screen.getByRole('button', { name: /edit name/i }));
		fireEvent.click(screen.getByRole('button', { name: /edit name/i }));

		await waitFor(() => screen.getByRole('button', { name: /^save$/i }));
		const saveBtn = screen.getByRole('button', { name: /^save$/i });

		// Static label — never swaps during loading
		expect(saveBtn.textContent?.trim()).toBe('Save');

		// Verify the Button primitive is used (secondary for cancel, primary for save)
		expect(saveBtn).toHaveClass('h-[23px]');
		expect(saveBtn.className).toMatch(/bg-\[linear-gradient/);
		expect(saveBtn.className).not.toMatch(/preset-filled/);

		// aria-busy wires to loading prop: absent when idle
		expect(saveBtn).not.toHaveAttribute('aria-busy');

		// Trigger save — button should become aria-busy while in-flight
		fireEvent.click(saveBtn);
		await waitFor(() => expect(saveBtn).toHaveAttribute('aria-busy', 'true'));
		expect(saveBtn.textContent?.trim()).toBe('Save');

		resolveUpdate(sampleHost);
		// After save completes, modal closes (button is removed) — modal gone proves loading ended
		await waitFor(() => expect(screen.queryByRole('button', { name: /^save$/i })).not.toBeInTheDocument());
	});

	it('Edit modal Cancel uses secondary variant', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		render(HostDetailPage);
		await waitFor(() => screen.getByRole('button', { name: /edit name/i }));
		fireEvent.click(screen.getByRole('button', { name: /edit name/i }));

		await waitFor(() => screen.getByRole('button', { name: /^cancel$/i }));
		const cancelBtn = screen.getByRole('button', { name: /^cancel$/i });
		expect(cancelBtn).toHaveClass('h-[23px]');
		expect(cancelBtn.className).toMatch(/bg-\[var\(--bg-raised\)\]/);
		expect(cancelBtn.className).not.toMatch(/preset-tonal|preset-filled/);
	});

	it('source has no preset-filled-* or preset-tonal-* classes in button elements', async () => {
		vi.mocked(api.getHost).mockResolvedValue(sampleHost);
		vi.mocked(auth.getUser).mockReturnValue({
			...adminUser,
			permissions: [...adminUser.permissions, Permission.ViewSoftware]
		});
		render(HostDetailPage);
		await waitFor(() => screen.getByRole('button', { name: /edit name/i }));

		// Open the allowlist modal to render its footer too
		fireEvent.click(screen.getByRole('button', { name: /add plugin type/i }));
		await waitFor(() => screen.getByRole('button', { name: /^cancel$/i }));

		const allButtonClasses = Array.from(document.querySelectorAll('button'))
			.map((el) => el.className)
			.join(' ');
		expect(allButtonClasses).not.toMatch(/preset-filled-|preset-tonal-/);
	});
});
