import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import AssignToHostModal from './AssignToHostModal.svelte';
import { PluginCapability } from '$lib/types';
import type { HostResponse, PluginConfigResponse, PluginTypeInfo, SoftwareItemDetailResponse } from '$lib/types';

vi.mock('$lib/api', () => ({
	getSoftwareItem: vi.fn(),
	getHosts: vi.fn(),
	assignHostsToSoftwareItem: vi.fn(),
	unassignHostFromSoftwareItem: vi.fn(),
	getPluginConfigs: vi.fn(),
	listPluginTypes: vi.fn()
}));

vi.mock('$lib/notifications.svelte', () => ({
	showError: vi.fn(),
	showSuccess: vi.fn()
}));

import * as api from '$lib/api';

function makeHostsPage(items: HostResponse[]) {
	return {
		items,
		total: items.length,
		page: 1,
		per_page: 200,
		total_pages: 1
	};
}

function makeHost(id: string, name: string): HostResponse {
	return {
		id,
		machine_id: `machine-${id}`,
		hostname: name.toLowerCase().replace(/\s+/g, '-'),
		friendly_name: name,
		os_type: 'Linux',
		os_version: '6.8',
		architecture: 'x86_64',
		ip_address: '10.0.0.10',
		last_seen_at: '2024-01-01T00:00:00Z',
		created_at: '2024-01-01T00:00:00Z',
		updated_at: '2024-01-01T00:00:00Z',
		agents: [],
		tags: [],
		software_status: { known: true, update_count: 1, error_count: 0 }
	};
}

function makeDetail(hostIds: string[] = []): SoftwareItemDetailResponse {
	return {
		id: 'software-1',
		name: 'Demo App',
		plugins: ['generic_shell'],
		featured: true,
		last_checked_at: null,
		host_count: hostIds.length,
		installed_version: null,
		installed_display_version: null,
		latest_version: null,
		latest_release_metadata: null,
		update_available: false,
		created_at: '2024-01-01T00:00:00Z',
		updated_at: '2024-01-01T00:00:00Z',
		icon_url: null,
		hosts: hostIds.map((hostId, index) => ({
			id: `row-${hostId}`,
			host_id: hostId,
			hostname: `host-${index + 1}`,
			friendly_name: `Host ${index + 1}`,
			qualifier: null,
			installed_version: '1.0.0',
			installed_version_detected_at: '2024-01-01T00:00:00Z',
			installed_display_version: null,
			latest_version: '1.1.0',
			latest_release_metadata: null,
			update_available: true,
			active_update_history_id: null,
			last_updated_at: null,
			linked_at: '2024-01-01T00:00:00Z',
			plugins: []
		}))
	};
}

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

function renderModal(hostItems: HostResponse[] = [makeHost('host-1', 'Host One')]) {
	vi.mocked(api.getSoftwareItem).mockResolvedValue(makeDetail([]));
	vi.mocked(api.getHosts).mockResolvedValue(makeHostsPage(hostItems));
	vi.mocked(api.getPluginConfigs).mockResolvedValue({
		items: makePluginConfigs(),
		total: 2,
		page: 1,
		per_page: 500,
		total_pages: 1
	});
	vi.mocked(api.listPluginTypes).mockResolvedValue(makePluginTypes());
	vi.mocked(api.assignHostsToSoftwareItem).mockResolvedValue(makeDetail([]));
	vi.mocked(api.unassignHostFromSoftwareItem).mockResolvedValue();

	return render(AssignToHostModal, {
		softwareItemId: 'software-1',
		softwareItemName: 'Demo App',
		onclose: vi.fn(),
		onsuccess: vi.fn()
	});
}

describe('AssignToHostModal', () => {
	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
	});

	it('renders the host empty state when there are no hosts to assign', async () => {
		renderModal([]);

		expect(
			await screen.findByText(
				'No hosts are registered yet. Hosts appear once an approved agent reports from a machine.'
			)
		).toBeInTheDocument();
	});

	it('shows inline role-assignment validation for new hosts before submit', async () => {
		const user = userEvent.setup();
		renderModal();

		const hostCheckbox = await screen.findByRole('checkbox', { name: /Host One/i });
		await user.click(hostCheckbox);

		const detectRow = screen.getByText('Detect Version').closest('tr');
		expect(detectRow).not.toBeNull();
		const detectConfigSelect = within(detectRow!).getAllByRole('combobox')[0];
		await fireEvent.change(detectConfigSelect, { target: { value: '' } });

		await user.click(screen.getByRole('button', { name: 'Save' }));

		expect(screen.getByText('Select a plugin config for Detect Version.')).toBeInTheDocument();
		expect(api.assignHostsToSoftwareItem).not.toHaveBeenCalled();
	});

	it('shows saving state while assignment submit is in flight', async () => {
		const user = userEvent.setup();
		let resolveAssign: (() => void) | null = null;
		renderModal();
		vi.mocked(api.assignHostsToSoftwareItem).mockImplementation(
			() =>
				new Promise((resolve) => {
					resolveAssign = () => resolve(makeDetail([]));
				})
		);

		const hostCheckbox = await screen.findByRole('checkbox', { name: /Host One/i });
		await user.click(hostCheckbox);

		await user.click(screen.getByRole('button', { name: 'Save' }));
		expect(await screen.findByRole('button', { name: 'Saving...' })).toBeDisabled();

		expect(resolveAssign).not.toBeNull();
		resolveAssign!();
		await waitFor(() => expect(api.assignHostsToSoftwareItem).toHaveBeenCalledTimes(1));
	});
});
