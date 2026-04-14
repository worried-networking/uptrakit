import { describe, expect, it } from 'vitest';
import type { SurfaceReadResponse, SurfaceResponse } from '$lib/surfaces/contract';
import {
	buildStaticSurfaceData,
	filterSurfacesByPermission,
	isSurfaceReadRenderable,
	isSurfaceTabPending,
	shouldUseSurfaceRoute
} from './read-model';

function makeSurface(surfaceId: string): SurfaceResponse {
	return {
		surface_id: surfaceId,
		label: surfaceId,
		priority: 100,
		slot: 'software.tabs',
		scope: 'tenant',
		targeting: 'universal',
		provider_kind: 'service',
		required_capabilities: [],
		root_node: { kind: 'text_block', text: surfaceId },
		provider_count: 1
	};
}

function makeRead(options?: { includeProviderQuery?: boolean }): SurfaceReadResponse {
	return {
		descriptor: {
			surface_id: 'surface.one',
			label: 'Surface One',
			priority: 100,
			slot: 'software.tabs',
			scope: 'tenant',
			targeting: 'universal',
			provider_kind: 'service',
			required_capabilities: [],
			root_node: { kind: 'text_block', text: 'ok' }
		},
		interactions: [],
		data_sources: [
			{
				data_source_id: 'data.static',
				kind: { kind: 'static', data: { version: '1.2.3' } },
				result_schema: 'object',
				refresh_policy: { type: 'manual' }
			},
			...(options?.includeProviderQuery
				? [
						{
							data_source_id: 'data.remote',
							kind: { kind: 'provider_query' as const, operation_id: 'load_data' },
							result_schema: 'array' as const,
							refresh_policy: { type: 'manual' as const }
						}
					]
				: [])
		]
	};
}

function makeProviderQueryRead(options: {
	nodeKind: 'key_value' | 'table';
	includeDataLoadInteraction?: boolean;
}): SurfaceReadResponse {
	return {
		descriptor: {
			surface_id: 'surface.provider',
			label: 'Provider Surface',
			priority: 100,
			slot: 'host_detail.tabs',
			scope: 'tenant',
			targeting: 'universal',
			provider_kind: 'plugin',
			required_capabilities: [],
			root_node: {
				kind: options.nodeKind,
				data_source_id: 'data.remote'
			}
		},
		interactions: options.includeDataLoadInteraction
			? [
					{
						interaction_id: 'get-info',
						kind: 'data_load',
						transport: { mode: 'controller_local' }
					}
				]
			: [],
		data_sources: [
			{
				data_source_id: 'data.remote',
				kind: { kind: 'provider_query', operation_id: 'get-info' },
				result_schema: 'object',
				refresh_policy: { type: 'manual' }
			}
		]
	};
}

describe('surface read model helpers', () => {
	it('extracts static data sources into renderer data map', () => {
		const data = buildStaticSurfaceData(makeRead().data_sources);
		expect(data).toEqual({
			'data.static': { version: '1.2.3' }
		});
	});

	it('marks read payload as non-renderable when unsupported data source kinds are present', () => {
		expect(isSurfaceReadRenderable(makeRead())).toBe(true);
		expect(isSurfaceReadRenderable(makeRead({ includeProviderQuery: true }))).toBe(false);
	});

	it('treats key-value provider-query payloads as renderable when backed by data-load interaction', () => {
		expect(
			isSurfaceReadRenderable(
				makeProviderQueryRead({
					nodeKind: 'key_value',
					includeDataLoadInteraction: true
				})
			)
		).toBe(true);
	});

	it('treats table provider-query payloads as renderable when backed by data-load interaction', () => {
		expect(
			isSurfaceReadRenderable(
				makeProviderQueryRead({
					nodeKind: 'table',
					includeDataLoadInteraction: true
				})
			)
		).toBe(true);
	});

	it('fails closed to legacy route rendering unless all slot surfaces are renderable', () => {
		const surfaces = [makeSurface('surface.one')];
		const readBySurface = {
			'surface.one': makeRead()
		};

		expect(shouldUseSurfaceRoute(false, surfaces, readBySurface)).toBe(false);
		expect(shouldUseSurfaceRoute(true, [], readBySurface)).toBe(false);
		expect(shouldUseSurfaceRoute(true, surfaces, {})).toBe(false);
		expect(shouldUseSurfaceRoute(true, surfaces, readBySurface)).toBe(true);
	});

	it('filters slot surfaces by required permission before rendering', () => {
		const surfaces: SurfaceResponse[] = [
			{
				...makeSurface('surface.public'),
				required_permission: undefined
			},
			{
				...makeSurface('surface.admin'),
				required_permission: 'manage_settings'
			}
		];

		const visible = filterSurfacesByPermission(
			surfaces,
			(requiredPermission) => requiredPermission !== 'manage_settings'
		);

		expect(visible.map((surface) => surface.surface_id)).toEqual(['surface.public']);
	});

	it('only keeps pending surface tabs while waiting for read data', () => {
		const surfaces = [makeSurface('surface.one')];
		const readBySurface = {
			'surface.one': makeRead({ includeProviderQuery: true })
		};

		expect(
			isSurfaceTabPending({
				rolloutActive: true,
				activeTab: 'surface.one',
				slotSurfaces: surfaces,
				readBySurface: {},
				isReadRequested: false,
				isReadLoading: false
			})
		).toBe(true);

		expect(
			isSurfaceTabPending({
				rolloutActive: true,
				activeTab: 'surface.one',
				slotSurfaces: surfaces,
				readBySurface,
				isReadRequested: true,
				isReadLoading: false
			})
		).toBe(false);

		expect(
			isSurfaceTabPending({
				rolloutActive: true,
				activeTab: 'surface.one',
				slotSurfaces: surfaces,
				readBySurface: {},
				isReadRequested: true,
				isReadLoading: false
			})
		).toBe(false);
	});
});
