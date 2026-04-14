import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import SurfaceReadPanel from './SurfaceReadPanel.svelte';
import type { SurfaceReadResponse, SurfaceResponse } from '$lib/surfaces/contract';
import { invokeSurfaceInteraction } from '$lib/api';

vi.mock('$lib/surfaces/registry.svelte', () => ({
	getSurfaceProviders: vi.fn(() => [])
}));

vi.mock('$lib/api', () => ({
	invokeSurfaceInteraction: vi.fn()
}));

function makeSurface(): SurfaceResponse {
	return {
		surface_id: 'surface.one',
		label: 'Surface One',
		priority: 100,
		slot: 'extension.page',
		scope: 'tenant',
		targeting: 'universal',
		provider_kind: 'service',
		required_capabilities: [],
		root_node: { kind: 'text_block', text: 'list descriptor node' },
		provider_count: 1
	};
}

function makeRead(surfaceId = 'surface.one'): SurfaceReadResponse {
	return {
		descriptor: {
			surface_id: surfaceId,
			label: 'Read Descriptor',
			priority: 100,
			slot: 'extension.page',
			scope: 'tenant',
			targeting: 'universal',
			provider_kind: 'service',
			required_capabilities: [],
			root_node: { kind: 'text_block', text: 'read descriptor node' }
		},
		interactions: [],
		data_sources: []
	};
}

describe('SurfaceReadPanel', () => {
	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
	});

	it('renders from read.descriptor instead of the list descriptor when read is present', () => {
		render(SurfaceReadPanel, {
			surface: makeSurface(),
			read: makeRead()
		});

		expect(screen.getByText('read descriptor node')).toBeInTheDocument();
		expect(screen.queryByText('list descriptor node')).not.toBeInTheDocument();
	});

	it('rejects mismatched descriptors instead of mixing list and read metadata', () => {
		render(SurfaceReadPanel, {
			surface: makeSurface(),
			read: makeRead('surface.two')
		});

		expect(screen.getByText('Surface contract mismatch detected. Please refresh and try again.')).toBeInTheDocument();
		expect(screen.queryByText('read descriptor node')).not.toBeInTheDocument();
	});

	it('hydrates key-value provider-query data via the surface interaction endpoint', async () => {
		vi.mocked(invokeSurfaceInteraction).mockResolvedValue({
			region: 'eu-west-1',
			node: 'pve-01'
		});
		const read: SurfaceReadResponse = {
			descriptor: {
				surface_id: 'surface.one',
				label: 'Read Descriptor',
				priority: 100,
				slot: 'host_detail.tabs',
				scope: 'tenant',
				targeting: 'universal',
				provider_kind: 'plugin',
				required_capabilities: [],
				root_node: {
					kind: 'key_value',
					data_source_id: 'data.remote'
				}
			},
			interactions: [
				{
					interaction_id: 'get-info',
					kind: 'data_load',
					transport: { mode: 'controller_local' }
				}
			],
			data_sources: [
				{
					data_source_id: 'data.remote',
					kind: { kind: 'provider_query', operation_id: 'get-info' },
					result_schema: 'object',
					refresh_policy: { type: 'manual' }
				}
			]
		};

		render(SurfaceReadPanel, {
			surface: makeSurface(),
			read,
			baseParams: { host_id: 'host-001' }
		});

		await screen.findByText('region');
		expect(screen.getByText('eu-west-1')).toBeInTheDocument();
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledWith(
			'surface.one',
			'get-info',
			expect.objectContaining({
				params: {
					host_id: 'host-001'
				}
			})
		);
	});

	it('does not rehydrate key-value provider-query data on rerender when base params are semantically unchanged', async () => {
		vi.mocked(invokeSurfaceInteraction).mockResolvedValue({
			region: 'eu-west-1'
		});
		const read: SurfaceReadResponse = {
			descriptor: {
				surface_id: 'surface.one',
				label: 'Read Descriptor',
				priority: 100,
				slot: 'host_detail.tabs',
				scope: 'tenant',
				targeting: 'universal',
				provider_kind: 'plugin',
				required_capabilities: [],
				root_node: {
					kind: 'key_value',
					data_source_id: 'data.remote'
				}
			},
			interactions: [
				{
					interaction_id: 'get-info',
					kind: 'data_load',
					transport: { mode: 'controller_local' }
				}
			],
			data_sources: [
				{
					data_source_id: 'data.remote',
					kind: { kind: 'provider_query', operation_id: 'get-info' },
					result_schema: 'object',
					refresh_policy: { type: 'manual' }
				}
			]
		};

		const view = render(SurfaceReadPanel, {
			surface: makeSurface(),
			read,
			baseParams: { host_id: 'host-001' }
		});
		await screen.findByText('region');
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(1);

		await view.rerender({
			surface: makeSurface(),
			read,
			baseParams: { host_id: 'host-001' }
		});
		await screen.findByText('region');

		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(1);
	});

	it('shows an explicit error state when provider-query hydration fails', async () => {
		vi.mocked(invokeSurfaceInteraction).mockRejectedValue(new Error('boom'));
		const read: SurfaceReadResponse = {
			descriptor: {
				surface_id: 'surface.one',
				label: 'Read Descriptor',
				priority: 100,
				slot: 'host_detail.tabs',
				scope: 'tenant',
				targeting: 'universal',
				provider_kind: 'plugin',
				required_capabilities: [],
				root_node: {
					kind: 'key_value',
					data_source_id: 'data.remote'
				}
			},
			interactions: [
				{
					interaction_id: 'get-info',
					kind: 'data_load',
					transport: { mode: 'controller_local' }
				}
			],
			data_sources: [
				{
					data_source_id: 'data.remote',
					kind: { kind: 'provider_query', operation_id: 'get-info' },
					result_schema: 'object',
					refresh_policy: { type: 'manual' }
				}
			]
		};

		render(SurfaceReadPanel, {
			surface: makeSurface(),
			read,
			baseParams: { host_id: 'host-001' }
		});

		expect(await screen.findByText('Failed to load surface data. Please try again.')).toBeInTheDocument();
		expect(screen.queryByText('No data available.')).not.toBeInTheDocument();
	});
});
