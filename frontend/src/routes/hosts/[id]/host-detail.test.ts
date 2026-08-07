import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import type { HostResponse, PaginatedResponse, UpdateHistoryResponse } from '$lib/api';
import { Actions } from '$lib/api';
import type { SurfaceReadResponse, SurfaceResponse } from '$lib/surfaces/contract';

vi.mock('$lib/api', async (importOriginal) => ({
	...(await importOriginal<typeof import('$lib/api')>()),
	getHost: vi.fn(),
	listUpdateHistory: vi.fn(),
	updateHost: vi.fn(),
	deactivateHost: vi.fn(),
	discoverHost: vi.fn(),
	invokeSurfaceInteraction: vi.fn(),
	readSurfaceInteraction: vi.fn(),
	listPluginTypes: vi.fn(),
	listHostDiscoveryAllowlist: vi.fn(),
	addHostDiscoveryAllowlistEntry: vi.fn(),
	removeHostDiscoveryAllowlistEntry: vi.fn(),
	listHostTags: vi.fn(),
	setHostTags: vi.fn(),
	listSoftwareItems: vi.fn()
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
	refreshSurfaceReadModel: vi.fn(() => Promise.resolve()),
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
	has_pending_email_change: false,
	actions: [
		Actions.HOSTS_UPDATE,
		Actions.HOSTS_DELETE,
		Actions.SOFTWARE_CREATE,
		Actions.SOFTWARE_UPDATE,
		Actions.SOFTWARE_DELETE,
		Actions.CHECKS_TRIGGER,
		Actions.UPDATES_TRIGGER
	],
	authority: 'ok' as const
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

const sampleHost2: HostResponse = {
	id: 'host-002',
	machine_id: 'machine-def',
	hostname: 'staging-server',
	friendly_name: 'Staging Server',
	os_type: 'Linux',
	os_version: 'Ubuntu 22.04',
	architecture: 'aarch64',
	ip_address: '10.0.0.6',
	last_seen_at: '2024-06-02T12:00:00Z',
	created_at: '2024-01-02T00:00:00Z',
	updated_at: '2024-01-02T00:00:00Z',
	agents: [],
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
	output: '',
	created_at: '2024-06-01T11:54:00Z',
	interactive: false,
	output_truncated: false,
	update_category: 'unknown'
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('Host Detail Page', () => {
	beforeEach(() => {
		page.params.id = 'host-001';
		vi.mocked(auth.getUser).mockReturnValue(adminUser);
		vi.mocked(api.listUpdateHistory).mockResolvedValue({ data: makeHistoryPage([]) } as unknown as Awaited<
			ReturnType<typeof api.listUpdateHistory>
		>);
		vi.mocked(api.listPluginTypes).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listPluginTypes>
		>);
		vi.mocked(api.listHostDiscoveryAllowlist).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listHostDiscoveryAllowlist>
		>);
		vi.mocked(api.listHostTags).mockResolvedValue({
			data: { items: [], total: 0, page: 1, per_page: 100, total_pages: 1 }
		} as unknown as Awaited<ReturnType<typeof api.listHostTags>>);
		vi.mocked(api.listSoftwareItems).mockResolvedValue({ data: makeSoftwareItemsPage() } as unknown as Awaited<
			ReturnType<typeof api.listSoftwareItems>
		>);
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('renders the host name and hostname from the API response', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		expect(screen.getByText('prod-server')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
	});

	it('shows a loading indicator before data arrives', () => {
		vi.mocked(api.getHost).mockImplementation(() => new Promise(() => {}) as ReturnType<typeof api.getHost>);
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
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		render(HostDetailPage);
		expect(screen.queryByText('Loading...')).not.toBeInTheDocument();
		expect(screen.queryByText('Production Server')).not.toBeInTheDocument();
		expect(vi.mocked(api.getHost)).not.toHaveBeenCalled();
		expect(vi.mocked(api.listUpdateHistory)).not.toHaveBeenCalled();
	});

	it('renders the back link to the hosts list', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		const backLink = screen.getByRole('link', { name: /back to hosts/i });
		expect(backLink).toHaveAttribute('href', '/hosts');
	});

	it('renders host metadata fields in the info grid', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		expect(screen.getByText('Ubuntu 24.04')).toBeInTheDocument();
		expect(screen.getByText('x86_64')).toBeInTheDocument();
		expect(screen.getByText('10.0.0.5')).toBeInTheDocument();
		expect(screen.getByText('machine-abc')).toBeInTheDocument();
	});

	it('shows dashes for missing optional fields', async () => {
		vi.mocked(api.getHost).mockResolvedValue({
			data: {
				...sampleHost,
				os_type: null,
				os_version: null,
				architecture: null,
				ip_address: null
			}
		} as unknown as Awaited<ReturnType<typeof api.getHost>>);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		// os_version and os_type combine into one cell, so nulling all four optional
		// fields (os_type, os_version, architecture, ip_address) produces 3 dashes.
		const dashes = screen.getAllByText('—');
		expect(dashes.length).toBeGreaterThanOrEqual(3);
	});

	it('renders connected agents with their status badges', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByText('Main Agent')).toBeInTheDocument());
		expect(screen.getByText('Backup Agent')).toBeInTheDocument();
		expect(screen.getByText('approved')).toBeInTheDocument();
		expect(screen.getByText('pending')).toBeInTheDocument();
	});

	it('shows empty-state message when host has no agents', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: { ...sampleHost, agents: [] } } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByText(/no agents connected/i)).toBeInTheDocument());
	});

	it('renders recent update history rows', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		vi.mocked(api.listUpdateHistory).mockResolvedValue({
			data: makeHistoryPage([sampleHistoryEntry])
		} as unknown as Awaited<ReturnType<typeof api.listUpdateHistory>>);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByText('nginx')).toBeInTheDocument());
		expect(screen.getByText('1.24.0')).toBeInTheDocument();
		expect(screen.getByText('1.25.0')).toBeInTheDocument();
		expect(screen.getByText('Done')).toBeInTheDocument();
	});

	it('shows empty-state message when there is no update history', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		vi.mocked(api.listUpdateHistory).mockResolvedValue({ data: makeHistoryPage([]) } as unknown as Awaited<
			ReturnType<typeof api.listUpdateHistory>
		>);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByText(/no update history/i)).toBeInTheDocument());
	});

	it('renders a "View all" history link pointing to /history?host_id={id}', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		const link = screen.getByRole('link', { name: /view all/i });
		expect(link).toHaveAttribute('href', '/history?host_id=host-001');
	});

	it('shows Edit Name and Deactivate buttons when user has ManageHosts permission', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		expect(screen.getByRole('button', { name: /edit name/i })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: /deactivate/i })).toBeInTheDocument();
	});

	it('hides Edit Name and Deactivate buttons when user lacks host management permissions', async () => {
		vi.mocked(auth.getUser).mockReturnValue({
			...adminUser,
			actions: [Actions.CHECKS_TRIGGER]
		});
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		expect(screen.queryByRole('button', { name: /edit name/i })).not.toBeInTheDocument();
		expect(screen.queryByRole('button', { name: /deactivate/i })).not.toBeInTheDocument();
	});

	it('shows Trigger Discovery button when user has software management permissions', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		expect(screen.getByRole('button', { name: /trigger discovery/i })).toBeInTheDocument();
	});

	it('hides Trigger Discovery button when user lacks software management permissions', async () => {
		vi.mocked(auth.getUser).mockReturnValue({
			...adminUser,
			actions: [Actions.HOSTS_UPDATE]
		});
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		expect(screen.queryByRole('button', { name: /trigger discovery/i })).not.toBeInTheDocument();
	});

	it('calls triggerHostDiscovery and shows a success notification when plugins are queued', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		vi.mocked(api.discoverHost).mockResolvedValue({ data: { plugins_queued: 3, message: 'ok' } } as unknown as Awaited<
			ReturnType<typeof api.discoverHost>
		>);
		const { showSuccess } = await import('$lib/notifications.svelte');

		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('button', { name: /trigger discovery/i })).toBeInTheDocument());

		fireEvent.click(screen.getByRole('button', { name: /trigger discovery/i }));

		await waitFor(() => expect(vi.mocked(api.discoverHost)).toHaveBeenCalledWith({ path: { id: 'host-001' } }));
		await waitFor(() => expect(vi.mocked(showSuccess)).toHaveBeenCalledWith(expect.stringContaining('3 plugin(s)')));
	});

	it('renders host-detail shared surfaces and preloads their read models', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);

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
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);

		vi.mocked(surfaceRegistry.getSurfacesBySlot).mockImplementation((slot: string) =>
			slot === 'host_detail.tabs' ? [] : []
		);

		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());

		expect(document.querySelector('[data-parity-region="host_detail.tabs"]')).not.toBeInTheDocument();
		expect(screen.queryByText('No surfaces available.')).not.toBeInTheDocument();
	});

	it('renders targeted no-compatible-provider host_detail.tabs state with canonical empty copy', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);

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
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);

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
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);

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
					transport: { mode: 'controller_local' },
					http_method: 'get'
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
		vi.mocked(api.readSurfaceInteraction).mockRejectedValue(new Error('boom'));

		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());

		expect(await screen.findByText('Unable to load surface data')).toBeInTheDocument();
		expect(screen.getByText('Failed to load surface data. Please try again.')).toBeInTheDocument();
		expect(document.querySelector('[data-parity-region="host_detail.tabs"]')).toBeInTheDocument();
	});

	it('renders permission_denied host_detail.tabs state inside the host container', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);

		const gatedSurface = buildHostDetailSurface({
			required_action: Actions.SETTINGS_READ
		});
		vi.mocked(surfaceRegistry.getSurfacesBySlot).mockImplementation((slot: string) =>
			slot === 'host_detail.tabs' ? [gatedSurface] : []
		);
		vi.mocked(auth.getUser).mockReturnValue({
			...adminUser,
			actions: [Actions.HOSTS_UPDATE]
		});

		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());

		expect(document.querySelector('[data-parity-region="host_detail.tabs"]')).toBeInTheDocument();
		expect(screen.getByText(gatedSurface.label)).toBeInTheDocument();
		expect(screen.getByText('Access denied')).toBeInTheDocument();
		expect(screen.getByText('You do not have permission to access this surface.')).toBeInTheDocument();
		expect(vi.mocked(surfaceRegistry.loadSurfaceReadModels)).not.toHaveBeenCalled();
	});

	// M1.7 fix: `required_action` is a typed catalog action string (`resource:verb`), and
	// `User.actions` now carries the same catalog vocabulary (server-expanded via
	// AccessEngine::allowed_actions()). The SPA's client-side gate compares them literally,
	// so an action-gated surface is now visible to a fully-privileged admin fixture user
	// that actually holds the required action.
	it('shows an action-gated surface for the admin fixture user who holds the required action', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);

		const gatedSurface = buildHostDetailSurface({
			required_action: 'hosts:update'
		});
		vi.mocked(surfaceRegistry.getSurfacesBySlot).mockImplementation((slot: string) =>
			slot === 'host_detail.tabs' ? [gatedSurface] : []
		);
		vi.mocked(auth.getUser).mockReturnValue(adminUser);

		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());

		expect(document.querySelector('[data-parity-region="host_detail.tabs"]')).toBeInTheDocument();
		expect(screen.getByText(gatedSurface.label)).toBeInTheDocument();
		expect(screen.queryByText('Access denied')).not.toBeInTheDocument();
		expect(vi.mocked(surfaceRegistry.loadSurfaceReadModels)).toHaveBeenCalledWith([gatedSurface.surface_id]);
	});

	it('does not submit allowlist entry when plugin type is empty', async () => {
		vi.mocked(auth.getUser).mockReturnValue({
			...adminUser,
			actions: [...adminUser.actions, Actions.SOFTWARE_READ]
		});
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		vi.mocked(api.listPluginTypes).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listPluginTypes>
		>);
		vi.mocked(api.listHostDiscoveryAllowlist).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listHostDiscoveryAllowlist>
		>);
		vi.mocked(api.listSoftwareItems).mockResolvedValue({ data: makeSoftwareItemsPage() } as unknown as Awaited<
			ReturnType<typeof api.listSoftwareItems>
		>);

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
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		render(HostDetailPage);
		await waitFor(() => screen.getByRole('button', { name: /edit name/i }));

		const btn = screen.getByRole('button', { name: /edit name/i });
		expect(btn).toHaveClass('h-[23px]');
		expect(btn.className).toMatch(/bg-\[var\(--bg-raised\)\]/);
		expect(btn.className).not.toMatch(/preset-tonal|preset-filled/);
	});

	it('Deactivate uses danger variant (error colors)', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		render(HostDetailPage);
		await waitFor(() => screen.getByRole('button', { name: /^deactivate$/i }));

		const btn = screen.getByRole('button', { name: /^deactivate$/i });
		expect(btn).toHaveClass('h-[23px]');
		expect(btn.className).toMatch(/color-danger/);
		expect(btn.className).not.toMatch(/preset-filled-error/);
	});

	it('Trigger Discovery uses secondary variant and aria-busy while discovering', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		let resolveTrigger!: (v: Awaited<ReturnType<typeof api.discoverHost>>) => void;
		vi.mocked(api.discoverHost).mockReturnValue(
			new Promise((res) => {
				resolveTrigger = res;
			}) as ReturnType<typeof api.discoverHost>
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

		resolveTrigger({ data: { plugins_queued: 0, message: 'ok' } } as unknown as Awaited<
			ReturnType<typeof api.discoverHost>
		>);
		await waitFor(() =>
			expect(screen.getByRole('button', { name: /trigger discovery/i })).not.toHaveAttribute('aria-busy')
		);
	});

	it('Set Tags uses secondary sm variant', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		render(HostDetailPage);
		await waitFor(() => screen.getByRole('button', { name: /set tags/i }));

		const btn = screen.getByRole('button', { name: /set tags/i });
		expect(btn).toHaveClass('h-[19px]');
		expect(btn.className).toMatch(/bg-\[var\(--bg-raised\)\]/);
		expect(btn.className).not.toMatch(/preset-tonal|preset-filled/);
	});

	it('Add Plugin Type uses primary sm variant', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		vi.mocked(auth.getUser).mockReturnValue({
			...adminUser,
			actions: [...adminUser.actions, Actions.SOFTWARE_READ]
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
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);

		let resolveUpdate!: (v: Awaited<ReturnType<typeof api.updateHost>>) => void;
		vi.mocked(api.updateHost).mockReturnValue(
			new Promise((res) => (resolveUpdate = res)) as ReturnType<typeof api.updateHost>
		);

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

		resolveUpdate({ data: sampleHost } as unknown as Awaited<ReturnType<typeof api.updateHost>>);
		// After save completes, modal closes (button is removed) — modal gone proves loading ended
		await waitFor(() => expect(screen.queryByRole('button', { name: /^save$/i })).not.toBeInTheDocument());
	});

	it('Edit modal Cancel uses secondary variant', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
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
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);
		vi.mocked(auth.getUser).mockReturnValue({
			...adminUser,
			actions: [...adminUser.actions, Actions.SOFTWARE_READ]
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

describe('Host Detail Page — param-only navigation reload', () => {
	beforeEach(() => {
		page.params.id = 'host-001';
		vi.mocked(auth.getUser).mockReturnValue(adminUser);
		vi.mocked(api.listUpdateHistory).mockResolvedValue({ data: makeHistoryPage([]) } as unknown as Awaited<
			ReturnType<typeof api.listUpdateHistory>
		>);
		vi.mocked(api.listPluginTypes).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listPluginTypes>
		>);
		vi.mocked(api.listHostDiscoveryAllowlist).mockResolvedValue({ data: [] } as unknown as Awaited<
			ReturnType<typeof api.listHostDiscoveryAllowlist>
		>);
		vi.mocked(api.listHostTags).mockResolvedValue({
			data: { items: [], total: 0, page: 1, per_page: 100, total_pages: 1 }
		} as unknown as Awaited<ReturnType<typeof api.listHostTags>>);
		vi.mocked(api.listSoftwareItems).mockResolvedValue({ data: makeSoftwareItemsPage() } as unknown as Awaited<
			ReturnType<typeof api.listSoftwareItems>
		>);
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('re-fetches the host when the route id param changes', async () => {
		vi.mocked(api.getHost).mockImplementation(
			({ path }: { path: { id: string } }) =>
				Promise.resolve({
					data: path.id === 'host-002' ? sampleHost2 : sampleHost
				}) as ReturnType<typeof api.getHost>
		);

		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		expect(vi.mocked(api.getHost)).toHaveBeenCalledWith({ path: { id: 'host-001' } });

		page.params.id = 'host-002';
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Staging Server' })).toBeInTheDocument());
		expect(vi.mocked(api.getHost)).toHaveBeenCalledWith({ path: { id: 'host-002' } });
		expect(screen.getByText('staging-server')).toBeInTheDocument();
	});

	it('does not re-fetch when the same id is re-assigned', async () => {
		vi.mocked(api.getHost).mockResolvedValue({ data: sampleHost } as unknown as Awaited<
			ReturnType<typeof api.getHost>
		>);

		render(HostDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Production Server' })).toBeInTheDocument());
		expect(vi.mocked(api.getHost)).toHaveBeenCalledTimes(1);

		page.params.id = 'host-001';
		await new Promise((resolve) => setTimeout(resolve, 0));
		expect(vi.mocked(api.getHost)).toHaveBeenCalledTimes(1);
	});

	it('discards a stale response that resolves after a newer navigation (out-of-order resolution guard)', async () => {
		let resolveHost1!: (value: Awaited<ReturnType<typeof api.getHost>>) => void;
		let resolveHost2!: (value: Awaited<ReturnType<typeof api.getHost>>) => void;

		vi.mocked(api.getHost).mockImplementation(({ path }: { path: { id: string } }) => {
			if (path.id === 'host-001') {
				return new Promise((resolve) => {
					resolveHost1 = resolve;
				}) as ReturnType<typeof api.getHost>;
			}
			return new Promise((resolve) => {
				resolveHost2 = resolve;
			}) as ReturnType<typeof api.getHost>;
		});

		render(HostDetailPage);
		await waitFor(() => expect(vi.mocked(api.getHost)).toHaveBeenCalledWith({ path: { id: 'host-001' } }));

		// Navigate to host-002 before host-001's fetch resolves.
		page.params.id = 'host-002';
		await waitFor(() => expect(vi.mocked(api.getHost)).toHaveBeenCalledWith({ path: { id: 'host-002' } }));

		// Resolve host-002 first (the current id), then host-001 (the stale, superseded id).
		resolveHost2({ data: sampleHost2 } as unknown as Awaited<ReturnType<typeof api.getHost>>);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Staging Server' })).toBeInTheDocument());

		resolveHost1({ data: sampleHost } as unknown as Awaited<ReturnType<typeof api.getHost>>);

		// Give the stale host-001 response a chance to (incorrectly) commit if the guard were absent.
		await new Promise((resolve) => setTimeout(resolve, 0));

		// Committed state must remain host-002's — the generation guard must have discarded host-001's late response.
		expect(screen.getByRole('heading', { name: 'Staging Server' })).toBeInTheDocument();
		expect(screen.queryByRole('heading', { name: 'Production Server' })).not.toBeInTheDocument();
		expect(screen.getByText('staging-server')).toBeInTheDocument();
	});

	it('discards a stale assigned-software response that resolves after a newer navigation', async () => {
		vi.mocked(api.getHost).mockImplementation(
			({ path }: { path: { id: string } }) =>
				Promise.resolve({
					data: path.id === 'host-002' ? sampleHost2 : sampleHost
				}) as ReturnType<typeof api.getHost>
		);
		vi.mocked(auth.getUser).mockReturnValue({
			...adminUser,
			actions: [...adminUser.actions, Actions.SOFTWARE_READ]
		});

		let resolveSoftware1!: (value: Awaited<ReturnType<typeof api.listSoftwareItems>>) => void;
		let resolveSoftware2!: (value: Awaited<ReturnType<typeof api.listSoftwareItems>>) => void;

		vi.mocked(api.listSoftwareItems).mockImplementation(((opts: Parameters<typeof api.listSoftwareItems>[0]) => {
			if (opts?.query?.host_id === 'host-001') {
				return new Promise((resolve) => {
					resolveSoftware1 = resolve;
				});
			}
			return new Promise((resolve) => {
				resolveSoftware2 = resolve;
			});
		}) as unknown as typeof api.listSoftwareItems);

		render(HostDetailPage);
		await waitFor(() =>
			expect(vi.mocked(api.listSoftwareItems)).toHaveBeenCalledWith({
				query: { page: 1, per_page: 20, host_id: 'host-001' }
			})
		);

		// Navigate to host-002 before host-001's assigned-software fetch resolves.
		page.params.id = 'host-002';
		await waitFor(() =>
			expect(vi.mocked(api.listSoftwareItems)).toHaveBeenCalledWith({
				query: { page: 1, per_page: 20, host_id: 'host-002' }
			})
		);

		// Resolve host-002 first (the current id), then host-001 (the stale, superseded id).
		resolveSoftware2({
			data: {
				items: [{ id: 'sw-002', name: 'host-002-software' }],
				total: 1,
				page: 1,
				per_page: 20,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof api.listSoftwareItems>>);
		await waitFor(() => expect(screen.getByText('host-002-software')).toBeInTheDocument());

		resolveSoftware1({
			data: {
				items: [{ id: 'sw-001', name: 'host-001-software' }],
				total: 1,
				page: 1,
				per_page: 20,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof api.listSoftwareItems>>);

		// Give the stale host-001 response a chance to (incorrectly) commit if the guard were absent.
		await new Promise((resolve) => setTimeout(resolve, 0));

		// Committed assigned-software list must remain host-002's — the guard must have discarded host-001's late response.
		expect(screen.getByText('host-002-software')).toBeInTheDocument();
		expect(screen.queryByText('host-001-software')).not.toBeInTheDocument();
	});
});
