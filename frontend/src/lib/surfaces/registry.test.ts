import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { tick } from 'svelte';
import type { SurfaceProviderInfo, SurfaceReadResponse, SurfaceResponse } from '$lib/surfaces/contract';
import {
	clearSurfaceRegistry,
	getSurfaceReadFailed,
	getSurfaceReadLoading,
	getSurfaceReadRequested,
	getSurfaceProviders,
	getSurfaceReadModel,
	getSurfacesBySlot,
	loadSurfaceReadModel,
	loadSurfaceReadModels,
	loadSurfaceRegistry,
	refreshSurfaceReadModel,
	resolveSurfacePageNavItems
} from './registry.svelte';
import { getSurfaceRead, listSurfaceProviders, listSurfaces } from '$lib/api';

vi.mock('$lib/api', async (importOriginal) => ({
	...(await importOriginal<typeof import('$lib/api')>()),
	listSurfaces: vi.fn(),
	listSurfaceProviders: vi.fn(),
	getSurfaceRead: vi.fn()
}));

function makeSurface({
	surfaceId,
	label,
	priority,
	slot,
	targeting,
	nav_icon
}: {
	surfaceId: string;
	label: string;
	priority: number;
	slot: string;
	targeting: 'universal' | 'targeted';
	nav_icon?: string;
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
		provider_count: targeting === 'targeted' ? 2 : 1,
		nav_icon
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

	it('marks a surface read as failed and does not refetch on a second load call (loop prevention)', async () => {
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
		vi.mocked(getSurfaceRead).mockRejectedValue(new Error('boom'));

		await loadSurfaceRegistry();
		await loadSurfaceReadModel('surface.targeted');

		expect(getSurfaceReadFailed('surface.targeted')).toBe(true);
		expect(getSurfaceReadModel('surface.targeted')).toBeUndefined();
		expect(getSurfaceRead).toHaveBeenCalledTimes(1);

		await loadSurfaceReadModel('surface.targeted');
		await tick();

		expect(getSurfaceRead).toHaveBeenCalledTimes(1);
		expect(getSurfaceReadFailed('surface.targeted')).toBe(true);
	});

	it('refreshSurfaceReadModel clears the failed mark and re-fetches successfully', async () => {
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
		vi.mocked(getSurfaceRead)
			.mockRejectedValueOnce(new Error('boom'))
			.mockImplementation(async (surfaceId: string) => makeRead(surfaceId));

		await loadSurfaceRegistry();
		await loadSurfaceReadModel('surface.targeted');
		expect(getSurfaceReadFailed('surface.targeted')).toBe(true);

		await refreshSurfaceReadModel('surface.targeted');

		expect(getSurfaceRead).toHaveBeenCalledTimes(2);
		expect(getSurfaceReadFailed('surface.targeted')).toBe(false);
		expect(getSurfaceReadModel('surface.targeted')?.descriptor.surface_id).toBe('surface.targeted');
	});

	it('clears a pre-existing failed mark on a subsequent successful load', async () => {
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
		vi.mocked(getSurfaceRead)
			.mockRejectedValueOnce(new Error('boom'))
			.mockImplementation(async (surfaceId: string) => makeRead(surfaceId));

		await loadSurfaceRegistry();
		await loadSurfaceReadModel('surface.targeted');
		expect(getSurfaceReadFailed('surface.targeted')).toBe(true);

		// A direct retry through refreshSurfaceReadModel (clear-then-load) succeeds and clears the mark.
		await refreshSurfaceReadModel('surface.targeted');

		expect(getSurfaceReadFailed('surface.targeted')).toBe(false);
		expect(getSurfaceReadModel('surface.targeted')?.descriptor.surface_id).toBe('surface.targeted');
	});

	it('clearSurfaceRegistry evicts failed marks, making a previously-failed id re-fetchable', async () => {
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
		vi.mocked(getSurfaceRead).mockRejectedValue(new Error('boom'));

		await loadSurfaceRegistry();
		await loadSurfaceReadModel('surface.targeted');
		expect(getSurfaceReadFailed('surface.targeted')).toBe(true);

		clearSurfaceRegistry();

		expect(getSurfaceReadFailed('surface.targeted')).toBe(false);

		vi.mocked(getSurfaceRead).mockImplementation(async (surfaceId: string) => makeRead(surfaceId));
		await loadSurfaceReadModel('surface.targeted');

		expect(getSurfaceReadModel('surface.targeted')?.descriptor.surface_id).toBe('surface.targeted');
	});

	it('loadSurfaceRegistry evicts failed marks, making a previously-failed id re-fetchable', async () => {
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
		vi.mocked(getSurfaceRead).mockRejectedValue(new Error('boom'));

		await loadSurfaceRegistry();
		await loadSurfaceReadModel('surface.targeted');
		expect(getSurfaceReadFailed('surface.targeted')).toBe(true);

		// A registry re-fetch (e.g. tenant switch) should evict failedReads too.
		await loadSurfaceRegistry();

		expect(getSurfaceReadFailed('surface.targeted')).toBe(false);

		vi.mocked(getSurfaceRead).mockImplementation(async (surfaceId: string) => makeRead(surfaceId));
		await loadSurfaceReadModel('surface.targeted');

		expect(getSurfaceReadModel('surface.targeted')?.descriptor.surface_id).toBe('surface.targeted');
	});

	it('does not re-fire loadSurfaceReadModel on repeated per-navigation refresh-then-fail churn (loop freedom)', async () => {
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
		vi.mocked(getSurfaceRead).mockRejectedValue(new Error('boom'));

		await loadSurfaceRegistry();

		// Simulate a keyed-effect navigation: refresh (clears mark) then load (fails again).
		await refreshSurfaceReadModel('surface.targeted');
		expect(getSurfaceRead).toHaveBeenCalledTimes(1);
		expect(getSurfaceReadFailed('surface.targeted')).toBe(true);

		// Reactive settle after the failure — the churn on failedReads (delete-then-add) must not
		// itself trigger another loadSurfaceReadModel call outside the one deliberate refresh above.
		await tick();
		await tick();

		expect(getSurfaceRead).toHaveBeenCalledTimes(1);

		// A second, independent navigation performs exactly one more deliberate retry.
		await refreshSurfaceReadModel('surface.targeted');
		expect(getSurfaceRead).toHaveBeenCalledTimes(2);
		expect(getSurfaceReadFailed('surface.targeted')).toBe(true);

		// And again settling must not climb further.
		await tick();
		await tick();
		expect(getSurfaceRead).toHaveBeenCalledTimes(2);
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
				priority: 50,
				icon: 'box'
			},
			{
				id: 'surface.settings',
				href: '/surfaces/surface.settings',
				label: 'Surface Settings',
				priority: 100,
				icon: 'box'
			}
		]);
	});

	it('passes nav_icon through as icon when set', () => {
		const slotSurfaces = [
			makeSurface({
				surfaceId: 'surface.plugin',
				label: 'Plugin',
				priority: 100,
				slot: 'surface.page',
				targeting: 'universal',
				nav_icon: 'package'
			})
		];
		expect(resolveSurfacePageNavItems(slotSurfaces)[0].icon).toBe('package');
	});

	it("falls back to 'box' when nav_icon is absent", () => {
		const slotSurfaces = [
			makeSurface({
				surfaceId: 'surface.plugin',
				label: 'Plugin',
				priority: 100,
				slot: 'surface.page',
				targeting: 'universal'
			})
		];
		expect(resolveSurfacePageNavItems(slotSurfaces)[0].icon).toBe('box');
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
