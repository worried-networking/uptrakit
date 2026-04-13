import { SvelteMap } from 'svelte/reactivity';
import { getSurfaceRuntimeStatus as fetchSurfaceRuntimeStatus, listSurfaceProviders, listSurfaces } from '$lib/api';
import type { SurfaceProviderInfo, SurfaceResponse, SurfaceRuntimeStatusResponse } from './contract';

let surfaces: SurfaceResponse[] = $state([]);
let surfacesLoaded = $state(false);
let surfacesLoading = $state(false);
let runtimeStatus: SurfaceRuntimeStatusResponse = $state({ active: false });

const surfacesBySlot = new SvelteMap<string, SurfaceResponse[]>();
const providersBySurface = new SvelteMap<string, SurfaceProviderInfo[]>();
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

export interface ExtensionPageNavItem {
	id: string;
	href: string;
	label: string;
	priority: number;
}

export function resolveExtensionPageNavItems(
	legacyNavItems: ExtensionPageNavItem[],
	slotSurfaces: SurfaceResponse[],
	rolloutActive: boolean
): ExtensionPageNavItem[] {
	if (!rolloutActive) {
		return legacyNavItems;
	}

	// Compatibility rule for Task 4: only surfaces that match existing legacy
	// extension ids are allowed to influence /extensions/[id] nav.
	const surfaceByLegacyId: Record<string, SurfaceResponse> = {};
	for (const surface of [...slotSurfaces].sort(compareSurfaces)) {
		if (!(surface.surface_id in surfaceByLegacyId)) {
			surfaceByLegacyId[surface.surface_id] = surface;
		}
	}

	return legacyNavItems.map((item) => {
		const surface = surfaceByLegacyId[item.id];
		if (!surface) {
			return item;
		}
		return {
			...item,
			label: surface.label,
			priority: surface.priority
		};
	});
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
	loadPromise = null;
}
