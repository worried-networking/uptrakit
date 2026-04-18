import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import SurfaceReadPanel from './SurfaceReadPanel.svelte';
import type { SurfaceReadResponse, SurfaceResponse } from '$lib/surfaces/contract';
import { invokeSurfaceInteraction } from '$lib/api';
import { getSurfaceProviders } from '$lib/surfaces/registry.svelte';

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
		slot: 'surface.page',
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
			slot: 'surface.page',
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
	beforeEach(() => {
		vi.resetAllMocks();
		vi.mocked(getSurfaceProviders).mockReturnValue([]);
	});

	afterEach(() => {
		cleanup();
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

	it('maps missing read payload to the canonical contract_mismatch state', () => {
		render(SurfaceReadPanel, {
			surface: makeSurface(),
			read: undefined
		});

		expect(screen.getByText('Surface contract mismatch detected. Please refresh and try again.')).toBeInTheDocument();
		expect(screen.queryByText('Surface contract is not available yet.')).not.toBeInTheDocument();
	});

	it('maps unrenderable read payload to the canonical contract_mismatch state', () => {
		const read: SurfaceReadResponse = {
			descriptor: {
				surface_id: 'surface.one',
				label: 'Read Descriptor',
				priority: 100,
				slot: 'surface.page',
				scope: 'tenant',
				targeting: 'universal',
				provider_kind: 'plugin',
				required_capabilities: [],
				root_node: {
					kind: 'key_value',
					data_source_id: 'data.unsupported'
				}
			},
			interactions: [],
			data_sources: [
				{
					data_source_id: 'data.unsupported',
					kind: { kind: 'controller_query', query_id: 'query.unsupported' },
					result_schema: 'object',
					refresh_policy: { type: 'manual' }
				}
			]
		};

		render(SurfaceReadPanel, {
			surface: makeSurface(),
			read
		});

		expect(screen.getByText('Surface contract mismatch detected. Please refresh and try again.')).toBeInTheDocument();
		expect(
			screen.queryByText('This surface uses unsupported data sources and cannot be rendered yet.')
		).not.toBeInTheDocument();
	});

	it('renders the shared provider selector for targeted surfaces', () => {
		vi.mocked(getSurfaceProviders).mockReturnValue([
			{
				provider_id: 'provider.a',
				display_label: 'Provider A',
				service_id: 'service-a',
				availability: 'available'
			}
		]);
		const surface: SurfaceResponse = {
			...makeSurface(),
			targeting: 'targeted',
			provider_kind: 'plugin'
		};
		const read: SurfaceReadResponse = {
			...makeRead(),
			descriptor: {
				...makeRead().descriptor,
				targeting: 'targeted',
				provider_kind: 'plugin'
			}
		};

		const { container } = render(SurfaceReadPanel, {
			surface,
			read
		});

		expect(screen.getByLabelText('Provider')).toBeInTheDocument();
		expect(screen.getByText('Service service-a')).toBeInTheDocument();
		expect(container.querySelector('[data-ui="provider-selector"]')).toBeInTheDocument();
		expect(container.querySelector('[data-ui="provider-selector"]')?.parentElement?.className).toContain(
			'max-w-[280px]'
		);
	});

	it('uses canonical no-provider copy when targeted surfaces only have unavailable providers', () => {
		vi.mocked(getSurfaceProviders).mockReturnValue([
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
		const surface: SurfaceResponse = {
			...makeSurface(),
			targeting: 'targeted',
			provider_kind: 'plugin'
		};
		const read: SurfaceReadResponse = {
			descriptor: {
				surface_id: 'surface.one',
				label: 'Read Descriptor',
				priority: 100,
				slot: 'surface.page',
				scope: 'tenant',
				targeting: 'targeted',
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
			surface,
			read,
			baseParams: { host_id: 'host-001' }
		});

		expect(screen.getByText('No provider connected')).toBeInTheDocument();
		expect(screen.getByText('Connect a compatible service to use this surface.')).toBeInTheDocument();
		expect(vi.mocked(invokeSurfaceInteraction)).not.toHaveBeenCalled();
	});

	it('keeps exactly one provider selector visible above loading for targeted surface.page hydration', async () => {
		vi.mocked(getSurfaceProviders).mockReturnValue([
			{
				provider_id: 'provider.a',
				display_label: 'Provider A',
				service_id: 'service-a',
				availability: 'available'
			}
		]);
		vi.mocked(invokeSurfaceInteraction).mockImplementation(
			() =>
				new Promise(() => {
					// Keep hydration in-flight to assert loading-state layout.
				})
		);
		const surface: SurfaceResponse = {
			...makeSurface(),
			targeting: 'targeted',
			provider_kind: 'plugin'
		};
		const read: SurfaceReadResponse = {
			descriptor: {
				surface_id: 'surface.one',
				label: 'Read Descriptor',
				priority: 100,
				slot: 'surface.page',
				scope: 'tenant',
				targeting: 'targeted',
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

		const { container } = render(SurfaceReadPanel, {
			surface,
			read,
			baseParams: { host_id: 'host-001' },
			reloadToken: 0
		});

		const loading = await screen.findByText('Loading...');
		const selectors = container.querySelectorAll('[data-ui="provider-selector"]');
		expect(selectors).toHaveLength(1);
		expect(screen.getByLabelText('Provider')).toBeInTheDocument();
		expect((selectors[0].compareDocumentPosition(loading) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0).toBe(true);
	});

	it('keeps exactly one provider selector visible above hydration failure for targeted surface.page', async () => {
		vi.mocked(getSurfaceProviders).mockReturnValue([
			{
				provider_id: 'provider.a',
				display_label: 'Provider A',
				service_id: 'service-a',
				availability: 'available'
			}
		]);
		vi.mocked(invokeSurfaceInteraction).mockRejectedValue(new Error('boom'));
		const surface: SurfaceResponse = {
			...makeSurface(),
			targeting: 'targeted',
			provider_kind: 'plugin'
		};
		const read: SurfaceReadResponse = {
			descriptor: {
				surface_id: 'surface.one',
				label: 'Read Descriptor',
				priority: 100,
				slot: 'surface.page',
				scope: 'tenant',
				targeting: 'targeted',
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

		const { container } = render(SurfaceReadPanel, {
			surface,
			read,
			baseParams: { host_id: 'host-001' },
			reloadToken: 0
		});

		const errorTitle = await screen.findByText('Unable to load surface data');
		const selectors = container.querySelectorAll('[data-ui="provider-selector"]');
		expect(selectors).toHaveLength(1);
		expect(screen.getByLabelText('Provider')).toBeInTheDocument();
		expect(screen.getByText('Failed to load surface data. Please try again.')).toBeInTheDocument();
		expect((selectors[0].compareDocumentPosition(errorTitle) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0).toBe(true);
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
			baseParams: { host_id: 'host-001' },
			reloadToken: 0
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
			baseParams: { host_id: 'host-001' },
			reloadToken: 0
		});
		await screen.findByText('region');
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(1);

		await view.rerender({
			surface: makeSurface(),
			read,
			baseParams: { host_id: 'host-001' },
			reloadToken: 0
		});
		await screen.findByText('region');

		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(1);
	});

	it('keeps base params stable when undefined keys are present', async () => {
		vi.mocked(invokeSurfaceInteraction).mockResolvedValueOnce({
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
			baseParams: { host_id: 'host-001', ignored: undefined },
			reloadToken: 0
		});
		await screen.findByText('region');
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledWith(
			'surface.one',
			'get-info',
			expect.objectContaining({
				params: {
					host_id: 'host-001'
				}
			})
		);
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(1);

		await view.rerender({
			surface: makeSurface(),
			read,
			baseParams: { host_id: 'host-001' },
			reloadToken: 0
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
			baseParams: { host_id: 'host-001' },
			reloadToken: 0
		});

		expect(await screen.findByText('Failed to load surface data. Please try again.')).toBeInTheDocument();
		expect(screen.getByText('Unable to load surface data')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Try again' })).toBeInTheDocument();
		expect(screen.queryByText('No data available.')).not.toBeInTheDocument();
	});

	it('retries hydration from the in-UI retry action after failure', async () => {
		vi.mocked(invokeSurfaceInteraction).mockRejectedValueOnce(new Error('boom')).mockResolvedValueOnce({
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

		render(SurfaceReadPanel, {
			surface: makeSurface(),
			read,
			baseParams: { host_id: 'host-001' },
			reloadToken: 0
		});

		expect(await screen.findByText('Failed to load surface data. Please try again.')).toBeInTheDocument();
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(1);

		await fireEvent.click(screen.getByRole('button', { name: 'Try again' }));

		expect(await screen.findByText('region')).toBeInTheDocument();
		expect(screen.getByText('eu-west-1')).toBeInTheDocument();
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(2);
	});

	it('retries hydration on same-key rerender after failure', async () => {
		vi.mocked(invokeSurfaceInteraction)
			.mockRejectedValueOnce(new Error('boom'))
			.mockResolvedValueOnce({ region: 'eu-west-1' })
			.mockResolvedValueOnce({ region: 'eu-west-2' });
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
			baseParams: { host_id: 'host-001' },
			reloadToken: 0
		});
		expect(await screen.findByText('Failed to load surface data. Please try again.')).toBeInTheDocument();
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(1);

		await view.rerender({
			surface: makeSurface(),
			read,
			baseParams: { host_id: 'host-001' },
			reloadToken: 0
		});
		await screen.findByText('region');

		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(2);
		expect(screen.getByText('eu-west-1')).toBeInTheDocument();

		await view.rerender({
			surface: makeSurface(),
			read,
			baseParams: { host_id: 'host-001' },
			reloadToken: 0
		});
		await screen.findByText('region');
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(2);

		await view.rerender({
			surface: makeSurface(),
			read,
			baseParams: { host_id: 'host-001' },
			reloadToken: 1
		});
		await screen.findByText('eu-west-2');
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(3);
	});

	it('keeps in-flight hydration active across same-key rerender and applies the result', async () => {
		let resolveHydration: ((value: unknown) => void) | undefined;
		vi.mocked(invokeSurfaceInteraction).mockImplementation(
			() =>
				new Promise((resolve) => {
					resolveHydration = resolve;
				})
		);
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
			baseParams: { host_id: 'host-001' },
			reloadToken: 0
		});
		expect(await screen.findByText('Loading...')).toBeInTheDocument();

		const semanticallySameRead: SurfaceReadResponse = {
			descriptor: { ...read.descriptor, root_node: { ...read.descriptor.root_node } },
			interactions: read.interactions.map((interaction) => ({ ...interaction })),
			data_sources: read.data_sources.map((source) => ({ ...source, kind: { ...source.kind } }))
		};
		await view.rerender({
			surface: makeSurface(),
			read: semanticallySameRead,
			baseParams: { host_id: 'host-001' },
			reloadToken: 0
		});

		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(1);
		if (resolveHydration) {
			resolveHydration({ region: 'eu-west-1' });
		}

		expect(await screen.findByText('region')).toBeInTheDocument();
		expect(screen.getByText('eu-west-1')).toBeInTheDocument();
	});

	it('restores cached successful hydration when returning to a completed fingerprint', async () => {
		vi.mocked(invokeSurfaceInteraction)
			.mockResolvedValueOnce({ region: 'host-001' })
			.mockRejectedValueOnce(new Error('boom'));
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
			baseParams: { host_id: 'host-001' },
			reloadToken: 0
		});
		await screen.findByText('host-001');
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(1);

		await view.rerender({
			surface: makeSurface(),
			read,
			baseParams: { host_id: 'host-002' },
			reloadToken: 0
		});
		expect(await screen.findByText('Failed to load surface data. Please try again.')).toBeInTheDocument();
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(2);

		await view.rerender({
			surface: makeSurface(),
			read,
			baseParams: { host_id: 'host-001' },
			reloadToken: 0
		});
		expect(await screen.findByText('host-001')).toBeInTheDocument();
		expect(screen.queryByText('Failed to load surface data. Please try again.')).not.toBeInTheDocument();
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(2);
	});
});
