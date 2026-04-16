import { SvelteMap, SvelteSet } from 'svelte/reactivity';
import {
	getSurfaceRead,
	getSurfaceRuntimeStatus as fetchSurfaceRuntimeStatus,
	listSurfaceProviders,
	listSurfaces
} from '$lib/api';
import type {
	SurfaceProviderInfo,
	SurfaceReadResponse,
	SurfaceResponse,
	SurfaceRuntimeStatusResponse
} from './contract';

let surfaces: SurfaceResponse[] = $state([]);
let surfacesLoaded = $state(false);
let surfacesLoading = $state(false);
let runtimeStatus: SurfaceRuntimeStatusResponse = $state({ active: false });

const surfacesBySlot = new SvelteMap<string, SurfaceResponse[]>();
const providersBySurface = new SvelteMap<string, SurfaceProviderInfo[]>();
const readsBySurface = new SvelteMap<string, SurfaceReadResponse>();
const readRequestedBySurface = new SvelteMap<string, boolean>();
const readLoadPromises = new SvelteMap<string, Promise<void>>();
let loadPromise: Promise<void> | null = null;

const providerAvailabilityOrder: Record<SurfaceProviderInfo['availability'], number> = {
	available: 0,
	disconnected: 1,
	incompatible_tenant: 2
};

function compareSurfaces(a: SurfaceResponse, b: SurfaceResponse): number {
	return a.priority - b.priority || a.label.localeCompare(b.label) || a.surface_id.localeCompare(b.surface_id);
}

function compareProviders(a: SurfaceProviderInfo, b: SurfaceProviderInfo): number {
	return (
		providerAvailabilityOrder[a.availability] - providerAvailabilityOrder[b.availability] ||
		a.display_label.localeCompare(b.display_label) ||
		a.provider_id.localeCompare(b.provider_id)
	);
}

export interface SurfacePageNavItem {
	id: string;
	href: string;
	label: string;
	priority: number;
}

export function resolveSurfacePageNavItems(
	slotSurfaces: SurfaceResponse[],
	rolloutActive: boolean
): SurfacePageNavItem[] {
	if (!rolloutActive) {
		return [];
	}

	const seenSurfaceIds = new SvelteSet<string>();
	const navItems: SurfacePageNavItem[] = [];
	for (const surface of [...slotSurfaces].sort(compareSurfaces)) {
		if (seenSurfaceIds.has(surface.surface_id)) {
			continue;
		}
		seenSurfaceIds.add(surface.surface_id);
		navItems.push({
			id: surface.surface_id,
			href: `/surfaces/${surface.surface_id}`,
			label: surface.label,
			priority: surface.priority
		});
	}
	return navItems;
}

function rebuildIndexes(nextSurfaces: SurfaceResponse[]): void {
	surfacesBySlot.clear();
	for (const surface of nextSurfaces) {
		const existing = surfacesBySlot.get(surface.slot);
		if (existing) {
			existing.push(surface);
		} else {
			surfacesBySlot.set(surface.slot, [surface]);
		}
	}
	for (const [slot, items] of surfacesBySlot) {
		items.sort(compareSurfaces);
		surfacesBySlot.set(slot, items);
	}
}

export function getSurfaceRegistryLoaded(): boolean {
	return surfacesLoaded;
}

export function getSurfaceRegistryLoading(): boolean {
	return surfacesLoading;
}

export function getSurfaceRuntimeStatus(): SurfaceRuntimeStatusResponse {
	return runtimeStatus;
}

export function getAllSurfaces(): SurfaceResponse[] {
	return surfaces;
}

export function getSurfacesBySlot(slot: string): SurfaceResponse[] {
	return surfacesBySlot.get(slot) ?? [];
}

export function getSurfaceById(surfaceId: string): SurfaceResponse | undefined {
	return surfaces.find((surface) => surface.surface_id === surfaceId);
}

