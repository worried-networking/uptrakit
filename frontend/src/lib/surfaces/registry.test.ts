import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SurfaceProviderInfo, SurfaceReadResponse, SurfaceResponse } from '$lib/surfaces/contract';
import {
	clearSurfaceRegistry,
	getSurfaceReadLoading,
	getSurfaceReadRequested,
	getSurfaceProviders,
	getSurfaceReadModel,
	getSurfacesBySlot,
	loadSurfaceReadModels,
	loadSurfaceRegistry,
	resolveSurfacePageNavItems
} from './registry.svelte';
import { getSurfaceRead, listSurfaceProviders, listSurfaces } from '$lib/api';

vi.mock('$lib/api', () => ({
	listSurfaces: vi.fn(),
	listSurfaceProviders: vi.fn(),
	getSurfaceRead: vi.fn()
}));

function makeSurface({
	surfaceId,
	label,
	priority,
	slot,
	targeting
}: {
	surfaceId: string;
	label: string;
	priority: number;
	slot: string;
	targeting: 'universal' | 'targeted';
}): SurfaceResponse {
	return {
		surface_id: surfaceId,
		label,
		priority,
		slot,
		scope: 'tenant',
		targeting,
		provider_kind: 'service',
		required_capabilities: [],
		root_node: {
			kind: 'text_block',
			text: label
		},
		provider_count: targeting === 'targeted' ? 2 : 1
	};
}

function makeProvider(providerId: string): SurfaceProviderInfo {
	return {
		provider_id: providerId,
		display_label: providerId,
		availability: 'available'
	};
}

function makeRead(surfaceId: string): SurfaceReadResponse {
	return {
		descriptor: {
			surface_id: surfaceId,
			label: `Read ${surfaceId}`,
			priority: 100,
			slot: 'surface.page',
			scope: 'tenant',
			targeting: 'universal',
			provider_kind: 'service',
			required_capabilities: [],
			root_node: {
				kind: 'section',
				children: [{ kind: 'text_block', text: 'read' }]
			}
		},
		interactions: [],
		data_sources: []
	};
}

