import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { Permission, type PaginatedResponse, type SystemServiceResponse } from '$lib/types';

vi.mock('$lib/api', () => ({
	getSystemServices: vi.fn(),
	approveSystemService: vi.fn(),
	rejectSystemService: vi.fn(),
	deleteSystemService: vi.fn(),
	updateSystemService: vi.fn(),
	batchSystemServices: vi.fn(),
	executeBatchChunked: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null)
}));

vi.mock('$lib/stores/events.svelte', () => ({
	subscribeToEvent: vi.fn(() => () => {})
}));

vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn()
}));

import SystemServicesPage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';

const user = {
	id: '00000000-0000-0000-0000-000000000103',
	email: 'system-services@example.com',
	first_name: 'System',
	last_name: 'User',
	permissions: [
		Permission.ViewSystemServices,
		Permission.ApproveSystemServices,
		Permission.RejectSystemServices,
		Permission.RemoveSystemServices,
		Permission.UpdateSystemServices
	]
};

function makePage(items: SystemServiceResponse[]): PaginatedResponse<SystemServiceResponse> {
	return { items, total: items.length, page: 1, per_page: 25, total_pages: 1 };
}

describe('System Services Route', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.getSystemServices).mockResolvedValue(
			makePage([
				{
					id: 'sys-1',
					friendly_name: 'scheduler-service',
					hostname: 'controller-a',
					ip_address: '10.10.1.5',
					status: 'pending',
					is_embedded: false,
					yielded_to: [],
					last_seen_at: '2026-02-01T10:00:00Z',
					capabilities: []
				} as unknown as SystemServiceResponse
			])
		);
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('renders shared shell primitives and status badges for rows', async () => {
		render(SystemServicesPage);

		await waitFor(() => expect(screen.getByText('System Services')).toBeInTheDocument());
		expect(screen.getByText('scheduler-service')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="status-badge"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="table-footer-bar"]')).toBeInTheDocument();
		expect(screen.getByText('1 total')).toBeInTheDocument();
	});

	it('stacks multiple status badges with shared spacing', async () => {
		vi.mocked(api.getSystemServices).mockResolvedValue(
			makePage([
				{
					id: 'sys-embedded',
					friendly_name: 'embedded-scheduler',
					hostname: 'controller-a',
					ip_address: '10.10.1.5',
					status: 'approved',
					is_embedded: true,
					yielded_to: ['svc-other'],
					last_seen_at: '2026-02-01T10:00:00Z',
					capabilities: []
				} as unknown as SystemServiceResponse
			])
		);

		render(SystemServicesPage);

		await waitFor(() => expect(screen.getByText('embedded-scheduler')).toBeInTheDocument());
		const badgeStack = document.querySelector('[data-ui="status-badge-stack"]');
		expect(badgeStack).toBeInTheDocument();
		expect(within(badgeStack as HTMLElement).getByText('Approved')).toBeInTheDocument();
		expect(within(badgeStack as HTMLElement).getByText('Embedded')).toBeInTheDocument();
		expect(within(badgeStack as HTMLElement).getByText('Yielded (1)')).toBeInTheDocument();
	});

	describe('status filter chips', () => {
		it('All chip is active by default — carries accent/bg-hover fragments', async () => {
			render(SystemServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: 'All' })).toBeInTheDocument());
			const allChip = screen.getByRole('button', { name: 'All' });
			expect(allChip.className).toContain('text-[var(--accent)]');
			expect(allChip.className).toContain('bg-[var(--bg-hover)]');
		});

		it('inactive chips carry no accent/bg-hover fragments', async () => {
			render(SystemServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: 'Pending' })).toBeInTheDocument());
			for (const label of ['Pending', 'Approved', 'Rejected', 'Deactivated']) {
				const chip = screen.getByRole('button', { name: label });
				expect(chip.className).not.toContain('text-[var(--accent)]');
				expect(chip.className).not.toContain('bg-[var(--bg-hover)]');
			}
		});

		it('clicking Pending chip makes it active and deactivates All', async () => {
			render(SystemServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: 'Pending' })).toBeInTheDocument());
			fireEvent.click(screen.getByRole('button', { name: 'Pending' }));
			await waitFor(() => {
				const pendingChip = screen.getByRole('button', { name: 'Pending' });
				expect(pendingChip.className).toContain('text-[var(--accent)]');
				expect(pendingChip.className).toContain('bg-[var(--bg-hover)]');
			});
			expect(screen.getByRole('button', { name: 'All' }).className).not.toContain('text-[var(--accent)]');
		});

		it('clicking Approved chip makes it active', async () => {
			render(SystemServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: 'Approved' })).toBeInTheDocument());
			fireEvent.click(screen.getByRole('button', { name: 'Approved' }));
			await waitFor(() => {
				const chip = screen.getByRole('button', { name: 'Approved' });
				expect(chip.className).toContain('text-[var(--accent)]');
			});
		});

		it('clicking Rejected chip makes it active', async () => {
			render(SystemServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: 'Rejected' })).toBeInTheDocument());
			fireEvent.click(screen.getByRole('button', { name: 'Rejected' }));
			await waitFor(() => {
				const chip = screen.getByRole('button', { name: 'Rejected' });
				expect(chip.className).toContain('text-[var(--accent)]');
			});
		});

		it('clicking Deactivated chip makes it active', async () => {
			render(SystemServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: 'Deactivated' })).toBeInTheDocument());
			fireEvent.click(screen.getByRole('button', { name: 'Deactivated' }));
			await waitFor(() => {
				const chip = screen.getByRole('button', { name: 'Deactivated' });
				expect(chip.className).toContain('text-[var(--accent)]');
			});
		});
	});

	describe('row ellipsis trigger', () => {
		it('renders variant="ghost" size="sm" — bg-transparent and h-[19px]', async () => {
			render(SystemServicesPage);
			await waitFor(() =>
				expect(screen.getByRole('button', { name: /actions for scheduler-service/i })).toBeInTheDocument()
			);
			const trigger = screen.getByRole('button', { name: /actions for scheduler-service/i });
			expect(trigger.className).toContain('bg-transparent');
			expect(trigger.className).toContain('h-[19px]');
		});

		it('aria-label matches "Actions for {friendly_name}" including space in name', async () => {
			vi.mocked(api.getSystemServices).mockResolvedValue(
				makePage([
					{
						id: 'sys-space',
						friendly_name: 'my system svc',
						hostname: 'host-a',
						ip_address: null,
						status: 'pending',
						is_embedded: false,
						yielded_to: [],
						last_seen_at: '2026-02-01T10:00:00Z',
						capabilities: []
					} as unknown as SystemServiceResponse
				])
			);
			render(SystemServicesPage);
			await waitFor(() =>
				expect(screen.getByRole('button', { name: 'Actions for my system svc' })).toBeInTheDocument()
			);
		});

		it('clicking the trigger opens the context menu', async () => {
			render(SystemServicesPage);
			await waitFor(() =>
				expect(screen.getByRole('button', { name: /actions for scheduler-service/i })).toBeInTheDocument()
			);
			fireEvent.click(screen.getByRole('button', { name: /actions for scheduler-service/i }));
			await waitFor(() => expect(document.querySelector('[data-ui="context-menu-item"]')).toBeInTheDocument());
		});
	});

	describe('Retry button', () => {
		it('renders variant="primary" (md default size) in error state', async () => {
			vi.mocked(api.getSystemServices).mockRejectedValue(new Error('network error'));
			render(SystemServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument());
			const retryBtn = screen.getByRole('button', { name: /retry/i });
			expect(retryBtn.className).toContain('h-[23px]');
			expect(retryBtn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
		});

		it('sets aria-busy="true" during fetch and clears after rejection', async () => {
			vi.mocked(api.getSystemServices).mockRejectedValue(new Error('network error'));
			render(SystemServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument());
			let resolveReject!: () => void;
			vi.mocked(api.getSystemServices).mockReturnValue(
				new Promise<never>((_, reject) => {
					resolveReject = () => reject(new Error('still failing'));
				})
			);
			fireEvent.click(screen.getByRole('button', { name: /retry/i }));
			await waitFor(() => expect(screen.getByRole('button', { name: /retry/i })).toHaveAttribute('aria-busy', 'true'));
			resolveReject();
			await waitFor(() => expect(screen.getByRole('button', { name: /retry/i })).not.toHaveAttribute('aria-busy'));
		});

		it('clears aria-busy after successful retry and hides the Retry button', async () => {
			const approvedSvc = {
				id: 'sys-ok',
				friendly_name: 'recovered-svc',
				hostname: 'host-c',
				ip_address: null,
				status: 'approved',
				is_embedded: false,
				yielded_to: [],
				last_seen_at: '2026-02-01T10:00:00Z',
				capabilities: []
			} as unknown as SystemServiceResponse;

			vi.mocked(api.getSystemServices).mockRejectedValue(new Error('network error'));
			render(SystemServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument());
			vi.mocked(api.getSystemServices).mockResolvedValue(makePage([approvedSvc]));
			fireEvent.click(screen.getByRole('button', { name: /retry/i }));
			await waitFor(() => expect(screen.queryByRole('button', { name: /retry/i })).not.toBeInTheDocument());
			expect(screen.getByText('recovered-svc')).toBeInTheDocument();
		});
	});

	describe('Ping modal footer (system-services)', () => {
		const approvedSystemSvc = {
			id: 'sys-approved',
			friendly_name: 'approved-scheduler',
			hostname: 'controller-b',
			ip_address: '10.10.1.6',
			status: 'approved',
			is_embedded: false,
			yielded_to: [],
			last_seen_at: '2026-02-01T10:00:00Z',
			capabilities: []
		} as unknown as SystemServiceResponse;

		beforeEach(() => {
			vi.mocked(api.getSystemServices).mockResolvedValue(makePage([approvedSystemSvc]));
		});

		async function openPingModal() {
			render(SystemServicesPage);
			await waitFor(() =>
				expect(screen.getByRole('button', { name: /actions for approved-scheduler/i })).toBeInTheDocument()
			);
			fireEvent.click(screen.getByRole('button', { name: /actions for approved-scheduler/i }));
			await waitFor(() => expect(screen.getByText('Edit Ping Interval')).toBeInTheDocument());
			fireEvent.click(screen.getByText('Edit Ping Interval'));
			await waitFor(() => expect(screen.getByRole('heading', { name: 'Edit Ping Interval' })).toBeInTheDocument());
		}

		it('Cancel renders variant="secondary"', async () => {
			await openPingModal();
			const cancelBtn = screen.getByRole('button', { name: /cancel/i });
			expect(cancelBtn.className).toContain('bg-[var(--bg-raised)]');
		});

		it('Save renders variant="primary" with static "Save" label', async () => {
			await openPingModal();
			const saveBtn = screen.getByRole('button', { name: /^save$/i });
			expect(saveBtn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
			expect(saveBtn.textContent).not.toContain('Saving');
		});

		it('Save shows aria-busy during submit and "Saving..." text never appears', async () => {
			vi.mocked(api.updateSystemService).mockReturnValue(new Promise(() => {}));
			await openPingModal();
			fireEvent.click(screen.getByRole('button', { name: /^save$/i }));
			await waitFor(() => expect(screen.getByRole('button', { name: /^save$/i })).toHaveAttribute('aria-busy', 'true'));
			expect(document.body.textContent).not.toContain('Saving...');
		});
	});

	it('ContextMenuItem entries are not wrapped in <Button> (scope guard for #3k)', async () => {
		render(SystemServicesPage);
		await waitFor(() =>
			expect(screen.getByRole('button', { name: /actions for scheduler-service/i })).toBeInTheDocument()
		);
		fireEvent.click(screen.getByRole('button', { name: /actions for scheduler-service/i }));
		await waitFor(() => expect(document.querySelector('[data-ui="context-menu-item"]')).toBeInTheDocument());
		const menuItems = document.querySelectorAll('[data-ui="context-menu-item"]');
		expect(menuItems.length).toBeGreaterThan(0);
		for (const item of menuItems) {
			expect(item.closest('button[class*="h-[23px]"]')).toBeNull();
			expect(item.closest('button[class*="h-[19px]"]')).toBeNull();
		}
	});
});
