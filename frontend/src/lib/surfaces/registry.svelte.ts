import { SvelteMap, SvelteSet } from 'svelte/reactivity';
import { getSurfaceRead, listSurfaceProviders, listSurfaces } from '$lib/api';
import type { SurfaceProviderInfo, SurfaceReadResponse, SurfaceResponse } from './contract';

let surfaces: SurfaceResponse[] = $state([]);
let surfacesLoaded = $state(false);
let surfacesLoading = $state(false);

const surfacesBySlot = new SvelteMap<string, SurfaceResponse[]>();
const providersBySurface = new SvelteMap<string, SurfaceProviderInfo[]>();
const readsBySurface = new SvelteMap<string, SurfaceReadResponse>();
const readRequestedBySurface = new SvelteMap<string, boolean>();
const readLoadPromises = new SvelteMap<string, Promise<void>>();
const failedReads = new SvelteSet<string>();
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
	icon: string;
}

export function resolveSurfacePageNavItems(slotSurfaces: SurfaceResponse[]): SurfacePageNavItem[] {
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
			priority: surface.priority,
			icon: surface.nav_icon ?? 'box'
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

export function getSurfaceReadFailed(surfaceId: string): boolean {
	return failedReads.has(surfaceId);
}

export async function loadSurfaceReadModel(surfaceId: string): Promise<void> {
	readRequestedBySurface.set(surfaceId, true);
	if (readsBySurface.has(surfaceId) || readLoadPromises.has(surfaceId) || failedReads.has(surfaceId)) {
		if (readLoadPromises.has(surfaceId)) {
			await readLoadPromises.get(surfaceId);
		}
		return;
	}

	const promise = (async () => {
		try {
			const { data } = await getSurfaceRead({ path: { surface_id: surfaceId } });
			const read = data as unknown as SurfaceReadResponse;
			readsBySurface.set(surfaceId, read);
			failedReads.delete(surfaceId);
		} catch (error) {
			console.error(`Failed to load surface read model for ${surfaceId}:`, error);
			failedReads.add(surfaceId);
		}
	})();

	readLoadPromises.set(surfaceId, promise);
	try {
		await promise;
	} finally {
		readLoadPromises.delete(surfaceId);
	}
}

/** Clears a failed-read mark and re-fetches. The load path retains failures
 *  in `failedReads` instead of re-arming (loop prevention); this is the
 *  explicit retry affordance for surface UIs and for the per-navigation
 *  eviction below. Mutates ONLY `failedReads` — never deletes
 *  `readsBySurface`/`readRequestedBySurface`. */
export async function refreshSurfaceReadModel(surfaceId: string): Promise<void> {
	failedReads.delete(surfaceId);
	await loadSurfaceReadModel(surfaceId);
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
			const { data } = await listSurfaces();
			const fetchedSurfaces = (data ?? []) as unknown as SurfaceResponse[];
			const sortedSurfaces = [...fetchedSurfaces].sort(compareSurfaces);
			surfaces = sortedSurfaces;
			rebuildIndexes(sortedSurfaces);
			providersBySurface.clear();
			readsBySurface.clear();
			readRequestedBySurface.clear();
			readLoadPromises.clear();
			failedReads.clear();

			const targetedSurfaces = sortedSurfaces.filter((surface) => surface.targeting === 'targeted');
			const providerResults: Array<[string, SurfaceProviderInfo[]]> = await Promise.all(
				targetedSurfaces.map(async (surface): Promise<[string, SurfaceProviderInfo[]]> => {
					try {
						const { data: providerData } = await listSurfaceProviders({
							path: { surface_id: surface.surface_id }
						});
						const providers = (providerData ?? []) as unknown as SurfaceProviderInfo[];
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
			failedReads.clear();
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
	surfacesBySlot.clear();
	providersBySurface.clear();
	readsBySurface.clear();
	readRequestedBySurface.clear();
	readLoadPromises.clear();
	failedReads.clear();
	loadPromise = null;
}