describe('surface registry', () => {
	beforeEach(() => {
		clearSurfaceRegistry();
	});

	afterEach(() => {
		vi.clearAllMocks();
		clearSurfaceRegistry();
	});

	it('indexes surfaces by slot and orders deterministically by priority, label, and id', async () => {
		vi.mocked(listSurfaces).mockResolvedValue([
			makeSurface({
				surfaceId: 'surface.zed',
				label: 'Zed',
				priority: 200,
				slot: 'surface.page',
				targeting: 'universal'
			}),
			makeSurface({
				surfaceId: 'surface.alpha',
				label: 'Alpha',
				priority: 200,
				slot: 'surface.page',
				targeting: 'universal'
			}),
			makeSurface({
				surfaceId: 'surface.early',
				label: 'Early',
				priority: 100,
				slot: 'surface.page',
				targeting: 'universal'
			})
		]);

		await loadSurfaceRegistry();

		expect(getSurfacesBySlot('surface.page').map((surface) => surface.surface_id)).toEqual([
			'surface.early',
			'surface.alpha',
			'surface.zed'
		]);
	});

	it('indexes targeted providers per surface', async () => {
		vi.mocked(listSurfaces).mockResolvedValue([
			makeSurface({
				surfaceId: 'surface.targeted',
				label: 'Targeted',
				priority: 100,
				slot: 'surface.page',
				targeting: 'targeted'
			}),
			makeSurface({
				surfaceId: 'surface.universal',
				label: 'Universal',
				priority: 200,
				slot: 'surface.page',
				targeting: 'universal'
			})
		]);
		vi.mocked(listSurfaceProviders).mockImplementation(async (surfaceId) => {
			if (surfaceId === 'surface.targeted') {
				return [makeProvider('provider.b'), makeProvider('provider.a')];
			}
			return [];
		});

		await loadSurfaceRegistry();

		expect(listSurfaceProviders).toHaveBeenCalledTimes(1);
		expect(listSurfaceProviders).toHaveBeenCalledWith('surface.targeted');
		expect(getSurfaceProviders('surface.targeted').map((provider) => provider.provider_id)).toEqual([
			'provider.a',
			'provider.b'
		]);
		expect(getSurfaceProviders('surface.universal')).toEqual([]);
	});

	it('loads and caches surface read payloads for requested surfaces', async () => {
		vi.mocked(listSurfaces).mockResolvedValue([
			makeSurface({
				surfaceId: 'surface.targeted',
				label: 'Targeted',
				priority: 100,
				slot: 'software.tabs',
				targeting: 'targeted'
			})
		]);
		vi.mocked(listSurfaceProviders).mockResolvedValue([makeProvider('provider.a')]);
		vi.mocked(getSurfaceRead).mockImplementation(async (surfaceId: string) => makeRead(surfaceId));

		await loadSurfaceRegistry();
		await loadSurfaceReadModels(['surface.targeted', 'surface.targeted']);

		expect(getSurfaceRead).toHaveBeenCalledTimes(1);
		expect(getSurfaceRead).toHaveBeenCalledWith('surface.targeted');
		expect(getSurfaceReadModel('surface.targeted')?.descriptor.surface_id).toBe('surface.targeted');
		expect(getSurfaceReadRequested('surface.targeted')).toBe(true);
		expect(getSurfaceReadLoading('surface.targeted')).toBe(false);
	});

	it('does not refetch a surface read payload once it is already cached', async () => {
		vi.mocked(listSurfaces).mockResolvedValue([
			makeSurface({
				surfaceId: 'surface.targeted',
				label: 'Targeted',
				priority: 100,
				slot: 'software.tabs',
				targeting: 'targeted'
			})
		]);
		vi.mocked(listSurfaceProviders).mockResolvedValue([makeProvider('provider.a')]);
		vi.mocked(getSurfaceRead).mockImplementation(async (surfaceId: string) => makeRead(surfaceId));

		await loadSurfaceRegistry();
		await loadSurfaceReadModels(['surface.targeted']);
		await loadSurfaceReadModels(['surface.targeted']);

		expect(getSurfaceRead).toHaveBeenCalledTimes(1);
		expect(getSurfaceReadModel('surface.targeted')?.descriptor.surface_id).toBe('surface.targeted');
	});

	it('derives surface page nav items directly from the surface registry slot', () => {
		const slotSurfaces = [
			makeSurface({
				surfaceId: 'surface.settings',
				label: 'Surface Settings',
				priority: 100,
				slot: 'surface.page',
				targeting: 'universal'
			}),
			makeSurface({
				surfaceId: 'surface.only',
				label: 'Surface Only',
				priority: 50,
				slot: 'surface.page',
				targeting: 'universal'
			})
		];

		expect(resolveSurfacePageNavItems(slotSurfaces)).toEqual([
			{
				id: 'surface.only',
				href: '/surfaces/surface.only',
				label: 'Surface Only',
				priority: 50
			},
			{
				id: 'surface.settings',
				href: '/surfaces/surface.settings',
				label: 'Surface Settings',
				priority: 100
			}
		]);
	});

	it('keeps canonical ordering stable when duplicate surface ids are present', () => {
		const slotSurfaces = [
			makeSurface({
				surfaceId: 'surface.b',
				label: 'Surface B',
				priority: 200,
				slot: 'surface.page',
				targeting: 'universal'
			}),
			makeSurface({
				surfaceId: 'surface.a',
				label: 'Surface A',
				priority: 100,
				slot: 'surface.page',
				targeting: 'universal'
			}),
			makeSurface({
				surfaceId: 'surface.a',
				label: 'Surface A Duplicate',
				priority: 300,
				slot: 'surface.page',
				targeting: 'universal'
			})
		];

		expect(resolveSurfacePageNavItems(slotSurfaces).map((item) => item.id)).toEqual(['surface.a', 'surface.b']);
	});
});
