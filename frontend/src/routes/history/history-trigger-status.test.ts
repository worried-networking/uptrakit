import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import type {
	PaginatedResponse,
	SoftwareItemDetailResponse,
	SoftwareItemHostSummary,
	SoftwareItemResponse,
	UpdateHistoryResponse
} from '$lib/types';
import { Permission } from '$lib/types';

vi.mock('$lib/api', () => ({
	listUpdateHistory: vi.fn(),
	triggerSoftwareUpdate: vi.fn(),
	getSoftwareItems: vi.fn(),
	getUpdateHistoryEntry: vi.fn(),
	getSoftwareItem: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null)
}));

vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn()
}));

vi.mock('$lib/interactive', () => ({
	connectInteractiveSession: vi.fn(() => ({
		disconnect: vi.fn(),
		sendInput: vi.fn(),
		sendSignal: vi.fn()
	}))
}));

vi.mock('$lib/sse', () => ({
	connectEventStream: vi.fn(() => () => {})
}));

import HistoryPage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';
import * as notifications from '$lib/notifications.svelte';
import { page } from '$app/state';

const adminUser = {
	id: '00000000-0000-0000-0000-000000000001',
	email: 'admin@example.com',
	first_name: 'Admin',
	last_name: 'User',
	has_pending_email_change: false,
	permissions: [Permission.ViewSoftware, Permission.TriggerUpdates]
};

function makeHistoryPage(items: UpdateHistoryResponse[]): PaginatedResponse<UpdateHistoryResponse> {
	return {
		items,
		total: items.length,
		page: 1,
		per_page: 25,
		total_pages: 1
	};
}

