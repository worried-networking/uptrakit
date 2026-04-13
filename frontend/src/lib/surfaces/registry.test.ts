import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type {
	SurfaceProviderInfo,
	SurfaceReadResponse,
	SurfaceResponse,
	SurfaceRuntimeStatusResponse
} from '$lib/surfaces/contract';
import {
	clearSurfaceRegistry,
	getSurfaceReadLoading,
	getSurfaceReadRequested,
	getSurfaceProviders,
	getSurfaceReadModel,
	getSurfacesBySlot,
	getSurfaceRuntimeStatus,
	loadSurfaceReadModels,
	loadSurfaceRegistry,
	resolveExtensionPageNavItems
} from './registry.svelte';
import {
	getSurfaceRead,
	getSurfaceRuntimeStatus as fetchSurfaceRuntimeStatus,
	listSurfaceProviders,
	listSurfaces
} from '$lib/api';

vi.mock('$lib/api', () => ({
	listSurfaces: vi.fn(),
	listSurfaceProviders: vi.fn(),
	getSurfaceRuntimeStatus: vi.fn(),
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
			slot: 'extension.page',
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
		vi.mocked(fetchSurfaceRuntimeStatus).mockResolvedValue({
			active: false
		} satisfies SurfaceRuntimeStatusResponse);
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
				slot: 'extension.page',
				targeting: 'universal'
			}),
			makeSurface({
				surfaceId: 'surface.alpha',
				label: 'Alpha',
				priority: 200,
				slot: 'extension.page',
				targeting: 'universal'
			}),
			makeSurface({
				surfaceId: 'surface.early',
				label: 'Early',
				priority: 100,
				slot: 'extension.page',
				targeting: 'universal'
			})
		]);

		await loadSurfaceRegistry();

		expect(getSurfacesBySlot('extension.page').map((surface) => surface.surface_id)).toEqual([
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
				slot: 'extension.page',
				targeting: 'targeted'
			}),
			makeSurface({
				surfaceId: 'surface.universal',
				label: 'Universal',
				priority: 200,
				slot: 'extension.page',
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

	it('reads rollout status from the controller-owned status endpoint', async () => {
		vi.mocked(listSurfaces).mockResolvedValue([]);
		vi.mocked(fetchSurfaceRuntimeStatus).mockResolvedValue({
			active: true
		} satisfies SurfaceRuntimeStatusResponse);

		await loadSurfaceRegistry();

		expect(getSurfaceRuntimeStatus().active).toBe(true);
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

	it('keeps legacy extension ids and only overlays compatible surface nav metadata', () => {
		const legacyNav = [
			{ id: 'legacy.settings', href: '/extensions/legacy.settings', label: 'Legacy Settings', priority: 600 },
			{ id: 'legacy.software', href: '/extensions/legacy.software', label: 'Legacy Software', priority: 700 }
		];
		const slotSurfaces = [
			makeSurface({
				surfaceId: 'legacy.settings',
				label: 'Surface Settings',
				priority: 100,
				slot: 'extension.page',
				targeting: 'universal'
			}),
			makeSurface({
				surfaceId: 'surface.only',
				label: 'Surface Only',
				priority: 50,
				slot: 'extension.page',
				targeting: 'universal'
			})
		];

		const merged = resolveExtensionPageNavItems(legacyNav, slotSurfaces, true);

		expect(merged).toEqual([
			{
				id: 'legacy.settings',
				href: '/extensions/legacy.settings',
				label: 'Surface Settings',
				priority: 100
			},
			{
				id: 'legacy.software',
				href: '/extensions/legacy.software',
				label: 'Legacy Software',
				priority: 700
			}
		]);
	});
});
