import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import type { SoftwareItemDetailResponse, SoftwareItemHostSummary } from '$lib/types';
import { Permission } from '$lib/types';

vi.mock('$lib/api', () => ({
	getSoftwareItem: vi.fn(),
	getSoftwareItems: vi.fn(),
	checkSoftwareItemVersions: vi.fn(),
	checkSoftwareItemVersionsHost: vi.fn(),
	triggerSoftwareUpdate: vi.fn(),
	updateSoftwareItem: vi.fn(),
	deleteSoftwareItem: vi.fn(),
	unassignHostFromSoftwareItem: vi.fn(),
	getUpdateHistoryEntry: vi.fn(),
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
	getSurfaceReadModel: vi.fn(() => undefined),
	getSurfaceRuntimeStatus: vi.fn(() => ({ active: false })),
	getSurfacesBySlot: vi.fn(() => []),
	loadSurfaceReadModels: vi.fn(() => Promise.resolve())
}));

vi.mock('$lib/interactive', () => ({
	connectInteractiveSession: vi.fn(() => ({
		disconnect: vi.fn(),
		sendInput: vi.fn(),
		sendSignal: vi.fn()
	}))
}));

vi.mock('$lib/components/TerminalOutput.svelte', async () => {
	const mod = await import('$lib/test-mocks/terminal-output-mock.svelte');
	return { default: mod.default };
});

import SoftwareDetailPage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';
import * as notifications from '$lib/notifications.svelte';
import * as interactive from '$lib/interactive';
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

function makeHost({
	id,
	hostId,
	hostname,
	latestVersion = '1.1.0'
}: {
	id: string;
	hostId: string;
	hostname: string;
	latestVersion?: string;
}): SoftwareItemHostSummary {
	return {
		id,
		host_id: hostId,
		hostname,
		friendly_name: hostname,
		qualifier: null,
		installed_version: '1.0.0',
		installed_version_detected_at: '2024-01-01T00:00:00Z',
		installed_display_version: null,
		latest_version: latestVersion,
		latest_release_metadata: null,
		update_available: true,
		active_update_history_id: null,
		last_updated_at: null,
		linked_at: '2024-01-01T00:00:00Z',
		plugins: []
	};
}

function makeSoftwareItem(hosts: SoftwareItemHostSummary[]): SoftwareItemDetailResponse {
	return {
		id: 'software-1',
		name: 'Demo App',
		plugins: ['generic_shell'],
		featured: false,
		last_checked_at: null,
		host_count: hosts.length,
		installed_version: null,
		installed_display_version: null,
		latest_version: '1.1.0',
		latest_release_metadata: null,
		update_available: true,
		created_at: '2024-01-01T00:00:00Z',
		updated_at: '2024-01-01T00:00:00Z',
		icon_url: null,
		hosts
	};
}

describe('Software Detail Update Triggers', () => {
	beforeEach(() => {
		page.params.id = 'software-1';
		vi.mocked(auth.getUser).mockReturnValue(adminUser);
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('shows error and avoids live modal when single-host trigger returns failed', async () => {
		const host = makeHost({ id: 'row-1', hostId: 'host-1', hostname: 'host-one' });
		vi.mocked(api.getSoftwareItem).mockResolvedValue(makeSoftwareItem([host]));
		vi.mocked(api.triggerSoftwareUpdate).mockResolvedValue({
			update_history_id: 'uh-failed',
			status: 'failed'
		});

		render(SoftwareDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Demo App' })).toBeInTheDocument());

		await fireEvent.click(screen.getByRole('button', { name: 'Update Available' }));
		await waitFor(() => expect(screen.getByText('Confirm Update')).toBeInTheDocument());
		await fireEvent.click(screen.getByRole('button', { name: 'Trigger Update' }));

		await waitFor(() => {
			expect(api.triggerSoftwareUpdate).toHaveBeenCalledWith('software-1', 'host-1', {
				to_version: '1.1.0'
			});
		});
		expect(notifications.showError).toHaveBeenCalled();
		expect(notifications.showSuccess).not.toHaveBeenCalled();
		expect(interactive.connectInteractiveSession).not.toHaveBeenCalled();
		expect(screen.queryByText('Confirm Update')).not.toBeInTheDocument();
		expect(screen.queryByText('Update Output')).not.toBeInTheDocument();
	});

	it('does not count failed trigger responses as successful in update-all flow', async () => {
		const hostOne = makeHost({ id: 'row-1', hostId: 'host-1', hostname: 'host-one' });
		const hostTwo = makeHost({ id: 'row-2', hostId: 'host-2', hostname: 'host-two' });
		vi.mocked(api.getSoftwareItem).mockResolvedValue(makeSoftwareItem([hostOne, hostTwo]));
		vi.mocked(api.triggerSoftwareUpdate).mockImplementation(async (_itemId, hostId) => {
			if (hostId === 'host-2') {
				return { update_history_id: 'uh-host-2', status: 'failed' };
			}
			return { update_history_id: 'uh-host-1', status: 'pending' };
		});

		render(SoftwareDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Demo App' })).toBeInTheDocument());

		await fireEvent.click(screen.getByRole('button', { name: 'Update All' }));
		await waitFor(() => expect(screen.getByRole('button', { name: 'Update 2 host(s)' })).toBeInTheDocument());
		await fireEvent.click(screen.getByRole('button', { name: 'Update 2 host(s)' }));

		await waitFor(() => expect(api.triggerSoftwareUpdate).toHaveBeenCalledTimes(2));
		expect(notifications.showSuccess).toHaveBeenCalledWith('Update triggered for 1 host(s).');
		expect(notifications.showError).toHaveBeenCalledWith('Failed to trigger update for 1 host(s).');
	});
});
