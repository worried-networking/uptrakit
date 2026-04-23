import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import type { HostResponse, PaginatedResponse } from '$lib/types';
import { Permission } from '$lib/types';

vi.mock('$lib/api', () => ({
	getHosts: vi.fn(),
	updateHost: vi.fn(),
	deactivateHost: vi.fn(),
	triggerHostDiscovery: vi.fn(),
	batchHosts: vi.fn(),
	executeBatchChunked: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null),
	getAccessToken: vi.fn(() => null)
}));

vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn()
}));

vi.mock('$lib/stores/events.svelte', () => ({
	subscribeToEvent: vi.fn(() => () => {})
}));

import HostsPage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const adminUser = {
	id: '00000000-0000-0000-0000-000000000002',
	email: 'admin@example.com',
	first_name: 'Admin',
	last_name: 'User',
	permissions: [
		Permission.UpdateHosts,
		Permission.DeactivateHosts,
		Permission.ViewSoftware,
		Permission.CreateSoftware,
		Permission.UpdateSoftware,
		Permission.DeleteSoftware,
		Permission.TriggerChecks,
		Permission.TriggerUpdates
	]
};

function makePage(items: HostResponse[]): PaginatedResponse<HostResponse> {
	return { items, total: items.length, page: 1, per_page: 25, total_pages: 1 };
}

const sampleHost: HostResponse = {
	id: 'host-001',
	machine_id: 'machine-abc',
	hostname: 'prod-server',
	friendly_name: 'Production Server',
	os_type: 'Linux',
	os_version: 'Ubuntu 24.04',
	architecture: 'x86_64',
	ip_address: '10.0.0.5',
	last_seen_at: '2024-06-01T12:00:00Z',
	created_at: '2024-01-01T00:00:00Z',
	updated_at: '2024-01-01T00:00:00Z',
	agents: [],
	tags: [],
	software_status: {
		known: true,
		update_count: 2,
		error_count: 0
	}
} as unknown as HostResponse;

const errorHost: HostResponse = {
	...sampleHost,
	id: 'host-002',
	friendly_name: 'Backup Server',
	hostname: 'backup-server',
	software_status: {
		known: true,
		update_count: 0,
		error_count: 1
	}
} as unknown as HostResponse;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('Hosts Page', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(adminUser);
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('renders the page heading when a user is logged in', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([]));
		render(HostsPage);
		await waitFor(() => expect(screen.getByText('Hosts')).toBeInTheDocument());
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
	});

	it('renders a host row after a successful API response', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
		render(HostsPage);
		await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());
		expect(screen.getByText('prod-server')).toBeInTheDocument();
		expect(screen.getByText('Ubuntu 24.04')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
	});

	it('renders navigable software status badges for updates and errors', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost, errorHost]));
		render(HostsPage);
		await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());

		const updatesBadge = screen.getByRole('button', { name: '2 updates' });
		expect(updatesBadge).toHaveAttribute('data-ui', 'action-badge');
		expect(updatesBadge).toHaveAttribute('data-tone', 'info');

		const historyBadge = screen.getByRole('button', { name: '1 error' });
		expect(historyBadge).toHaveAttribute('data-ui', 'action-badge');
		expect(historyBadge).toHaveAttribute('data-tone', 'danger');
	});

	it('renders hosts stat cards with semantic value colors', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost, errorHost]));
		render(HostsPage);
		await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());

		expect(screen.getByText('Online')).toBeInTheDocument();
		expect(screen.getByText('Offline')).toBeInTheDocument();
		expect(screen.getByText('Updates pending')).toBeInTheDocument();
		expect(screen.getByText('Errors')).toBeInTheDocument();

		const onlineCard = screen.getByTestId('host-stat-online');
		const offlineCard = screen.getByTestId('host-stat-offline');
		const updatesCard = screen.getByTestId('host-stat-updates');
		const errorsCard = screen.getByTestId('host-stat-errors');

		expect(within(onlineCard).getByText('2')).toHaveClass('text-[var(--color-success)]');
		expect(within(offlineCard).getByText('0')).toHaveClass('text-[var(--text-muted)]');
		expect(within(updatesCard).getByText('1')).toHaveClass('text-[var(--color-info)]');
		expect(within(errorsCard).getByText('1')).toHaveClass('text-[var(--color-danger)]');
	});

	it('shows the empty-state message when the host list is empty', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([]));
		render(HostsPage);
		await waitFor(() => expect(screen.getByText(/No hosts discovered yet/)).toBeInTheDocument());
	});

	it('shows an error message and a Retry button when the API call fails', async () => {
		vi.mocked(api.getHosts).mockRejectedValue(new Error('Server unavailable'));
		render(HostsPage);
		await waitFor(() => expect(screen.getByText('Server unavailable')).toBeInTheDocument());
		expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument();
		expect(document.querySelector('[data-ui="callout"]')).toBeInTheDocument();
	});

	it('renders nothing when no user is logged in', () => {
		vi.mocked(auth.getUser).mockReturnValue(null);
		render(HostsPage);
		expect(screen.queryByText('Hosts')).not.toBeInTheDocument();
	});

	it('displays a dash for unknown OS when os_type and os_version are both null', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([{ ...sampleHost, os_type: null, os_version: null }]));
		render(HostsPage);
		// The "—" dash for OS column should appear
		await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());
		// At least one em-dash should be present (could be OS, arch, or IP)
		const dashes = screen.getAllByText('—');
		expect(dashes.length).toBeGreaterThan(0);
	});

	it('shows the actions button when the user has ManageHosts permission', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
		render(HostsPage);
		await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());
		expect(screen.getByRole('button', { name: /actions for production server/i })).toBeInTheDocument();
	});

	it('does not show the actions button when the user lacks host management permissions', async () => {
		vi.mocked(auth.getUser).mockReturnValue({
			...adminUser,
			permissions: [Permission.TriggerChecks]
		});
		vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
		render(HostsPage);
		await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());
		expect(screen.queryByRole('button', { name: /actions for/i })).not.toBeInTheDocument();
	});

	it('opens the context menu with Edit Name and Deactivate options when the actions button is clicked', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
		render(HostsPage);
		await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());

		const actionsBtn = screen.getByRole('button', { name: /actions for production server/i });
		fireEvent.click(actionsBtn);

		await waitFor(() => expect(screen.getByRole('menu')).toBeInTheDocument());
		expect(screen.getByRole('menuitem', { name: 'Edit Name' })).toBeInTheDocument();
		expect(screen.getByRole('menuitem', { name: 'Deactivate' })).toBeInTheDocument();
	});

	it('shows the Trigger Discovery menu item when the user has software management permissions', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
		render(HostsPage);
		await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());

		fireEvent.click(screen.getByRole('button', { name: /actions for production server/i }));

		await waitFor(() => expect(screen.getByRole('menu')).toBeInTheDocument());
		expect(screen.getByRole('menuitem', { name: 'Trigger Discovery' })).toBeInTheDocument();
	});

	it('does not show Trigger Discovery when the user lacks software management permissions', async () => {
		vi.mocked(auth.getUser).mockReturnValue({
			...adminUser,
			permissions: [Permission.UpdateHosts]
		});
		vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
		render(HostsPage);
		await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());

		fireEvent.click(screen.getByRole('button', { name: /actions for production server/i }));

		await waitFor(() => expect(screen.getByRole('menu')).toBeInTheDocument());
		expect(screen.queryByRole('menuitem', { name: 'Trigger Discovery' })).not.toBeInTheDocument();
	});

	it('calls triggerHostDiscovery and shows a success notification when plugins are queued', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
		vi.mocked(api.triggerHostDiscovery).mockResolvedValue({ plugins_queued: 2, message: 'ok' });
		const { showSuccess } = await import('$lib/notifications.svelte');

		render(HostsPage);
		await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());

		fireEvent.click(screen.getByRole('button', { name: /actions for production server/i }));
		await waitFor(() => expect(screen.getByRole('menu')).toBeInTheDocument());

		fireEvent.click(screen.getByRole('menuitem', { name: 'Trigger Discovery' }));

		await waitFor(() => expect(vi.mocked(api.triggerHostDiscovery)).toHaveBeenCalledWith('host-001'));
		await waitFor(() =>
			expect(vi.mocked(showSuccess)).toHaveBeenCalledWith(expect.stringContaining('2 plugin(s) queued'))
		);
	});

	it('calls triggerHostDiscovery and shows a no-plugins notification when nothing is queued', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
		vi.mocked(api.triggerHostDiscovery).mockResolvedValue({ plugins_queued: 0, message: 'no plugins' });
		const { showSuccess } = await import('$lib/notifications.svelte');

		render(HostsPage);
		await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());

		fireEvent.click(screen.getByRole('button', { name: /actions for production server/i }));
		await waitFor(() => expect(screen.getByRole('menu')).toBeInTheDocument());

		fireEvent.click(screen.getByRole('menuitem', { name: 'Trigger Discovery' }));

		await waitFor(() =>
			expect(vi.mocked(showSuccess)).toHaveBeenCalledWith(expect.stringContaining('no plugins queued'))
		);
	});
});

