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
	const dataSourceUsage = collectNodeDataSourceUsage(read.descriptor.root_node);
	for (const dataSourceId of dataSourceUsage.keys()) {
		if (!read.data_sources.some((dataSource) => dataSource.data_source_id === dataSourceId)) {
			return false;
		}
	}

	const dataLoadInteractions = new Set(
		read.interactions
			.filter((interaction) => interaction.kind === 'data_load')
			.map((interaction) => interaction.interaction_id)
	);

	for (const dataSource of read.data_sources) {
		if (isStaticDataSource(dataSource)) {
			continue;
		}
		if (dataSource.kind.kind !== 'provider_query') {
			return false;
		}
		const usageKinds = dataSourceUsage.get(dataSource.data_source_id);
		if (!usageKinds || usageKinds.size !== 1 || !usageKinds.has('key_value')) {
			return false;
		}
		if (!dataLoadInteractions.has(dataSource.kind.operation_id)) {
			return false;
		}
	}

	return true;
}

function collectNodeDataSourceUsage(
	node: SurfaceReadResponse['descriptor']['root_node'],
	out: Map<string, Set<'key_value' | 'table'>> = new Map()
): Map<string, Set<'key_value' | 'table'>> {
	if (node.kind === 'section') {
		for (const child of node.children ?? []) {
			collectNodeDataSourceUsage(child, out);
		}
		return out;
	}
	if (node.kind === 'tabs') {
		for (const tab of node.tabs ?? []) {
			collectNodeDataSourceUsage(tab.root, out);
		}
		return out;
	}
	if (node.kind === 'modal_trigger') {
		for (const modalNode of node.modal_nodes ?? []) {
			collectNodeDataSourceUsage(modalNode, out);
		}
		return out;
	}
	if (node.kind === 'workflow_trigger') {
		for (const stepNode of node.step_nodes ?? []) {
			collectNodeDataSourceUsage(stepNode, out);
		}
		return out;
	}
	if (node.kind === 'key_value' || node.kind === 'table') {
		const kinds = out.get(node.data_source_id);
		if (kinds) {
			kinds.add(node.kind);
		} else {
			out.set(node.data_source_id, new Set([node.kind]));
		}
	}
	return out;
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
