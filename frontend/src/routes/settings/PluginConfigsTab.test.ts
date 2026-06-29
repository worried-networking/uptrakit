import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
	listPluginConfigs: vi
		.fn()
		.mockResolvedValue({ data: { items: [], total: 0, page: 1, per_page: 20, total_pages: 0 } }),
	createPluginConfig: vi.fn(),
	updatePluginConfig: vi.fn(),
	deletePluginConfig: vi.fn(),
	discoverPluginConfig: vi.fn(),
	listTenantDiscoveryAllowlist: vi.fn().mockResolvedValue({ data: [] }),
	addTenantDiscoveryAllowlistEntry: vi.fn(),
	removeTenantDiscoveryAllowlistEntry: vi.fn(),
	listPluginTypes: vi.fn().mockResolvedValue({ data: [] }),
	batchPluginConfigs: vi.fn(),
	listPluginTypeSettings: vi.fn().mockResolvedValue({ data: [] }),
	upsertPluginTypeSettings: vi.fn(),
	deletePluginTypeSettings: vi.fn(),
	testPluginConfig: vi.fn(),
	listInstancePlugins: vi.fn().mockResolvedValue({ data: [] }),
	setInstancePluginEnabled: vi.fn(),
	upsertInstancePluginConfig: vi.fn()
}));
vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => ({
		id: 'u1',
		email: 'a@b.com',
		first_name: 'A',
		last_name: 'B',
		has_pending_email_change: false,
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

import { Permission, type InstancePluginSummary } from '$lib/types';
import * as auth from '$lib/auth.svelte';
import * as api from '$lib/api';
import PluginConfigsTab from './PluginConfigsTab.svelte';

describe('PluginConfigsTab button variants', () => {
	it('has no raw preset-filled-primary-500 buttons', async () => {
		vi.mocked(auth.getUser).mockReturnValue({
			id: 'u1',
			email: 'a@b.com',
			first_name: 'A',
			last_name: 'B',
			has_pending_email_change: false,
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

describe('PluginConfigsTab — Instance Plugins section', () => {
	const dashboardIconsPlugin: InstancePluginSummary = {
		plugin_type: 'dashboard-icons',
		display_name: 'Dashboard Icons',
		enabled: false,
		running_enabled: false,
		has_instance_config: false,
		current_config: {},
		updated_at: null
	};

	it('renders the section when user has ManageGlobalSettings', async () => {
		vi.mocked(api.listInstancePlugins).mockResolvedValue({ data: [dashboardIconsPlugin] } as unknown as Awaited<
			ReturnType<typeof api.listInstancePlugins>
		>);
		render(PluginConfigsTab);
		expect(await screen.findByText('Instance Plugins')).toBeTruthy();
		expect(await screen.findByText('Dashboard Icons')).toBeTruthy();
	});

	it('does not render the section when user lacks ManageGlobalSettings', async () => {
		vi.mocked(auth.getUser).mockReturnValueOnce({
			id: 'u1',
			email: 'a@b.com',
			first_name: 'A',
			last_name: 'B',
			has_pending_email_change: false,
			permissions: [
				Permission.ViewSoftware,
				Permission.ManageCommands,
				Permission.TriggerChecks,
				Permission.UpdateSoftware,
				Permission.TestPluginConfigs
			]
		} as ReturnType<typeof auth.getUser>);
		vi.mocked(api.listInstancePlugins).mockResolvedValue({ data: [dashboardIconsPlugin] } as unknown as Awaited<
			ReturnType<typeof api.listInstancePlugins>
		>);
		render(PluginConfigsTab);
		await waitFor(() => {
			expect(screen.queryByText('Instance Plugins')).toBeNull();
		});
	});

	it('shows "Pending restart" badge when stored enabled differs from running_enabled', async () => {
		vi.mocked(api.listInstancePlugins).mockResolvedValue({
			data: [
				{
					...dashboardIconsPlugin,
					enabled: true,
					running_enabled: false
				}
			]
		} as unknown as Awaited<ReturnType<typeof api.listInstancePlugins>>);
		render(PluginConfigsTab);
		expect(await screen.findByText('Pending restart')).toBeTruthy();
	});

	it('hides the Edit Settings button when has_instance_config is false', async () => {
		vi.mocked(api.listInstancePlugins).mockResolvedValue({ data: [dashboardIconsPlugin] } as unknown as Awaited<
			ReturnType<typeof api.listInstancePlugins>
		>);
		render(PluginConfigsTab);
		// Wait for the plugin row to render before asserting absence.
		await screen.findByText('Dashboard Icons');
		expect(screen.queryByText('Edit Settings')).toBeNull();
	});

	it('toggle button opens confirm dialog with restart-required copy', async () => {
		vi.mocked(api.listInstancePlugins).mockResolvedValue({ data: [dashboardIconsPlugin] } as unknown as Awaited<
			ReturnType<typeof api.listInstancePlugins>
		>);
		render(PluginConfigsTab);
		const enableButton = await screen.findByRole('button', { name: 'Enable' });
		await fireEvent.click(enableButton);
		expect(await screen.findByText('Restart the controller to apply this change.')).toBeTruthy();
		expect(await screen.findByText(/Enable Dashboard Icons/)).toBeTruthy();
	});
});
