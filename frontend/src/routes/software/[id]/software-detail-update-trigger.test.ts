import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import type { SoftwareItemDetailResponse, SoftwareItemHostSummary } from '$lib/types';
import { Permission } from '$lib/types';

const interactiveMocks = vi.hoisted(() => ({
	disconnect: vi.fn(),
	sendInput: vi.fn(),
	sendSignal: vi.fn()
}));

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
	getSurfacesBySlot: vi.fn(() => []),
	loadSurfaceReadModels: vi.fn(() => Promise.resolve())
}));

vi.mock('$lib/interactive', () => ({
	connectInteractiveSession: vi.fn(() => interactiveMocks)
}));

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

const viewOnlyUser = {
	...adminUser,
	permissions: [Permission.ViewSoftware]
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
	beforeAll(() => {
		class ResizeObserverMock {
			observe = vi.fn();
			disconnect = vi.fn();
		}
		vi.stubGlobal('ResizeObserver', ResizeObserverMock);
		vi.stubGlobal(
			'matchMedia',
			vi.fn(() => ({
				matches: false,
				media: '',
				onchange: null,
				addEventListener: vi.fn(),
				removeEventListener: vi.fn(),
				addListener: vi.fn(),
				removeListener: vi.fn(),
				dispatchEvent: vi.fn(() => false)
			}))
		);
	});

	beforeEach(() => {
		page.params.id = 'software-1';
		vi.mocked(auth.getUser).mockReturnValue(adminUser);
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	afterAll(() => {
		vi.unstubAllGlobals();
	});

	it('shows error and avoids live modal when single-host trigger returns failed', async () => {
		const host = makeHost({ id: 'row-1', hostId: 'host-1', hostname: 'host-one' });
		vi.mocked(api.getSoftwareItem).mockResolvedValue(makeSoftwareItem([host]));
		vi.mocked(api.triggerSoftwareUpdate).mockResolvedValue({
			update_history_id: 'uh-failed',
			status: 'failed'
		});

		render(SoftwareDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Demo App' })).toBeInTheDocument());

		const updateBadge = screen.getByRole('button', { name: 'Update Avail' });
		expect(updateBadge).toHaveAttribute('data-ui', 'action-badge');
		expect(updateBadge).toHaveAttribute('data-variant', 'navigation');
		await fireEvent.click(updateBadge);
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

	it('launches terminal output without route-local terminal shell chrome on successful trigger', async () => {
		const host = makeHost({ id: 'row-1', hostId: 'host-1', hostname: 'host-one' });
		vi.mocked(api.getSoftwareItem).mockResolvedValue(makeSoftwareItem([host]));
		vi.mocked(api.triggerSoftwareUpdate).mockResolvedValue({
			update_history_id: 'uh-live',
			status: 'pending'
		});

		render(SoftwareDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Demo App' })).toBeInTheDocument());

		await fireEvent.click(screen.getByRole('button', { name: 'Update Avail' }));
		await waitFor(() => expect(screen.getByText('Confirm Update')).toBeInTheDocument());
		await fireEvent.click(screen.getByRole('button', { name: 'Trigger Update' }));

		await waitFor(() =>
			expect(api.triggerSoftwareUpdate).toHaveBeenCalledWith('software-1', 'host-1', {
				to_version: '1.1.0'
			})
		);
		const shell = await screen.findByRole('dialog', { name: 'Demo App on host-one' });
		expect(shell).toHaveAttribute('data-ui', 'terminal-shell');
		expect(screen.queryByText('Update Output')).not.toBeInTheDocument();
		await waitFor(() =>
			expect(interactive.connectInteractiveSession).toHaveBeenCalledWith('uh-live', expect.any(Object))
		);
		const sigintButton = await screen.findByRole('button', { name: 'Ctrl+C' });
		expect(sigintButton.closest('[data-ui="terminal-shell"]')).toBe(shell);
		await fireEvent.click(sigintButton);
		expect(interactiveMocks.sendSignal).toHaveBeenCalledWith(2);
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
		await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Demo App' })).toBeInTheDocument());

		await fireEvent.click(screen.getByRole('button', { name: 'Update All' }));
		await waitFor(() => expect(screen.getByRole('button', { name: 'Update 2 host(s)' })).toBeInTheDocument());
		await fireEvent.click(screen.getByRole('button', { name: 'Update 2 host(s)' }));

		await waitFor(() => expect(api.triggerSoftwareUpdate).toHaveBeenCalledTimes(2));
		expect(notifications.showSuccess).toHaveBeenCalledWith('Update triggered for 1 host(s).');
		expect(notifications.showError).toHaveBeenCalledWith('Failed to trigger update for 1 host(s).');
	});

	it('hides update triggers for users without trigger_updates permission', async () => {
		const host = makeHost({ id: 'row-1', hostId: 'host-1', hostname: 'host-one' });
		vi.mocked(auth.getUser).mockReturnValue(viewOnlyUser);
		vi.mocked(api.getSoftwareItem).mockResolvedValue(makeSoftwareItem([host]));

		render(SoftwareDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Demo App' })).toBeInTheDocument());

		expect(screen.queryByRole('button', { name: 'Update All' })).not.toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Update Avail' })).not.toBeInTheDocument();
	});

	it('Confirm Update modal Trigger Update renders primary loading during submit', async () => {
		const host = makeHost({ id: 'row-1', hostId: 'host-1', hostname: 'host-one' });
		vi.mocked(api.getSoftwareItem).mockResolvedValue(makeSoftwareItem([host]));
		vi.mocked(api.triggerSoftwareUpdate).mockReturnValue(new Promise(() => {}));

		render(SoftwareDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Demo App' })).toBeInTheDocument());

		await fireEvent.click(screen.getByRole('button', { name: 'Update Avail' }));
		await waitFor(() => expect(screen.getByText('Confirm Update')).toBeInTheDocument());

		const triggerBtn = screen.getByRole('button', { name: 'Trigger Update' });
		expect(triggerBtn).not.toHaveAttribute('aria-busy');
		await fireEvent.click(triggerBtn);
		await waitFor(() => expect(triggerBtn).toHaveAttribute('aria-busy', 'true'));
	});

	it('Delete header button renders danger variant', async () => {
		const host = makeHost({ id: 'row-1', hostId: 'host-1', hostname: 'host-one' });
		vi.mocked(api.getSoftwareItem).mockResolvedValue(makeSoftwareItem([host]));

		render(SoftwareDetailPage);
		await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Demo App' })).toBeInTheDocument());

		const deleteBtn = screen.getByRole('button', { name: 'Delete' });
		expect(deleteBtn.className).toContain('var(--color-error-bg)');
	});
});
