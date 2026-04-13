import type { DataSourceDescriptor, SurfaceReadResponse, SurfaceResponse } from './contract';

function isStaticDataSource(
	dataSource: DataSourceDescriptor
): dataSource is DataSourceDescriptor & { kind: { kind: 'static'; data: unknown } } {
	return dataSource.kind.kind === 'static';
}

export function buildStaticSurfaceData(dataSources: DataSourceDescriptor[]): Record<string, unknown> {
	const dataBySource: Record<string, unknown> = {};
	for (const dataSource of dataSources) {
		if (isStaticDataSource(dataSource)) {
			dataBySource[dataSource.data_source_id] = dataSource.kind.data;
		}
	}
	return dataBySource;
}

export function isSurfaceReadRenderable(read: SurfaceReadResponse): boolean {
	return read.data_sources.every(isStaticDataSource);
}

export function filterSurfacesByPermission<T extends { required_permission?: string }>(
	surfaces: T[],
	canAccess: (requiredPermission: string | undefined) => boolean
): T[] {
	return surfaces.filter((surface) => canAccess(surface.required_permission));
}

export function isSurfaceTabPending(options: {
	rolloutActive: boolean;
	activeTab: string;
	slotSurfaces: SurfaceResponse[];
	readBySurface: Record<string, SurfaceReadResponse>;
	isReadRequested: boolean;
	isReadLoading: boolean;
}): boolean {
	if (!options.rolloutActive) {
		return false;
	}
	if (!options.slotSurfaces.some((surface) => surface.surface_id === options.activeTab)) {
		return false;
	}
	if (options.readBySurface[options.activeTab]) {
		return false;
	}
	if (!options.isReadRequested) {
		return true;
	}
	return options.isReadLoading;
}

export function shouldUseSurfaceRoute(
	rolloutActive: boolean,
	slotSurfaces: SurfaceResponse[],
	readBySurface: Record<string, SurfaceReadResponse>
): boolean {
	if (!rolloutActive || slotSurfaces.length === 0) {
		return false;
	}

	for (const surface of slotSurfaces) {
		const read = readBySurface[surface.surface_id];
		if (!read) {
			return false;
		}
		if (!isSurfaceReadRenderable(read)) {
			return false;
		}
	}
	return true;
}