export function getSurfaceProviders(surfaceId: string): SurfaceProviderInfo[] {
	return providersBySurface.get(surfaceId) ?? [];
}

export function getSurfaceReadModel(surfaceId: string): SurfaceReadResponse | undefined {
	return readsBySurface.get(surfaceId);
}

export function getSurfaceReadRequested(surfaceId: string): boolean {
	return readRequestedBySurface.get(surfaceId) ?? false;
}

export function getSurfaceReadLoading(surfaceId: string): boolean {
	return readLoadPromises.has(surfaceId);
}

export async function loadSurfaceReadModel(surfaceId: string): Promise<void> {
	readRequestedBySurface.set(surfaceId, true);
	if (readsBySurface.has(surfaceId)) {
		return;
	}
	if (readLoadPromises.has(surfaceId)) {
		await readLoadPromises.get(surfaceId);
		return;
	}

	const promise = (async () => {
		try {
			const read = await getSurfaceRead(surfaceId);
			readsBySurface.set(surfaceId, read);
		} catch (error) {
			console.error(`Failed to load surface read model for ${surfaceId}:`, error);
			readsBySurface.delete(surfaceId);
		}
	})();

	readLoadPromises.set(surfaceId, promise);
	try {
		await promise;
	} finally {
		readLoadPromises.delete(surfaceId);
	}
}

export async function loadSurfaceReadModels(surfaceIds: string[]): Promise<void> {
	const uniqueIds = surfaceIds.filter((surfaceId, index) => surfaceIds.indexOf(surfaceId) === index);
	await Promise.all(uniqueIds.map(async (surfaceId) => loadSurfaceReadModel(surfaceId)));
}

export async function loadSurfaceRegistry(): Promise<void> {
	if (loadPromise) {
		return loadPromise;
	}

	loadPromise = (async () => {
		surfacesLoading = true;
		try {
			const [nextRuntimeStatus, fetchedSurfaces] = await Promise.all([fetchSurfaceRuntimeStatus(), listSurfaces()]);
			const sortedSurfaces = [...fetchedSurfaces].sort(compareSurfaces);
			runtimeStatus = nextRuntimeStatus;
			surfaces = sortedSurfaces;
			rebuildIndexes(sortedSurfaces);
			providersBySurface.clear();
			readsBySurface.clear();
			readRequestedBySurface.clear();
			readLoadPromises.clear();

			const targetedSurfaces = sortedSurfaces.filter((surface) => surface.targeting === 'targeted');
			const providerResults: Array<[string, SurfaceProviderInfo[]]> = await Promise.all(
				targetedSurfaces.map(async (surface): Promise<[string, SurfaceProviderInfo[]]> => {
					try {
						const providers = await listSurfaceProviders(surface.surface_id);
						return [surface.surface_id, [...providers].sort(compareProviders)];
					} catch (error) {
						console.error(`Failed to load providers for surface ${surface.surface_id}:`, error);
						return [surface.surface_id, []];
					}
				})
			);
			for (const [surfaceId, providers] of providerResults) {
				providersBySurface.set(surfaceId, providers);
			}
		} catch (error) {
			console.error('Failed to load surface registry:', error);
			surfaces = [];
			surfacesBySlot.clear();
			providersBySurface.clear();
			readsBySurface.clear();
			readRequestedBySurface.clear();
			readLoadPromises.clear();
			runtimeStatus = { active: false };
		} finally {
			surfacesLoaded = true;
			surfacesLoading = false;
		}
	})();

	try {
		await loadPromise;
	} finally {
		loadPromise = null;
	}
}

export function clearSurfaceRegistry(): void {
	surfaces = [];
	surfacesLoaded = false;
	surfacesLoading = false;
	runtimeStatus = { active: false };
	surfacesBySlot.clear();
	providersBySurface.clear();
	readsBySurface.clear();
	readRequestedBySurface.clear();
	readLoadPromises.clear();
	loadPromise = null;
}