describe('Button primitive contract — hosts/+page.svelte', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(adminUser as ReturnType<typeof auth.getUser>);
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('ellipsis trigger has no preset Skeleton class', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
		render(HostsPage);
		await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());

		const ellipsis = screen.getByRole('button', { name: /actions for production server/i });
		expect(ellipsis.className).not.toMatch(/preset-/);
	});

	it('Retry button is not aria-busy when idle', async () => {
		vi.mocked(api.getHosts).mockRejectedValue(new Error('network error'));
		render(HostsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument());

		const retry = screen.getByRole('button', { name: /retry/i });
		expect(retry).not.toHaveAttribute('aria-busy');
	});

	it('Edit modal Save button has static label "Save"', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
		render(HostsPage);
		await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());

		fireEvent.click(screen.getByRole('button', { name: /actions for production server/i }));
		await waitFor(() => expect(screen.getByRole('menu')).toBeInTheDocument());
		fireEvent.click(screen.getByRole('menuitem', { name: 'Edit Name' }));
		await waitFor(() => expect(screen.getByText('Edit Host Name')).toBeInTheDocument());

		expect(screen.getByRole('button', { name: /^save$/i })).toBeInTheDocument();
	});

	it('Edit modal Cancel button has no preset Skeleton class', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
		render(HostsPage);
		await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());

		fireEvent.click(screen.getByRole('button', { name: /actions for production server/i }));
		await waitFor(() => expect(screen.getByRole('menu')).toBeInTheDocument());
		fireEvent.click(screen.getByRole('menuitem', { name: 'Edit Name' }));
		await waitFor(() => expect(screen.getByText('Edit Host Name')).toBeInTheDocument());

		const cancel = screen.getByRole('button', { name: /^cancel$/i });
		expect(cancel.className).not.toMatch(/preset-/);
	});

	it('no raw preset-filled or preset-tonal classes on any button', async () => {
		vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
		const { container } = render(HostsPage);
		await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());

		const buttons = container.querySelectorAll('button');
		buttons.forEach((btn) => {
			expect(btn.className).not.toMatch(/preset-filled|preset-tonal/);
		});
	});
});