function makeSoftwareItem(): SoftwareItemResponse {
	return {
		id: 'software-1',
		name: 'Demo App',
		plugins: ['generic_shell'],
		featured: false,
		last_checked_at: null,
		host_count: 1,
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

function makeHostSummary(): SoftwareItemHostSummary {
	return {
		id: 'row-1',
		host_id: 'host-1',
		hostname: 'host-one',
		friendly_name: 'Host One',
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

function makeDetail(hosts: SoftwareItemHostSummary[]): SoftwareItemDetailResponse {
	return {
		...makeSoftwareItem(),
		hosts
	};
}

function makeHistoryEntry(overrides: Partial<UpdateHistoryResponse> = {}): UpdateHistoryResponse {
	return {
		id: 'history-1',
		host_id: 'host-1',
		host_name: 'Host One',
		software_item_id: 'software-1',
		software_item_name: 'Demo App',
		from_version: '1.0.0',
		to_version: '1.1.0',
		status: 'completed',
		actor_type: 'user',
		actor_id: adminUser.id,
		actor_name: 'History User',
		started_at: '2024-01-01T00:00:00Z',
		completed_at: '2024-01-01T00:05:00Z',
		output: 'Update finished.',
		created_at: '2024-01-01T00:00:00Z',
		interactive: false,
		output_truncated: false,
		pre_update_protection_status: null,
		pre_update_protection_summary: null,
		recovery_hint: null,
		...overrides
	};
}

describe('History Trigger Update Modal', () => {
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
		page.url.pathname = '/history';
		page.url.search = '';
		vi.mocked(auth.getUser).mockReturnValue(adminUser);
		vi.mocked(api.listUpdateHistory).mockResolvedValue(makeHistoryPage([]));
		vi.mocked(api.getSoftwareItems).mockResolvedValue({
			items: [makeSoftwareItem()],
			total: 1,
			page: 1,
			per_page: 100,
			total_pages: 1
		});
		vi.mocked(api.getSoftwareItem).mockResolvedValue(makeDetail([makeHostSummary()]));
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	afterAll(() => {
		vi.unstubAllGlobals();
	});

	it('treats status=failed trigger response as an error and closes modal', async () => {
		vi.mocked(api.triggerSoftwareUpdate).mockResolvedValue({
			update_history_id: 'history-failed',
			status: 'failed'
		});

		render(HistoryPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Update History' })).toBeInTheDocument());

		await fireEvent.click(screen.getByRole('button', { name: 'Trigger Update' }));
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Trigger Software Update' })).toBeInTheDocument());

		const selects = screen.getAllByRole('combobox');
		await fireEvent.change(selects[0], { target: { value: 'software-1' } });
		await waitFor(() => expect(screen.getAllByRole('combobox')).toHaveLength(2));
		await fireEvent.change(screen.getAllByRole('combobox')[1], { target: { value: 'host-1' } });

		await fireEvent.input(screen.getByPlaceholderText('e.g. 1.2.3'), { target: { value: '1.1.0' } });
		const triggerButtons = screen.getAllByRole('button', { name: 'Trigger Update' });
		await fireEvent.click(triggerButtons[triggerButtons.length - 1]);

		await waitFor(() =>
			expect(api.triggerSoftwareUpdate).toHaveBeenCalledWith('software-1', 'host-1', {
				to_version: '1.1.0',
				release_info: undefined
			})
		);
		expect(notifications.showError).toHaveBeenCalledWith('Update failed before dispatch — history ID: history-failed');
		expect(notifications.showSuccess).not.toHaveBeenCalled();
		expect(screen.queryByRole('heading', { name: 'Trigger Software Update' })).not.toBeInTheDocument();
	});

	it('renders generic additional details when summary and recovery hint are present', async () => {
		vi.mocked(api.listUpdateHistory).mockResolvedValue(
			makeHistoryPage([
				makeHistoryEntry({
					pre_update_protection_summary: 'Pre-update checks blocked this run.',
					recovery_hint: 'Resolve the reported issue, then retry the update.'
				})
			])
		);

		render(HistoryPage);
		await waitFor(() => expect(screen.getByText('Demo App on Host One')).toBeInTheDocument());

		const demoEntry = screen.getByText('Demo App on Host One').closest('article')!;
		const viewLogButton = within(demoEntry).getByRole('button', { name: /view logs/i });
		expect(viewLogButton).toBeInTheDocument();
		await fireEvent.click(viewLogButton);

		const shell = await screen.findByRole('dialog', { name: 'Demo App on Host One' });
		expect(shell).toHaveAttribute('data-ui', 'terminal-shell');
		await fireEvent.click(screen.getByRole('button', { name: /details/i }));
		expect(screen.getByText('Additional details')).toBeInTheDocument();
		expect(screen.getByText('Pre-update checks blocked this run.')).toBeInTheDocument();
		expect(screen.getByText('Resolve the reported issue, then retry the update.')).toBeInTheDocument();
	});

	describe('modal button variants', () => {
		it('Trigger Update header launcher renders primary sm, no loading', async () => {
			render(HistoryPage);
			await waitFor(() => expect(screen.getByRole('heading', { name: 'Update History' })).toBeInTheDocument());

			// There is exactly one "Trigger Update" button visible before modal opens
			const launcherBtn = screen.getByRole('button', { name: 'Trigger Update' });
			// primary variant: has gradient background class
			expect(launcherBtn.className).toContain('bg-[linear-gradient');
			// sm size
			expect(launcherBtn.className).toContain('h-[19px]');
			// no aria-busy
			expect(launcherBtn).not.toHaveAttribute('aria-busy');
		});

		it('modal Cancel renders secondary variant', async () => {
			render(HistoryPage);
			await waitFor(() => expect(screen.getByRole('heading', { name: 'Update History' })).toBeInTheDocument());
			await fireEvent.click(screen.getByRole('button', { name: 'Trigger Update' }));
			await waitFor(() => expect(screen.getByRole('heading', { name: 'Trigger Software Update' })).toBeInTheDocument());

			const cancelBtn = screen.getByRole('button', { name: 'Cancel' });
			// secondary variant: bg-[var(--bg-raised)]
			expect(cancelBtn.className).toContain('bg-[var(--bg-raised)]');
			// md size (default)
			expect(cancelBtn.className).toContain('h-[23px]');
		});

		it('modal Submit renders primary md, static children "Trigger Update"', async () => {
			render(HistoryPage);
			await waitFor(() => expect(screen.getByRole('heading', { name: 'Update History' })).toBeInTheDocument());
			await fireEvent.click(screen.getByRole('button', { name: 'Trigger Update' }));
			await waitFor(() => expect(screen.getByRole('heading', { name: 'Trigger Software Update' })).toBeInTheDocument());

			// Two "Trigger Update" buttons now: launcher (hidden behind modal) + submit
			const allTriggerBtns = screen.getAllByRole('button', { name: 'Trigger Update' });
			const submitBtn = allTriggerBtns[allTriggerBtns.length - 1];
			// primary variant
			expect(submitBtn.className).toContain('bg-[linear-gradient');
			// md size
			expect(submitBtn.className).toContain('h-[23px]');
			// static children — no "Triggering..." text present
			expect(submitBtn.textContent).not.toContain('Triggering');
		});

		it('modal Submit shows spinner via aria-busy when triggering, text stays static', async () => {
			// Stall the trigger call so we can inspect mid-flight state
			let resolveTrigger!: (v: { update_history_id: string; status: string }) => void;
			vi.mocked(api.triggerSoftwareUpdate).mockReturnValue(
				new Promise((res) => {
					resolveTrigger = res;
				})
			);

			render(HistoryPage);
			await waitFor(() => expect(screen.getByRole('heading', { name: 'Update History' })).toBeInTheDocument());
			await fireEvent.click(screen.getByRole('button', { name: 'Trigger Update' }));
			await waitFor(() => expect(screen.getByRole('heading', { name: 'Trigger Software Update' })).toBeInTheDocument());

			const selects = screen.getAllByRole('combobox');
			await fireEvent.change(selects[0], { target: { value: 'software-1' } });
			await waitFor(() => expect(screen.getAllByRole('combobox')).toHaveLength(2));
			await fireEvent.change(screen.getAllByRole('combobox')[1], { target: { value: 'host-1' } });
			await fireEvent.input(screen.getByPlaceholderText('e.g. 1.2.3'), { target: { value: '1.1.0' } });

			const allTriggerBtns = screen.getAllByRole('button', { name: 'Trigger Update' });
			const submitBtn = allTriggerBtns[allTriggerBtns.length - 1];
			await fireEvent.click(submitBtn);

			// Mid-flight: aria-busy=true, children text still "Trigger Update" (no text swap)
			await waitFor(() => {
				expect(submitBtn).toHaveAttribute('aria-busy', 'true');
				expect(submitBtn.textContent).not.toContain('Triggering');
			});

			// Resolve so test cleanup works
			resolveTrigger({ update_history_id: 'h-1', status: 'pending' });
		});
	});
});
