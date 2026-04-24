import type {
	DataSourceDescriptor,
	InteractionDescriptor,
	SurfaceProviderAvailability,
	SurfaceProviderInfo,
	SurfaceReadResponse,
	SurfaceResponse
} from '$lib/surfaces/contract';

export interface ParityBuiltInTab {
	id: string;
	label: string;
}

interface ParityTabScenario {
	builtInTabs: ParityBuiltInTab[];
	surfaceTabs: SurfaceResponse[];
}

interface ParitySurfacePageFixture {
	surface: SurfaceResponse;
	providers: SurfaceProviderInfo[];
}

export interface SharedActionBadgeParityFixture {
	idleLabel: string;
	hoverLabel: string;
	variant: 'navigation' | 'bulk-update';
	tone: 'info' | 'accent' | 'danger';
}

export interface SharedPillBadgeParityFixture {
	host: {
		id: string;
		friendlyName: string;
		tagName: string;
	};
}

export interface SharedContextMenuParityFixture {
	service: {
		id: string;
		friendlyName: string;
	};
}

export interface SharedTableFooterParityFixture {
	surface: SurfaceResponse;
	readModel: SurfaceReadResponse;
	dataLoadResponse: {
		items: Array<Record<string, unknown>>;
		total: number;
		page: number;
		per_page: number;
		total_pages: number;
	};
	dataLoadInteractionId: string;
}

export interface SharedEntityLinkParityFixture {
	surface: SurfaceResponse;
	readModel: SurfaceReadResponse;
	dataLoadInteractionId: string;
	dataLoadResponse: {
		items: Array<Record<string, unknown>>;
		total: number;
		page: number;
		per_page: number;
		total_pages: number;
	};
}

export interface SharedVisualParityFixture {
	actionBadge: SharedActionBadgeParityFixture;
	pillBadge: SharedPillBadgeParityFixture;
	contextMenu: SharedContextMenuParityFixture;
	tableFooter: SharedTableFooterParityFixture;
	entityLink: SharedEntityLinkParityFixture;
}

type SurfaceTabOverrides = Partial<Omit<SurfaceResponse, 'surface_id' | 'label'>>;

const SETTINGS_BUILT_IN_TABS: ParityBuiltInTab[] = [
	{ id: 'general', label: 'General' },
	{ id: 'plugin-configs', label: 'Plugin Configs' },
	{ id: 'scheduler', label: 'Scheduler' },
	{ id: 'global-settings', label: 'Global Settings' },
	{ id: 'notification-rules', label: 'Notification Rules' },
	{ id: 'notification-log', label: 'Notification Log' }
];

const SOFTWARE_BUILT_IN_TABS: ParityBuiltInTab[] = [
	{ id: 'all', label: 'All' },
	{ id: 'featured', label: 'Featured' },
	{ id: 'unfeatured', label: 'Unfeatured' },
	{ id: 'ignores', label: 'Ignore Rules' }
];

export function buildParitySurfaceTab(
	surfaceId: string,
	label: string,
	overrides: SurfaceTabOverrides = {}
): SurfaceResponse {
	return {
		surface_id: surfaceId,
		label,
		priority: 100,
		slot: 'settings.tabs',
		scope: 'tenant',
		targeting: 'universal',
		provider_kind: 'service',
		required_capabilities: [],
		root_node: { kind: 'text_block', text: surfaceId },
		provider_count: 1,
		...overrides
	};
}

export function buildParityProvider(
	availability: SurfaceProviderAvailability = 'available',
	overrides: Partial<SurfaceProviderInfo> = {}
): SurfaceProviderInfo {
	return {
		provider_id: 'provider.primary',
		display_label: 'Primary Provider',
		service_id: 'service.primary',
		availability,
		...overrides
	};
}

export function buildSettingsTabsParityFixture(): ParityTabScenario {
	return {
		builtInTabs: SETTINGS_BUILT_IN_TABS.map((tab) => ({ ...tab })),
		surfaceTabs: [
			buildParitySurfaceTab('mqtt.clients', 'MQTT Clients', {
				slot: 'settings.tabs',
				scope: 'tenant',
				targeting: 'targeted',
				required_permission: 'update_system_services',
				provider_kind: 'service',
				root_node: { kind: 'text_block', text: 'mqtt' }
			}),
			buildParitySurfaceTab('notifications.email', 'Email Channels', {
				priority: 101,
				slot: 'settings.tabs',
				scope: 'global',
				required_permission: 'view_notifications',
				provider_kind: 'plugin',
				root_node: { kind: 'text_block', text: 'email' }
			})
		]
	};
}

export function buildSoftwareTabsParityFixture(): ParityTabScenario {
	return {
		builtInTabs: SOFTWARE_BUILT_IN_TABS.map((tab) => ({ ...tab })),
		surfaceTabs: [
			buildParitySurfaceTab('plugin.software-category', 'Plugin Category', {
				slot: 'software.tabs',
				required_permission: 'view_software',
				provider_kind: 'plugin',
				root_node: { kind: 'text_block', text: 'plugin-category' }
			})
		]
	};
}

export function buildParitySurfacePageFixture(): ParitySurfacePageFixture {
	const providers = [buildParityProvider()];

	return {
		surface: buildParitySurfaceTab('surface.one', 'Surface One', {
			slot: 'surface.page',
			provider_kind: 'service',
			root_node: { kind: 'text_block', text: 'surface' },
			provider_count: providers.length
		}),
		providers
	};
}

