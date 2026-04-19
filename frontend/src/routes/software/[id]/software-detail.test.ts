import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import type {
	SoftwareItemDetailResponse,
	SoftwareItemHostSummary,
	SurfaceReadResponse,
	SurfaceResponse
} from '$lib/types';
import { Permission } from '$lib/types';

vi.mock('$lib/api', () => ({
	getSoftwareItem: vi.fn(),
	getSoftwareItems: vi.fn(),
	checkSoftwareItemVersions: vi.fn(),
	checkSoftwareItemVersionsHost: vi.fn(),
	triggerSoftwareUpdate: vi.fn(),
	updateSoftwareItem: vi.fn(),
	deleteSoftwareItem: vi.fn(),
	unassignHostFromSoftwareItem: vi.fn(),
	getUpdateHistoryEntry: vi.fn(),
	previewSoftwareItemMerge: vi.fn(),
	executeSoftwareItemMerge: vi.fn(),
	invokeSurfaceInteraction: vi.fn(() => Promise.resolve({}))
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null)
}));

vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn()
}));

vi.mock('$lib/stores/events.svelte', () => ({
	subscribeToEvent: vi.fn(() => () => {})
}));

vi.mock('$lib/surfaces/registry.svelte', () => ({
	getSurfaceReadModel: vi.fn(() => undefined),
	getSurfaceRuntimeStatus: vi.fn(() => ({ active: false })),
	getSurfaceProviders: vi.fn(() => []),
	getSurfacesBySlot: vi.fn(() => []),
	loadSurfaceReadModels: vi.fn(() => Promise.resolve())
}));

vi.mock('$lib/interactive', () => ({
	connectInteractiveSession: vi.fn(() => ({
		disconnect: vi.fn(),
		sendInput: vi.fn(),
		sendSignal: vi.fn()
	}))
}));

vi.mock('$lib/components/TerminalOutput.svelte', async () => {
	const mod = await import('$lib/test-mocks/terminal-output-mock.svelte');
	return { default: mod.default };
});

import SoftwareDetailPage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';
import * as surfaceRegistry from '$lib/surfaces/registry.svelte';
import { page } from '$app/state';

const adminUser = {
	id: '00000000-0000-0000-0000-000000000001',
	email: 'admin@example.com',
	first_name: 'Admin',
	last_name: 'User',
	permissions: [
		Permission.ViewSoftware,
		Permission.CreateSoftware,
		Permission.UpdateSoftware,
		Permission.DeleteSoftware,
		Permission.TriggerChecks,
		Permission.TriggerUpdates
	]
};

function makeHost(): SoftwareItemHostSummary {
	return {
		id: 'row-1',
		host_id: 'host-1',
		hostname: 'host-one',
		friendly_name: 'Host One',
		qualifier: null,
		installed_version: '1.0.0',
		installed_version_detected_at: '2024-01-01T00:00:00Z',
		installed_display_version: null,
		latest_version: '1.1.0',
		latest_release_metadata: null,
		update_available: true,
		active_update_history_id: null,
		last_updated_at: null,
		linked_at: '2024-01-01T00:00:00Z',
		plugins: []
	};
}

function makeSoftwareItem(hosts: SoftwareItemHostSummary[]): SoftwareItemDetailResponse {
	return {
		id: 'software-1',
		name: 'Demo App',
		plugins: ['generic_shell'],
		featured: false,
		last_checked_at: null,
		host_count: hosts.length,
		installed_version: null,
		installed_display_version: null,
		latest_version: '1.1.0',
		latest_release_metadata: null,
		update_available: true,
		created_at: '2024-01-01T00:00:00Z',
		updated_at: '2024-01-01T00:00:00Z',
		icon_url: null,
		hosts
	};
}

function makeSurface(surfaceId: string, slot: string, label: string, requiredPermission?: string): SurfaceResponse {
	return {
		surface_id: surfaceId,
		label,
		priority: 100,
		slot,
		scope: 'tenant',
		targeting: 'universal',
		required_permission: requiredPermission,
		provider_kind: 'plugin',
		required_capabilities: [],
		root_node: {
			kind: 'key_value',
			data_source_id: `${surfaceId}.source`
		},
		provider_count: 1
	};
}

function makeRenderableRead(surface: SurfaceResponse, interactionId: string): SurfaceReadResponse {
	const dataSourceId =
		surface.root_node.kind === 'key_value' || surface.root_node.kind === 'table'
			? surface.root_node.data_source_id
			: `${surface.surface_id}.source`;

	return {
		descriptor: {
			...surface
		},
		interactions: [
			{
				interaction_id: interactionId,
				kind: 'data_load',
				label: 'Load Surface Data',
				transport: { mode: 'provider_proxied' }
			}
		],
		data_sources: [
			{
				data_source_id: dataSourceId,
				kind: { kind: 'provider_query', operation_id: interactionId },
				result_schema: 'object',
				refresh_policy: { type: 'manual' }
			}
		]
	};
}

