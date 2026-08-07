import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { Actions, type PaginatedResponse, type SystemServiceResponse } from '$lib/api';

vi.mock('$lib/api', async (importOriginal) => ({
	...(await importOriginal<typeof import('$lib/api')>()),
	listSystemServices: vi.fn(),
	approveSystemService: vi.fn(),
	rejectSystemService: vi.fn(),
	deactivateSystemService: vi.fn(),
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
	has_pending_email_change: false,
	actions: [
		Actions.SYSTEM_SERVICES_READ,
		Actions.SYSTEM_SERVICES_APPROVE,
		Actions.SYSTEM_SERVICES_REJECT,
		Actions.SYSTEM_SERVICES_DELETE,
		Actions.SYSTEM_SERVICES_UPDATE
	],
	authority: 'ok' as const
};

function makePage(items: SystemServiceResponse[]): PaginatedResponse<SystemServiceResponse> {
	return { items, total: items.length, page: 1, per_page: 25, total_pages: 1 };
}

describe('System Services Route', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.listSystemServices).mockResolvedValue({
			data: makePage([
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
		} as unknown as Awaited<ReturnType<typeof api.listSystemServices>>);
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
		vi.mocked(api.listSystemServices).mockResolvedValue({
			data: makePage([
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
		} as unknown as Awaited<ReturnType<typeof api.listSystemServices>>);

		render(SystemServicesPage);

		await waitFor(() => expect(screen.getByText('embedded-scheduler')).toBeInTheDocument());
		const badgeStack = document.querySelector('[data-ui="status-badge-stack"]');
		expect(badgeStack).toBeInTheDocument();
		expect(within(badgeStack as HTMLElement).getByText('Approved')).toBeInTheDocument();
		expect(within(badgeStack as HTMLElement).getByText('Embedded')).toBeInTheDocument();
		expect(within(badgeStack as HTMLElement).getByText('Yielded (1)')).toBeInTheDocument();
	});

	describe('status filter Select', () => {
		beforeEach(() => {
			vi.mocked(api.listSystemServices).mockResolvedValue({ data: makePage([]) } as unknown as Awaited<
				ReturnType<typeof api.listSystemServices>
			>);
		});

		it('Select is present inside [data-ui="filter-bar"]', async () => {
			render(SystemServicesPage);
			await waitFor(() => expect(screen.getByLabelText('Filter by status')).toBeInTheDocument());
			const select = screen.getByLabelText('Filter by status') as HTMLSelectElement;
			expect(select.value).toBe('all');
		});

		it('no separate "Status Filters" SectionCard', async () => {
			render(SystemServicesPage);
			await waitFor(() =>
				expect(screen.getByRole('heading', { name: 'Registered System Services' })).toBeInTheDocument()
			);
			expect(screen.queryByRole('heading', { name: 'Status Filters' })).not.toBeInTheDocument();
		});

		it('initial load calls getSystemServices with no status filter', async () => {
			render(SystemServicesPage);
			await waitFor(() => expect(vi.mocked(api.listSystemServices)).toHaveBeenCalled());
			expect(vi.mocked(api.listSystemServices)).toHaveBeenCalledWith(
				expect.objectContaining({ query: expect.objectContaining({ status: undefined }) })
			);
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
			vi.mocked(api.listSystemServices).mockResolvedValue({
				data: makePage([
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
			} as unknown as Awaited<ReturnType<typeof api.listSystemServices>>);
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
			vi.mocked(api.listSystemServices).mockRejectedValue(new Error('network error'));
			render(SystemServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument());
			const retryBtn = screen.getByRole('button', { name: /retry/i });
			expect(retryBtn.className).toContain('h-[23px]');
			expect(retryBtn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
		});

		it('sets aria-busy="true" during fetch and clears after rejection', async () => {
			vi.mocked(api.listSystemServices).mockRejectedValue(new Error('network error'));
			render(SystemServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument());
			let resolveReject!: () => void;
			vi.mocked(api.listSystemServices).mockReturnValue(
				new Promise<never>((_, reject) => {
					resolveReject = () => reject(new Error('still failing'));
				}) as unknown as ReturnType<typeof api.listSystemServices>
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

			vi.mocked(api.listSystemServices).mockRejectedValue(new Error('network error'));
			render(SystemServicesPage);
			await waitFor(() => expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument());
			vi.mocked(api.listSystemServices).mockResolvedValue({ data: makePage([approvedSvc]) } as unknown as Awaited<
				ReturnType<typeof api.listSystemServices>
			>);
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
			vi.mocked(api.listSystemServices).mockResolvedValue({ data: makePage([approvedSystemSvc]) } as unknown as Awaited<
				ReturnType<typeof api.listSystemServices>
			>);
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
			vi.mocked(api.updateSystemService).mockReturnValue(
				new Promise(() => {}) as unknown as ReturnType<typeof api.updateSystemService>
			);
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
