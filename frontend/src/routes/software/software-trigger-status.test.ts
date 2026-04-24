import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
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

const { mockEventSubscriptions, mockSubscribeToEvent } = vi.hoisted(() => ({
	mockEventSubscriptions: new Map<string, () => void>(),
	mockSubscribeToEvent: vi.fn((eventName: string, callback: () => void) => {
		mockEventSubscriptions.set(eventName, callback);
		return () => {
			mockEventSubscriptions.delete(eventName);
		};
	})
}));

vi.mock('$lib/stores/events.svelte', () => ({
	subscribeToEvent: mockSubscribeToEvent
}));

vi.mock('$lib/surfaces/registry.svelte', () => ({
	getSurfaceReadLoading: vi.fn(() => false),
	getSurfaceReadModel: vi.fn(() => undefined),
	getSurfaceReadRequested: vi.fn(() => false),
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

const viewOnlyUser = {
	...adminUser,
	permissions: [Permission.ViewSoftware]
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

function makeHostSummaryWithUpdate(hostId: string, name: string, updateAvailable: boolean): SoftwareItemHostSummary {
	return {
		...makeHostSummary(hostId, name),
		update_available: updateAvailable,
		latest_version: updateAvailable ? '1.1.0' : null
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
		page.url.pathname = '/software';
		page.url.search = '';
		mockEventSubscriptions.clear();
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
		await waitFor(() => expect(screen.getAllByText('Demo App').length).toBeGreaterThan(0));
		await waitFor(() => expect(screen.getAllByText('· 2 updates').length).toBeGreaterThan(0));
		expect(screen.getAllByText('1.0.0').length).toBeGreaterThan(0);
		expect(screen.getAllByText('↑ 1.1.0').length).toBeGreaterThan(0);
		expect(screen.getAllByRole('button', { name: 'Update all' }).length).toBeGreaterThan(0);

		await fireEvent.click(screen.getAllByRole('button', { name: 'Update all' })[0]);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Update 2 host(s)' })).toBeInTheDocument());
		await fireEvent.click(screen.getByRole('button', { name: 'Update 2 host(s)' }));

		await waitFor(() => expect(api.triggerSoftwareUpdate).toHaveBeenCalledTimes(2));
		expect(notifications.showSuccess).toHaveBeenCalledWith('Update triggered for 1 host(s).');
		expect(notifications.showError).toHaveBeenCalledWith('Failed to trigger update for 1 host(s).');
	});

	it('folds all host rows when collapsing a software group and preserves that state through background refresh', async () => {
		const item = {
			...makeSoftwareItem('software-1', 'Demo App'),
			host_count: 4
		};
		const hosts = [
			makeHostSummaryWithUpdate('host-1', 'host-one', true),
			makeHostSummaryWithUpdate('host-2', 'host-two', true),
			makeHostSummaryWithUpdate('host-3', 'host-three', false),
			makeHostSummaryWithUpdate('host-4', 'host-four', false)
		];

		vi.mocked(api.getSoftwareItems).mockResolvedValue(makeItemsPage([item]));
		vi.mocked(api.getSoftwareItem).mockResolvedValue(makeDetail(item, hosts));

		render(SoftwarePage);
		await waitFor(() => expect(screen.getAllByText('Demo App').length).toBeGreaterThan(0));
		// NEW — wait until the trailing update label appears; this resolves only after detail loads
		// because softwareUpdateLabel returns '· loading updates' until itemDetailsById has data.
		await waitFor(() => expect(screen.getAllByText('· 2 updates').length).toBeGreaterThan(0));

		const collapseButton = screen.getAllByRole('button', { name: 'Collapse Demo App' })[0];
		expect(collapseButton).toHaveAttribute('aria-expanded', 'true');
		expect(screen.getAllByText('host-one').length).toBeGreaterThan(0);
		expect(screen.getAllByText('host-two').length).toBeGreaterThan(0);
		expect(screen.getAllByText('▸ 1 more — all up to date').length).toBeGreaterThan(0);

		await fireEvent.click(collapseButton);

		expect(screen.getAllByRole('button', { name: 'Expand Demo App' })[0]).toHaveAttribute('aria-expanded', 'false');
		expect(screen.queryByText('host-one')).not.toBeInTheDocument();
		expect(screen.queryByText('host-two')).not.toBeInTheDocument();
		expect(screen.queryByText('▸ 1 more — all up to date')).not.toBeInTheDocument();
		expect(screen.queryByText('host-four')).not.toBeInTheDocument();

		mockEventSubscriptions.get('version_check_completed')?.();

		await waitFor(() => expect(vi.mocked(api.getSoftwareItems)).toHaveBeenCalledTimes(2));
		expect(screen.getAllByRole('button', { name: 'Expand Demo App' })[0]).toHaveAttribute('aria-expanded', 'false');
		expect(screen.queryByText('host-one')).not.toBeInTheDocument();
		expect(screen.queryByText('host-two')).not.toBeInTheDocument();
		expect(screen.queryByText('▸ 1 more — all up to date')).not.toBeInTheDocument();
		expect(screen.queryByText('host-four')).not.toBeInTheDocument();
	});

	it('expands hidden hosts from the overflow row without changing the group fold state', async () => {
		const item = {
			...makeSoftwareItem('software-1', 'Demo App'),
			host_count: 4
		};
		const hosts = [
			makeHostSummaryWithUpdate('host-1', 'host-one', true),
			makeHostSummaryWithUpdate('host-2', 'host-two', true),
			makeHostSummaryWithUpdate('host-3', 'host-three', false),
			makeHostSummaryWithUpdate('host-4', 'host-four', false)
		];

		vi.mocked(api.getSoftwareItems).mockResolvedValue(makeItemsPage([item]));
		vi.mocked(api.getSoftwareItem).mockResolvedValue(makeDetail(item, hosts));

		render(SoftwarePage);
		await waitFor(() => expect(screen.getAllByText('Demo App').length).toBeGreaterThan(0));

		expect(screen.getAllByRole('button', { name: 'Collapse Demo App' })[0]).toHaveAttribute('aria-expanded', 'true');
		expect(screen.getAllByText('host-one').length).toBeGreaterThan(0);
		expect(screen.getAllByText('host-two').length).toBeGreaterThan(0);
		expect(screen.getAllByText('host-three').length).toBeGreaterThan(0);
		expect(screen.getAllByText('▸ 1 more — all up to date').length).toBeGreaterThan(0);
		expect(screen.queryByText('host-four')).not.toBeInTheDocument();

		await fireEvent.click(screen.getAllByRole('button', { name: '▸ 1 more — all up to date' })[0]);

		expect(screen.getAllByRole('button', { name: 'Collapse Demo App' })[0]).toHaveAttribute('aria-expanded', 'true');
		expect(screen.getAllByText('host-four').length).toBeGreaterThan(0);
		expect(screen.queryByText('▸ 1 more — all up to date')).not.toBeInTheDocument();
	});

	it('keeps the group header version column empty and renders the version stack on host rows for multi-host items', async () => {
		const item = makeSoftwareItem('software-1', 'Demo App');
		const hosts = [makeHostSummary('host-1', 'host-one'), makeHostSummary('host-2', 'host-two')];

		vi.mocked(api.getSoftwareItems).mockResolvedValue(makeItemsPage([item]));
		vi.mocked(api.getSoftwareItem).mockResolvedValue(makeDetail(item, hosts));

		render(SoftwarePage);
		await waitFor(() => expect(screen.getAllByText('Demo App').length).toBeGreaterThan(0));
		await waitFor(() => expect(screen.getAllByTestId('software-group-header-software-1').length).toBeGreaterThan(0));
		await waitFor(() => expect(screen.getAllByTestId('software-host-row-row-host-1').length).toBeGreaterThan(0));

		const group = screen.getAllByTestId('software-group-software-1')[0];
		const headerRow = within(group).getByTestId('software-group-header-software-1');
		const hostRow = within(group).getByTestId('software-host-row-row-host-1');

		expect(within(headerRow).queryByText('1.0.0')).not.toBeInTheDocument();
		expect(within(headerRow).queryByText('↑ 1.1.0')).not.toBeInTheDocument();
		expect(within(hostRow).getByText('1.0.0')).toBeInTheDocument();
		expect(within(hostRow).getByText('↑ 1.1.0')).toBeInTheDocument();
	});

	it('flattens single-host items into a compact row with a singular update action', async () => {
		const item = makeSoftwareItem('software-1', 'Demo App');
		const hosts = [makeHostSummary('host-1', 'host-one')];

		vi.mocked(api.getSoftwareItems).mockResolvedValue(makeItemsPage([item]));
		vi.mocked(api.getSoftwareItem).mockResolvedValue(makeDetail(item, hosts));

		render(SoftwarePage);
		await waitFor(() => expect(screen.getAllByText('Demo App').length).toBeGreaterThan(0));
		await waitFor(() => expect(screen.getAllByTestId('software-group-header-software-1').length).toBeGreaterThan(0));

		const group = screen.getAllByTestId('software-group-software-1')[0];
		const headerRow = within(group).getByTestId('software-group-header-software-1');

		expect(screen.queryByRole('button', { name: 'Collapse Demo App' })).not.toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Expand Demo App' })).not.toBeInTheDocument();
		expect(screen.queryByTestId('software-host-row-row-host-1')).not.toBeInTheDocument();
		expect(within(headerRow).getByText('host-one')).toBeInTheDocument();
		expect(within(headerRow).getByText('1.0.0')).toBeInTheDocument();
		expect(within(headerRow).getByText('↑ 1.1.0')).toBeInTheDocument();
		expect(within(headerRow).getByRole('button', { name: 'Update' })).toBeInTheDocument();
		expect(within(headerRow).queryByRole('button', { name: '↑ Update all' })).not.toBeInTheDocument();
		expect(within(headerRow).queryByText('1 host · 1 update')).not.toBeInTheDocument();
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
		await waitFor(() =>
			expect(screen.getAllByRole('checkbox', { name: 'Select Demo App One' }).length).toBeGreaterThan(0)
		);

		await fireEvent.click(screen.getAllByRole('checkbox', { name: 'Select Demo App One' })[0]);
		await fireEvent.click(screen.getAllByRole('checkbox', { name: 'Select Demo App Two' })[0]);
		await fireEvent.click(
			within(screen.getByRole('toolbar', { name: 'Batch actions' })).getByRole('button', { name: 'Update all' })
		);

		await fireEvent.click(screen.getByRole('button', { name: 'Update All' }));

		await waitFor(() => expect(api.triggerSoftwareUpdate).toHaveBeenCalledTimes(2));
		expect(notifications.showSuccess).toHaveBeenCalledWith('Update triggered for 1 host(s) across 2 item(s).');
		expect(notifications.showError).toHaveBeenCalledWith('Failed to trigger update for 1 host(s).');
	});

	it('hides software update actions from view-only users', async () => {
		const item = makeSoftwareItem('software-1', 'Demo App');
		const hosts = [makeHostSummary('host-1', 'host-one')];

		vi.mocked(auth.getUser).mockReturnValue(viewOnlyUser);
		vi.mocked(api.getSoftwareItems).mockResolvedValue(makeItemsPage([item]));
		vi.mocked(api.getSoftwareItem).mockResolvedValue(makeDetail(item, hosts));

		render(SoftwarePage);
		await waitFor(() => expect(screen.getAllByText('Demo App').length).toBeGreaterThan(0));

		expect(screen.queryByRole('button', { name: '↑ Update all' })).not.toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Update all' })).not.toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Actions for Demo App' })).not.toBeInTheDocument();
	});

	it('"Add Software" button renders with primary variant and sm size', async () => {
		const item = makeSoftwareItem('software-1', 'Demo App');
		vi.mocked(api.getSoftwareItems).mockResolvedValue(makeItemsPage([item]));
		vi.mocked(api.getSoftwareItem).mockResolvedValue(makeDetail(item, []));

		render(SoftwarePage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Software' })).toBeInTheDocument());

		const addBtn = screen.getByRole('button', { name: 'Add Software' });
		expect(addBtn.className).toContain('h-[19px]'); // size="sm"
		expect(addBtn.className).toContain('bg-[linear-gradient'); // variant="primary"
	});

	it('row context-menu toggle renders ghost sm button', async () => {
		const item = makeSoftwareItem('software-1', 'Demo App');
		const hosts = [makeHostSummary('host-1', 'host-one'), makeHostSummary('host-2', 'host-two')];
		vi.mocked(api.getSoftwareItems).mockResolvedValue(makeItemsPage([item]));
		vi.mocked(api.getSoftwareItem).mockResolvedValue(makeDetail(item, hosts));

		render(SoftwarePage);
		await waitFor(() => expect(screen.getAllByText('Demo App').length).toBeGreaterThan(0));

		const actionsBtn = screen.getAllByRole('button', { name: 'Actions for Demo App' })[0];
		expect(actionsBtn.className).toContain('h-[19px]');
		expect(actionsBtn.className).toContain('bg-transparent'); // ghost
	});

	it('header row aggregate trigger renders UpdateAllButton, not a raw Button', async () => {
		const item = makeSoftwareItem('software-1', 'Demo App');
		const hosts = [makeHostSummary('host-1', 'host-one'), makeHostSummary('host-2', 'host-two')];
		vi.mocked(api.getSoftwareItems).mockResolvedValue(makeItemsPage([item]));
		vi.mocked(api.getSoftwareItem).mockResolvedValue(makeDetail(item, hosts));

		render(SoftwarePage);
		await waitFor(() => expect(screen.getAllByText('Demo App').length).toBeGreaterThan(0));
		await waitFor(() => expect(screen.getAllByRole('button', { name: /update all/i }).length).toBeGreaterThan(0));

		const updateAllBtn = screen.getAllByRole('button', { name: /update all/i })[0];
		expect(updateAllBtn).not.toHaveAttribute('aria-busy');
	});
});
