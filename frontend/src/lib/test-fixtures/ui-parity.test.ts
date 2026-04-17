import { describe, expect, it } from 'vitest';

import {
	buildSharedVisualParityFixture,
	buildParitySurfacePageFixture,
	buildParitySurfaceTab,
	buildSettingsTabsParityFixture,
	buildSoftwareTabsParityFixture
} from './ui-parity';

describe('ui parity fixtures', () => {
	it('builds parity surface tabs with stable defaults', () => {
		expect(buildParitySurfaceTab('surface.alpha', 'Surface Alpha')).toEqual({
			surface_id: 'surface.alpha',
			label: 'Surface Alpha',
			priority: 100,
			slot: 'settings.tabs',
			scope: 'tenant',
			targeting: 'universal',
			provider_kind: 'service',
			required_capabilities: [],
			root_node: { kind: 'text_block', text: 'surface.alpha' },
			provider_count: 1
		});
	});

	it('builds deterministic settings and software parity scenarios', () => {
		expect(buildSettingsTabsParityFixture()).toEqual({
			builtInTabs: [
				{ id: 'general', label: 'General' },
				{ id: 'plugin-configs', label: 'Plugin Configs' },
				{ id: 'scheduler', label: 'Scheduler' },
				{ id: 'global-settings', label: 'Global Settings' },
				{ id: 'notification-rules', label: 'Notification Rules' },
				{ id: 'notification-log', label: 'Notification Log' }
			],
			surfaceTabs: [
				{
					surface_id: 'mqtt.clients',
					label: 'MQTT Clients',
					priority: 100,
					slot: 'settings.tabs',
					scope: 'tenant',
					targeting: 'targeted',
					required_permission: 'update_system_services',
					provider_kind: 'service',
					required_capabilities: [],
					root_node: { kind: 'text_block', text: 'mqtt' },
					provider_count: 1
				},
				{
					surface_id: 'notifications.email',
					label: 'Email Channels',
					priority: 101,
					slot: 'settings.tabs',
					scope: 'global',
					targeting: 'universal',
					required_permission: 'view_notifications',
					provider_kind: 'plugin',
					required_capabilities: [],
					root_node: { kind: 'text_block', text: 'email' },
					provider_count: 1
				}
			]
		});

		expect(buildSoftwareTabsParityFixture()).toEqual({
			builtInTabs: [
				{ id: 'all', label: 'All' },
				{ id: 'featured', label: 'Featured' },
				{ id: 'unfeatured', label: 'Unfeatured' },
				{ id: 'ignores', label: 'Ignore Rules' }
			],
			surfaceTabs: [
				{
					surface_id: 'proxmox.hosts',
					label: 'Proxmox VE Hosts',
					priority: 100,
					slot: 'software.tabs',
					scope: 'tenant',
					targeting: 'universal',
					required_permission: 'view_software',
					provider_kind: 'plugin',
					required_capabilities: [],
					root_node: { kind: 'text_block', text: 'proxmox' },
					provider_count: 1
				}
			]
		});
	});

	it('keeps surface-page parity fixtures fixed for provider count and availability', () => {
		expect(buildParitySurfacePageFixture()).toEqual({
			surface: {
				surface_id: 'surface.one',
				label: 'Surface One',
				priority: 100,
				slot: 'surface.page',
				scope: 'tenant',
				targeting: 'universal',
				provider_kind: 'service',
				required_capabilities: [],
				root_node: { kind: 'text_block', text: 'surface' },
				provider_count: 1
			},
			providers: [
				{
					provider_id: 'provider.primary',
					display_label: 'Primary Provider',
					service_id: 'service.primary',
					availability: 'available'
				}
			]
		});
	});

	it('builds deterministic shared visual parity fixtures for action badges, pill badges, context menus, and table footer rows', () => {
		expect(buildSharedVisualParityFixture()).toEqual({
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
				surface: {
					surface_id: 'surface.table-footer',
					label: 'Table Footer Surface',
					priority: 100,
					slot: 'surface.page',
					scope: 'tenant',
					targeting: 'universal',
					provider_kind: 'service',
					required_capabilities: [],
					root_node: {
						kind: 'table',
						data_source_id: 'table-footer.data',
						columns: [
							{ key: 'name', label: 'Name' },
							{ key: 'status', label: 'Status' }
						]
					},
					provider_count: 1
				},
				readModel: {
					descriptor: {
						surface_id: 'surface.table-footer',
						label: 'Table Footer Surface',
						priority: 100,
						slot: 'surface.page',
						scope: 'tenant',
						targeting: 'universal',
						provider_kind: 'service',
						required_capabilities: [],
						root_node: {
							kind: 'table',
							data_source_id: 'table-footer.data',
							columns: [
								{ key: 'name', label: 'Name' },
								{ key: 'status', label: 'Status' }
							]
						}
					},
					interactions: [
						{
							interaction_id: 'table-footer.load',
							kind: 'data_load',
							label: 'Load table footer parity data',
							input_schema: 'object',
							result_schema: 'object',
							transport: { mode: 'provider_proxied' }
						}
					],
					data_sources: [
						{
							data_source_id: 'table-footer.data',
							kind: { kind: 'provider_query', operation_id: 'table-footer.load' },
							result_schema: 'object',
							pagination: { default_page_size: 3, max_page_size: 3 },
							refresh_policy: { type: 'manual' },
							empty_state: {
								title: 'No rows available',
								description: 'No rows available for parity fixture.'
							}
						}
					]
				},
				dataLoadInteractionId: 'table-footer.load',
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
			}
		});
	});
});