export function buildSharedVisualParityFixture(): SharedVisualParityFixture {
	const dataLoadInteractionId = 'table-footer.load';
	const dataSourceId = 'table-footer.data';
	const surface = buildParitySurfaceTab('surface.table-footer', 'Table Footer Surface', {
		slot: 'surface.page',
		provider_kind: 'service',
		root_node: {
			kind: 'table',
			data_source_id: dataSourceId,
			columns: [
				{ key: 'name', label: 'Name' },
				{ key: 'status', label: 'Status' }
			]
		}
	});

	const interactions: InteractionDescriptor[] = [
		{
			interaction_id: dataLoadInteractionId,
			kind: 'data_load',
			label: 'Load table footer parity data',
			input_schema: 'object',
			result_schema: 'object',
			transport: { mode: 'provider_proxied' }
		}
	];

	const dataSources: DataSourceDescriptor[] = [
		{
			data_source_id: dataSourceId,
			kind: { kind: 'provider_query', operation_id: dataLoadInteractionId },
			result_schema: 'object',
			pagination: { default_page_size: 3, max_page_size: 3 },
			refresh_policy: { type: 'manual' },
			empty_state: {
				title: 'No rows available',
				description: 'No rows available for parity fixture.'
			}
		}
	];

	const { provider_count: _providerCount, ...descriptor } = surface;
	const readModel: SurfaceReadResponse = {
		descriptor,
		interactions,
		data_sources: dataSources
	};

	const entityLinkDataLoadInteractionId = 'entity-link.load';
	const entityLinkDataSourceId = 'entity-link.data';
	const entityLinkSurface = buildParitySurfaceTab('surface.entity-link', 'Entity Link Surface', {
		slot: 'surface.page',
		provider_kind: 'service',
		root_node: {
			kind: 'table',
			data_source_id: entityLinkDataSourceId,
			columns: [
				{ key: 'label_col', label: 'Label' },
				{ key: 'host_col', label: 'Host', cell_type: { kind: 'entity_link', entity_type: 'host' } },
				{
					key: 'future_col',
					label: 'Future',
					cell_type: { kind: 'entity_link', entity_type: 'future_entity' }
				}
			]
		}
	});

	const entityLinkInteractions: InteractionDescriptor[] = [
		{
			interaction_id: entityLinkDataLoadInteractionId,
			kind: 'data_load',
			label: 'Load entity link parity data',
			input_schema: 'object',
			result_schema: 'object',
			transport: { mode: 'provider_proxied' }
		}
	];

	const entityLinkDataSources: DataSourceDescriptor[] = [
		{
			data_source_id: entityLinkDataSourceId,
			kind: { kind: 'provider_query', operation_id: entityLinkDataLoadInteractionId },
			result_schema: 'object',
			pagination: { default_page_size: 10, max_page_size: 10 },
			refresh_policy: { type: 'manual' },
			empty_state: {
				title: 'No rows available',
				description: 'No rows available for entity link parity fixture.'
			}
		}
	];

	const { provider_count: _entityLinkProviderCount, ...entityLinkDescriptor } = entityLinkSurface;
	const entityLinkReadModel: SurfaceReadResponse = {
		descriptor: entityLinkDescriptor,
		interactions: entityLinkInteractions,
		data_sources: entityLinkDataSources
	};

	const entityLinkDataLoadResponse = {
		items: [
			{
				label_col: 'found – link',
				host_col: { entity_id: '00000000-0000-0000-0000-000000000001', label: 'web-01', found: true },
				future_col: null
			},
			{
				label_col: 'found – no route',
				host_col: null,
				future_col: { entity_id: '00000000-0000-0000-0000-000000000002', label: 'node-02', found: true }
			},
			{
				label_col: 'not found',
				host_col: { entity_id: '00000000-0000-0000-0000-000000000003', found: false },
				future_col: null
			},
			{
				label_col: 'unenriched',
				host_col: { entity_id: '00000000-0000-0000-0000-000000000004' },
				future_col: null
			},
			{
				label_col: 'null cell',
				host_col: null,
				future_col: null
			}
		],
		total: 5,
		page: 1,
		per_page: 10,
		total_pages: 1
	};

	return {
		actionBadge: {
			idleLabel: '2 updates',
			hoverLabel: '→ Software',
			variant: 'navigation',
			tone: 'info'
		},
		pillBadge: {
			host: {
				id: 'host-pill-001',
				friendlyName: 'Pill Badge Host',
				tagName: 'SSH Agent'
			}
		},
		contextMenu: {
			service: {
				id: 'service-menu-001',
				friendlyName: 'Parity Service'
			}
		},
		tableFooter: {
			surface,
			readModel,
			dataLoadInteractionId,
			dataLoadResponse: {
				items: [
					{ name: 'row-4', status: 'ok' },
					{ name: 'row-5', status: 'ok' },
					{ name: 'row-6', status: 'warning' }
				],
				total: 9,
				page: 2,
				per_page: 3,
				total_pages: 3
			}
		},
		entityLink: {
			surface: entityLinkSurface,
			readModel: entityLinkReadModel,
			dataLoadInteractionId: entityLinkDataLoadInteractionId,
			dataLoadResponse: entityLinkDataLoadResponse
		}
	};
}
