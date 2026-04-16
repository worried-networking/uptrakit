import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';

vi.mock('$app/state', () => ({
	page: {
		url: new URL('http://localhost/settings?tab=notifications.email')
	}
}));

vi.mock('$app/navigation', () => ({
	goto: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => ({
		id: 'user-1',
		email: 'user@example.com',
		first_name: 'Test',
		last_name: 'User',
		permissions: ['view_notifications', 'update_system_services']
	}))
}));

vi.mock('$lib/api', () => ({
	getCombinedSettings: vi.fn(async () => ({
		registration: {},
		authentication: {},
		agent_certificates: {},
		enrollment_tokens: {},
		multi_tenancy_enabled: false
	})),
	getOidcProviders: vi.fn(async () => [])
}));

vi.mock('$lib/surfaces/registry.svelte', () => ({
	getSurfaceReadLoading: vi.fn(() => false),
	getSurfaceReadModel: vi.fn(() => undefined),
	getSurfaceReadRequested: vi.fn(() => false),
	getSurfaceRegistryLoaded: vi.fn(() => true),
	getSurfaceRuntimeStatus: vi.fn(() => ({ active: true })),
	getSurfacesBySlot: vi.fn((slot: string) =>
		slot === 'settings.tabs'
			? [
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
			: []
	),
	loadSurfaceReadModels: vi.fn(async () => {})
}));

import { goto } from '$app/navigation';
import { getSurfaceRegistryLoaded, getSurfaceRuntimeStatus, getSurfacesBySlot } from '$lib/surfaces/registry.svelte';
import SettingsPage from './+page.svelte';

describe('/settings shared-surface tabs', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		vi.mocked(getSurfaceRegistryLoaded).mockReturnValue(true);
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('shows surface-backed tabs while read models are still pending', () => {
		render(SettingsPage);

		expect(screen.getByRole('button', { name: 'MQTT Clients' })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Email Channels' })).toBeInTheDocument();
	});

	it('does not reset a surface tab from the URL before the registry finishes loading', () => {
		vi.mocked(getSurfaceRegistryLoaded).mockReturnValue(false);
		vi.mocked(getSurfacesBySlot).mockReturnValue([]);

		render(SettingsPage);

		expect(vi.mocked(goto)).not.toHaveBeenCalledWith('/settings', expect.anything());
	});

	it('does not reset a surface tab from the URL while runtime status is still unresolved', () => {
		vi.mocked(getSurfaceRegistryLoaded).mockReturnValue(false);
		vi.mocked(getSurfaceRuntimeStatus).mockReturnValue({ active: false });
		vi.mocked(getSurfacesBySlot).mockReturnValue([]);

		render(SettingsPage);

		expect(vi.mocked(goto)).not.toHaveBeenCalledWith('/settings', expect.anything());
	});
});