describe('Software Detail shared-surface slots', () => {
	beforeEach(() => {
		page.params.id = 'software-1';
		vi.mocked(auth.getUser).mockReturnValue(adminUser);
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('loads software-item tab surfaces and passes software_item_id to panel reads', async () => {
		const item = makeSoftwareItem([makeHost()]);
		const softwareItemTabSurface = makeSurface(
			'software.item.tab.surface',
			'software_item.tabs',
			'Software Item Diagnostics',
			Permission.ViewSoftware
		);
		const hostContextSurface = makeSurface(
			'software.item.host.context.surface',
			'software_item.host_context_menu',
			'Host Context Action',
			Permission.UpdateSoftware
		);
		const reads = new Map<string, SurfaceReadResponse>([
			[softwareItemTabSurface.surface_id, makeRenderableRead(softwareItemTabSurface, 'load_software_item_tab')],
			[hostContextSurface.surface_id, makeRenderableRead(hostContextSurface, 'load_host_context')]
		]);

		vi.mocked(api.getSoftwareItem).mockResolvedValue(item);
		vi.mocked(surfaceRegistry.getSurfaceRuntimeStatus).mockReturnValue({ active: true });
		vi.mocked(surfaceRegistry.getSurfacesBySlot).mockImplementation((slot: string) => {
			if (slot === 'software_item.tabs') {
				return [softwareItemTabSurface];
			}
			if (slot === 'software_item.host_context_menu') {
				return [hostContextSurface];
			}
			return [];
		});
		vi.mocked(surfaceRegistry.getSurfaceReadModel).mockImplementation((surfaceId: string) => reads.get(surfaceId));

		render(SoftwareDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Demo App' })).toBeInTheDocument());

		expect(screen.getByRole('heading', { name: 'Software Item Diagnostics' })).toBeInTheDocument();
		expect(vi.mocked(surfaceRegistry.loadSurfaceReadModels)).toHaveBeenCalledWith(['software.item.tab.surface']);

		await waitFor(() =>
			expect(vi.mocked(api.invokeSurfaceInteraction)).toHaveBeenCalledWith(
				'software.item.tab.surface',
				'load_software_item_tab',
				{
					params: { software_item_id: 'software-1' },
					target_provider_id: undefined
				}
			)
		);
	});

	it('keeps host-context menu surface behavior active', async () => {
		const item = makeSoftwareItem([makeHost()]);
		const softwareItemTabSurface = makeSurface(
			'software.item.tab.surface',
			'software_item.tabs',
			'Software Item Diagnostics',
			Permission.ViewSoftware
		);
		const hostContextSurface = makeSurface(
			'software.item.host.context.surface',
			'software_item.host_context_menu',
			'Run Host Action',
			Permission.UpdateSoftware
		);
		const reads = new Map<string, SurfaceReadResponse>([
			[softwareItemTabSurface.surface_id, makeRenderableRead(softwareItemTabSurface, 'load_software_item_tab')],
			[hostContextSurface.surface_id, makeRenderableRead(hostContextSurface, 'load_host_context')]
		]);

		vi.mocked(api.getSoftwareItem).mockResolvedValue(item);
		vi.mocked(surfaceRegistry.getSurfaceRuntimeStatus).mockReturnValue({ active: true });
		vi.mocked(surfaceRegistry.getSurfacesBySlot).mockImplementation((slot: string) => {
			if (slot === 'software_item.tabs') {
				return [softwareItemTabSurface];
			}
			if (slot === 'software_item.host_context_menu') {
				return [hostContextSurface];
			}
			return [];
		});
		vi.mocked(surfaceRegistry.getSurfaceReadModel).mockImplementation((surfaceId: string) => reads.get(surfaceId));

		render(SoftwareDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Demo App' })).toBeInTheDocument());

		await fireEvent.click(screen.getByRole('button', { name: 'Actions for host-one' }));
		await waitFor(() => expect(screen.getByRole('menuitem', { name: 'Run Host Action' })).toBeInTheDocument());

		expect(screen.getByRole('menuitem', { name: 'Configure Plugins' })).toHaveAttribute('data-ui', 'context-menu-item');
		expect(screen.getByRole('menuitem', { name: 'Run Host Action' })).toHaveAttribute('data-ui', 'context-menu-item');

		vi.mocked(api.invokeSurfaceInteraction).mockClear();
		await fireEvent.click(screen.getByRole('menuitem', { name: 'Run Host Action' }));

		await waitFor(() => expect(screen.getByRole('heading', { name: /Run Host Action/ })).toBeInTheDocument());
		await waitFor(() =>
			expect(vi.mocked(api.invokeSurfaceInteraction)).toHaveBeenCalledWith(
				'software.item.host.context.surface',
				'load_host_context',
				{
					params: { software_item_id: 'software-1', host_id: 'host-1' },
					target_provider_id: undefined
				}
			)
		);
	});
});
