import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import EditHostAssignmentModal from './EditHostAssignmentModal.svelte';
import { PluginCapability } from '$lib/types';
import type {
	HostPluginRoleSummary,
	PluginConfigResponse,
	PluginTypeInfo,
	SoftwareItemDetailResponse
} from '$lib/types';

vi.mock('$lib/api', () => ({
	getPluginConfigs: vi.fn(),
	updateHostAssignment: vi.fn(),
	deletePluginAssignment: vi.fn(),
	listPluginTypes: vi.fn()
}));

vi.mock('$lib/notifications.svelte', () => ({
	showError: vi.fn(),
	showSuccess: vi.fn()
}));

import * as api from '$lib/api';

function makePluginConfigs(): PluginConfigResponse[] {
	return [
		{
			id: 'cfg-standard',
			name: 'Apt Standard',
			plugin_type: 'plugin_standard',
			config: {},
			enabled: true,
			capabilities: [
				PluginCapability.VersionDetection,
				PluginCapability.ReleaseFetching,
				PluginCapability.UpdateExecution
			],
			created_at: '2024-01-01T00:00:00Z',
			updated_at: '2024-01-01T00:00:00Z'
		},
		{
			id: 'cfg-hook',
			name: 'Systemd Hook',
			plugin_type: 'plugin_hook',
			config: {},
			enabled: true,
			capabilities: [PluginCapability.UpdateLifecycle],
			created_at: '2024-01-01T00:00:00Z',
			updated_at: '2024-01-01T00:00:00Z'
		}
	];
}

function makePluginTypes(): PluginTypeInfo[] {
	return [
		{
			plugin_type: 'plugin_standard',
			display_name: 'Standard',
			supports_plugin_configs: true,
			capabilities: [
				PluginCapability.VersionDetection,
				PluginCapability.ReleaseFetching,
				PluginCapability.UpdateExecution
			],
			sample_config: {},
			config_form_fields: []
		},
		{
			plugin_type: 'plugin_hook',
			display_name: 'Hook',
			supports_plugin_configs: true,
			capabilities: [PluginCapability.UpdateLifecycle],
			sample_config: {},
			config_form_fields: []
		}
	];
}

function makeDetail(): SoftwareItemDetailResponse {
	return {
		id: 'software-1',
		name: 'Demo App',
		plugins: ['generic_shell'],
		featured: true,
		last_checked_at: null,
		host_count: 1,
		installed_version: null,
		installed_display_version: null,
		latest_version: null,
		latest_release_metadata: null,
		update_available: false,
		created_at: '2024-01-01T00:00:00Z',
		updated_at: '2024-01-01T00:00:00Z',
		icon_url: null,
		hosts: []
	};
}

function renderModal(existingPlugins: HostPluginRoleSummary[] = []) {
	vi.mocked(api.getPluginConfigs).mockResolvedValue({
		items: makePluginConfigs(),
		total: 2,
		page: 1,
		per_page: 500,
		total_pages: 1
	});
	vi.mocked(api.listPluginTypes).mockResolvedValue(makePluginTypes());
	vi.mocked(api.updateHostAssignment).mockResolvedValue(makeDetail());
	vi.mocked(api.deletePluginAssignment).mockResolvedValue(makeDetail());

	return render(EditHostAssignmentModal, {
		softwareItemId: 'software-1',
		hostId: 'host-1',
		hostName: 'Host One',
		softwareItemName: 'Demo App',
		existingPlugins,
		onclose: vi.fn(),
		onsuccess: vi.fn()
	});
}

describe('EditHostAssignmentModal', () => {
	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
	});

	it('shows inline save error state when no role assignment is configured', async () => {
		const user = userEvent.setup();
		renderModal([]);

		await waitFor(() => expect(screen.getByRole('button', { name: 'Save Changes' })).toBeInTheDocument());
		await user.click(screen.getByRole('button', { name: 'Save Changes' }));

		expect(screen.getByText('Select at least one plugin config to save.')).toBeInTheDocument();
		expect(api.updateHostAssignment).not.toHaveBeenCalled();
	});

	it('requires destructive confirmation before removing a hook entry', async () => {
		const user = userEvent.setup();
		renderModal([
			{
				role: 'pre_update_hook',
				ordinal: 0,
				plugin_config_id: 'cfg-hook',
				plugin_config_name: 'Systemd Hook',
				plugin_type: 'plugin_hook',
				package_identifier: '',
				config_override: null,
				execution_site: 'auto'
			}
		]);

		await waitFor(() => expect(screen.getByRole('button', { name: 'Remove' })).toBeInTheDocument());
		await user.click(screen.getByRole('button', { name: 'Remove' }));

		expect(screen.getByText('Remove Hook Assignment')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Remove Hook' })).toBeInTheDocument();
		expect(screen.queryByText('No pre-update hooks configured.')).not.toBeInTheDocument();

		await user.click(screen.getByRole('button', { name: 'Remove Hook' }));
		expect(screen.getByText('No pre-update hooks configured.')).toBeInTheDocument();
	});

	it('preserves the hook entry when hook-removal confirmation is cancelled', async () => {
		const user = userEvent.setup();
		renderModal([
			{
				role: 'pre_update_hook',
				ordinal: 0,
				plugin_config_id: 'cfg-hook',
				plugin_config_name: 'Systemd Hook',
				plugin_type: 'plugin_hook',
				package_identifier: '',
				config_override: null,
				execution_site: 'auto'
			}
		]);

		await waitFor(() => expect(screen.getByRole('button', { name: 'Remove' })).toBeInTheDocument());
		await user.click(screen.getByRole('button', { name: 'Remove' }));

		const confirmDialog = screen.getByText('Remove Hook Assignment').closest('[role="dialog"]');
		expect(confirmDialog).not.toBeNull();
		await user.click(within(confirmDialog as HTMLElement).getByRole('button', { name: 'Cancel' }));

		expect(screen.queryByText('Remove Hook Assignment')).not.toBeInTheDocument();
		expect(screen.queryByText('No pre-update hooks configured.')).not.toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Remove' })).toBeInTheDocument();
	});
});
