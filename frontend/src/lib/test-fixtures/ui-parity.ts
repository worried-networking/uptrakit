import type { SurfaceProviderAvailability, SurfaceProviderInfo, SurfaceResponse } from '$lib/surfaces/contract';

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
			buildParitySurfaceTab('proxmox.hosts', 'Proxmox VE Hosts', {
				slot: 'software.tabs',
				required_permission: 'view_software',
				provider_kind: 'plugin',
				root_node: { kind: 'text_block', text: 'proxmox' }
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
