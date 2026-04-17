import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import type {
	BatchActionResponse,
	PaginatedResponse,
	PluginTypeInfo,
	SoftwareItemDetailResponse,
	SoftwareItemHostSummary,
	SoftwareItemResponse
} from '$lib/types';
import { Permission } from '$lib/types';

vi.mock('$lib/api', () => ({
	getSoftwareItems: vi.fn(),
	deleteSoftwareItem: vi.fn(),
	checkSoftwareItemVersions: vi.fn(),
	updateSoftwareItem: vi.fn(),
	listPluginTypes: vi.fn(),
	getSoftwareItem: vi.fn(),
	triggerSoftwareUpdate: vi.fn(),
	batchSoftwareItems: vi.fn(),
	executeBatchChunked: vi.fn(),
	previewSoftwareItemMerge: vi.fn(),
	executeSoftwareItemMerge: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null)
}));

vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn()
}));

vi.mock('$lib/stores/events.svelte', () => ({
	subscribeToEvent: vi.fn(() => () => {})
}));

vi.mock('$lib/surfaces/registry.svelte', () => ({
	getSurfaceReadLoading: vi.fn(() => false),
	getSurfaceReadModel: vi.fn(() => undefined),
	getSurfaceReadRequested: vi.fn(() => false),
	getSurfaceRuntimeStatus: vi.fn(() => ({ active: false })),
	getSurfacesBySlot: vi.fn(() => []),
	loadSurfaceReadModels: vi.fn(async () => {})
}));

import SoftwarePage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';
import * as notifications from '$lib/notifications.svelte';
import { page } from '$app/state';

const adminUser = {
	id: '00000000-0000-0000-0000-000000000001',
	email: 'admin@example.com',
	first_name: 'Admin',
	last_name: 'User',
	permissions: [
		Permission.ViewSoftware,
		Permission.CreateSoftware,
		Permission.UpdateSoftware,
		Permission.DeleteSoftware,
		Permission.TriggerChecks,
		Permission.TriggerUpdates
	]
};

function makeSoftwareItem(id: string, name: string): SoftwareItemResponse {
	return {
		id,
		name,
		plugins: ['generic_shell'],
		featured: false,
		last_checked_at: null,
		host_count: 2,
		installed_version: null,
		installed_display_version: null,
		latest_version: '1.1.0',
		latest_release_metadata: null,
		update_available: true,
		created_at: '2024-01-01T00:00:00Z',
		updated_at: '2024-01-01T00:00:00Z',
		icon_url: null
	};
}

function makeHostSummary(hostId: string, name: string): SoftwareItemHostSummary {
	return {
		id: `row-${hostId}`,
		host_id: hostId,
		hostname: name,
		friendly_name: name,
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
	};
}

function makeDetail(item: SoftwareItemResponse, hosts: SoftwareItemHostSummary[]): SoftwareItemDetailResponse {
	return {
		...item,
		hosts
	};
}

function makeItemsPage(items: SoftwareItemResponse[]): PaginatedResponse<SoftwareItemResponse> {
	return {
		items,
		total: items.length,
		page: 1,
		per_page: 50,
		total_pages: 1
	};
}

const emptyBatchResponse: BatchActionResponse = { succeeded: [], failed: [] };

describe('Software Page Trigger Status Handling', () => {
	beforeEach(() => {
		page.url = new URL('http://localhost/software');
		vi.mocked(auth.getUser).mockReturnValue(adminUser);
		vi.mocked(api.listPluginTypes).mockResolvedValue([] as PluginTypeInfo[]);
		vi.mocked(api.executeBatchChunked).mockResolvedValue(emptyBatchResponse);
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('treats fulfilled status=failed responses as failures in single-item update modal flow', async () => {
		const item = makeSoftwareItem('software-1', 'Demo App');
		const hosts = [makeHostSummary('host-1', 'host-one'), makeHostSummary('host-2', 'host-two')];

		vi.mocked(api.getSoftwareItems).mockResolvedValue(makeItemsPage([item]));
		vi.mocked(api.getSoftwareItem).mockResolvedValue(makeDetail(item, hosts));
		vi.mocked(api.triggerSoftwareUpdate).mockImplementation(async (_itemId, hostId) => {
			if (hostId === 'host-2') {
				return { update_history_id: 'uh-host-2', status: 'failed' };
			}
			return { update_history_id: 'uh-host-1', status: 'pending' };
		});

		render(SoftwarePage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Software' })).toBeInTheDocument());

		await fireEvent.click(screen.getByRole('button', { name: 'Update Available' }));
		await waitFor(() => expect(screen.getByRole('button', { name: 'Update 2 host(s)' })).toBeInTheDocument());
		await fireEvent.click(screen.getByRole('button', { name: 'Update 2 host(s)' }));

		await waitFor(() => expect(api.triggerSoftwareUpdate).toHaveBeenCalledTimes(2));
		expect(notifications.showSuccess).toHaveBeenCalledWith('Update triggered for 1 host(s).');
		expect(notifications.showError).toHaveBeenCalledWith('Failed to trigger update for 1 host(s).');
	});

	it('treats fulfilled status=failed responses as failures in batch update-all flow', async () => {
		const itemOne = makeSoftwareItem('software-1', 'Demo App One');
		const itemTwo = makeSoftwareItem('software-2', 'Demo App Two');

		vi.mocked(api.getSoftwareItems).mockResolvedValue(makeItemsPage([itemOne, itemTwo]));
		vi.mocked(api.getSoftwareItem).mockImplementation(async (itemId: string) => {
			if (itemId === 'software-2') {
				return makeDetail(itemTwo, [makeHostSummary('host-2', 'host-two')]);
			}
			return makeDetail(itemOne, [makeHostSummary('host-1', 'host-one')]);
		});
		vi.mocked(api.triggerSoftwareUpdate).mockImplementation(async (itemId: string) => {
			if (itemId === 'software-2') {
				return { update_history_id: 'uh-host-2', status: 'failed' };
			}
			return { update_history_id: 'uh-host-1', status: 'pending' };
		});

		render(SoftwarePage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Software' })).toBeInTheDocument());

		await fireEvent.click(screen.getByRole('checkbox', { name: 'Select Demo App One' }));
		await fireEvent.click(screen.getByRole('checkbox', { name: 'Select Demo App Two' }));
		await fireEvent.click(screen.getByRole('button', { name: 'Update All' }));

		const updateAllButtons = screen.getAllByRole('button', { name: 'Update All' });
		await fireEvent.click(updateAllButtons[updateAllButtons.length - 1]);

		await waitFor(() => expect(api.triggerSoftwareUpdate).toHaveBeenCalledTimes(2));
		expect(notifications.showSuccess).toHaveBeenCalledWith('Update triggered for 1 host(s) across 2 item(s).');
		expect(notifications.showError).toHaveBeenCalledWith('Failed to trigger update for 1 host(s).');
	});
});
