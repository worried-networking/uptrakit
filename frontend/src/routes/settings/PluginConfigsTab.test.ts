import { describe, expect, it, vi } from 'vitest';
import { render, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
	getPluginConfigs: vi.fn().mockResolvedValue({ items: [], total: 0, page: 1, per_page: 20, pages: 0 }),
	createPluginConfig: vi.fn(),
	updatePluginConfig: vi.fn(),
	deletePluginConfig: vi.fn(),
	triggerPluginConfigDiscovery: vi.fn(),
	listDiscoveryAllowlist: vi.fn().mockResolvedValue({ items: [], total: 0, page: 1, per_page: 20, pages: 0 }),
	addDiscoveryAllowlistEntry: vi.fn(),
	deleteDiscoveryAllowlistEntry: vi.fn(),
	listPluginTypes: vi.fn().mockResolvedValue([]),
	batchPluginConfigs: vi.fn(),
	listPluginTypeSettings: vi.fn().mockResolvedValue([]),
	upsertPluginTypeSettings: vi.fn(),
	deletePluginTypeSettings: vi.fn(),
	testPluginConfig: vi.fn()
}));
vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => ({
		id: 'u1',
		email: 'a@b.com',
		first_name: 'A',
		last_name: 'B',
		permissions: [
			'view_software',
			'manage_commands',
			'trigger_checks',
			'update_software',
			'manage_global_settings',
			'test_plugin_configs'
		]
	}))
}));
vi.mock('$lib/notifications.svelte', () => ({ showSuccess: vi.fn(), showError: vi.fn() }));
vi.mock('$lib/stores/events.svelte', () => ({
	subscribe: vi.fn(() => () => {}),
	getLastEvent: vi.fn(() => null)
}));

import { Permission } from '$lib/types';
import * as auth from '$lib/auth.svelte';
import PluginConfigsTab from './PluginConfigsTab.svelte';

describe('PluginConfigsTab button variants', () => {
	it('has no raw preset-filled-primary-500 buttons', async () => {
		vi.mocked(auth.getUser).mockReturnValue({
			id: 'u1',
			email: 'a@b.com',
			first_name: 'A',
			last_name: 'B',
			permissions: [
				Permission.ViewSoftware,
				Permission.ManageCommands,
				Permission.TriggerChecks,
				Permission.UpdateSoftware,
				Permission.ManageGlobalSettings,
				Permission.TestPluginConfigs
			]
		} as ReturnType<typeof auth.getUser>);
		const { container } = render(PluginConfigsTab);
		await waitFor(() => expect(container.querySelector('button.preset-filled-primary-500')).toBeNull());
	});

	it('has no raw preset-tonal-error buttons', async () => {
		const { container } = render(PluginConfigsTab);
		await waitFor(() => expect(container.querySelector('button.preset-tonal-error')).toBeNull());
	});

	it('has no raw preset-tonal-surface buttons', async () => {
		const { container } = render(PluginConfigsTab);
		await waitFor(() => expect(container.querySelector('button.preset-tonal-surface')).toBeNull());
	});

	it('Add Config button has primary gradient class', async () => {
		const { container } = render(PluginConfigsTab);
		await waitFor(() => {
			const btn = Array.from(container.querySelectorAll('button')).find((b) => b.textContent?.trim() === 'Add Config');
			expect(btn?.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
		});
	});
});
