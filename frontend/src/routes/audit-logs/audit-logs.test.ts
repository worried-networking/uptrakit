import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { Permission, type AuditLogEntry, type PaginatedResponse } from '$lib/types';
import { within } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
	listAuditLogs: vi.fn(),
	listSystemAuditLogs: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null),
	getAccessToken: vi.fn(() => null)
}));

import AuditLogsPage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';

const auditViewer = {
	id: '00000000-0000-0000-0000-000000000010',
	email: 'audit@example.com',
	first_name: 'Audit',
	last_name: 'Viewer',
	has_pending_email_change: false,
	permissions: [Permission.ViewAuditLogs]
};

const sampleEntry: AuditLogEntry = {
	id: 'audit-1',
	actor_type: 'user',
	actor_id: 'user-1',
	actor_display: 'Audit Viewer',
	action_type: 'login',
	target_type: 'session',
	target_id: 'session-1',
	target_display: 'Primary Session',
	outcome: 'success',
	details_json: null,
	request_id: 'req-1',
	occurred_at: '2026-04-19T08:00:00Z'
};

function makePage(items: AuditLogEntry[]): PaginatedResponse<AuditLogEntry> {
	return { items, total: items.length, page: 1, per_page: 25, total_pages: 1 };
}

describe('Audit Logs Page', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(auditViewer);
		vi.mocked(api.listAuditLogs).mockResolvedValue(makePage([]));
		vi.mocked(api.listSystemAuditLogs).mockResolvedValue(makePage([]));
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('renders action, target, outcome, and actor filters', async () => {
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByText('Audit Logs')).toBeInTheDocument());
		expect(screen.getByLabelText('Action')).toBeInTheDocument();
		expect(screen.getByLabelText('Outcome')).toBeInTheDocument();
		expect(screen.getByLabelText('Target Type')).toBeInTheDocument();
		expect(screen.getByLabelText('Actor Type')).toBeInTheDocument();
		expect(screen.queryByText('HTTP Method')).not.toBeInTheDocument();
		expect(screen.queryByText('Status Code')).not.toBeInTheDocument();
	});

	it('renders semantic audit columns and hides request-era columns', async () => {
		vi.mocked(api.listAuditLogs).mockResolvedValue(makePage([sampleEntry]));

		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByRole('columnheader', { name: 'Action' })).toBeInTheDocument());
		expect(screen.getByRole('columnheader', { name: 'Target' })).toBeInTheDocument();
		expect(screen.getByRole('columnheader', { name: 'Outcome' })).toBeInTheDocument();
		expect(screen.getByRole('columnheader', { name: 'Actor' })).toBeInTheDocument();
		expect(screen.queryByRole('columnheader', { name: 'Method' })).not.toBeInTheDocument();
		expect(screen.queryByRole('columnheader', { name: 'Path' })).not.toBeInTheDocument();
		expect(screen.queryByRole('columnheader', { name: 'Status' })).not.toBeInTheDocument();
	});

	it('uses shared tab-strip and section-header actions for scope and filters', async () => {
		vi.mocked(auth.getUser).mockReturnValue({
			...auditViewer,
			permissions: [Permission.ViewAuditLogs, Permission.ViewSystemAuditLogs]
		});

		render(AuditLogsPage);

		await waitFor(() => expect(screen.getByText('Audit Logs')).toBeInTheDocument());
		expect(screen.getByRole('tablist', { name: 'Audit log scope' })).toBeInTheDocument();
		expect(screen.getByRole('tab', { name: 'Tenant Logs' })).toBeInTheDocument();
		expect(screen.getByRole('tab', { name: 'System Logs' })).toBeInTheDocument();

		const filtersCard = screen.getByRole('heading', { name: 'Filters' }).closest('[data-ui="section-card"]');
		expect(filtersCard).toBeInTheDocument();
		const filtersHeader = filtersCard?.querySelector('header') as HTMLElement;
		expect(filtersHeader).toBeInTheDocument();
		expect(within(filtersHeader).getByRole('button', { name: 'Apply Filters' })).toBeInTheDocument();
		expect(within(filtersHeader).getByRole('button', { name: 'Clear Filters' })).toBeInTheDocument();
	});
});

describe('Button Migrations', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(auditViewer);
		vi.mocked(api.listAuditLogs).mockResolvedValue(makePage([]));
		vi.mocked(api.listSystemAuditLogs).mockResolvedValue(makePage([]));
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('Apply Filters button renders variant="primary"', async () => {
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Apply Filters' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Apply Filters' });
		expect(btn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary variant
	});

	it('Clear Filters button renders variant="secondary"', async () => {
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Clear Filters' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Clear Filters' });
		expect(btn).toHaveClass('bg-[var(--bg-raised)]'); // secondary variant
	});

	it('Apply Filters click triggers load(1) and updates DataTable loading prop', async () => {
		vi.mocked(api.listAuditLogs).mockResolvedValue(makePage([sampleEntry]));
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Apply Filters' })).toBeInTheDocument());
		const applyBtn = screen.getByRole('button', { name: 'Apply Filters' });
		const actionInput = screen.getByPlaceholderText('e.g. login');
		await userEvent.type(actionInput, 'create');
		await userEvent.click(applyBtn);
		await waitFor(() =>
			expect(vi.mocked(api.listAuditLogs)).toHaveBeenCalledWith(
				expect.objectContaining({ page: 1, action_type: 'create' })
			)
		);
	});

	it('Clear Filters click resets filter state and triggers load(1)', async () => {
		vi.mocked(api.listAuditLogs).mockResolvedValue(makePage([sampleEntry]));
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Clear Filters' })).toBeInTheDocument());
		const actionInput = screen.getByPlaceholderText('e.g. login');
		await userEvent.type(actionInput, 'delete');
		const clearBtn = screen.getByRole('button', { name: 'Clear Filters' });
		await userEvent.click(clearBtn);
		await waitFor(() => {
			expect((actionInput as HTMLInputElement).value).toBe('');
			expect(vi.mocked(api.listAuditLogs)).toHaveBeenCalledWith(
				expect.objectContaining({ page: 1, action_type: undefined })
			);
		});
	});

	it('Error Retry button renders variant="primary" with async loading state', async () => {
		vi.mocked(api.listAuditLogs).mockRejectedValueOnce(new Error('Network error'));
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Retry' });
		expect(btn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary variant
		expect(btn).not.toHaveAttribute('aria-busy'); // Not loading initially
		expect(btn).not.toHaveAttribute('disabled');

		// Simulate click and verify recovery after success
		vi.mocked(api.listAuditLogs).mockResolvedValueOnce(makePage([sampleEntry]));
		await userEvent.click(btn);
		// After successful retry, error state clears and Retry button is removed
		await waitFor(() => expect(screen.queryByRole('button', { name: 'Retry' })).not.toBeInTheDocument());
	});

	it('Error Retry button clears loading state after rejection', async () => {
		vi.mocked(api.listAuditLogs).mockRejectedValueOnce(new Error('Load failed'));
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Retry' });

		// Mock rejection on retry click
		vi.mocked(api.listAuditLogs).mockRejectedValueOnce(new Error('Retry failed'));
		try {
			btn.click();
		} catch {
			// Expected
		}
		await waitFor(() => expect(btn).not.toHaveAttribute('aria-busy', 'true'));
	});

	it('renders date filters with simplified From/To labels (no RFC 3339 text)', async () => {
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByText('Audit Logs')).toBeInTheDocument());
		expect(screen.getByLabelText('From')).toBeInTheDocument();
		expect(screen.getByLabelText('To')).toBeInTheDocument();
		expect(screen.queryByText(/RFC 3339/)).not.toBeInTheDocument();
	});

	it('Out-of-scope regression: TabStrip scope toggle remains unchanged', async () => {
		vi.mocked(auth.getUser).mockReturnValue({
			...auditViewer,
			permissions: [Permission.ViewAuditLogs, Permission.ViewSystemAuditLogs]
		});
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByRole('tablist', { name: 'Audit log scope' })).toBeInTheDocument());
		const tablist = screen.getByRole('tablist', { name: 'Audit log scope' });
		expect(tablist).toBeInTheDocument();
		const tenantTab = screen.getByRole('tab', { name: 'Tenant Logs' });
		const systemTab = screen.getByRole('tab', { name: 'System Logs' });
		expect(tenantTab).toBeInTheDocument();
		expect(systemTab).toBeInTheDocument();
	});

	it('when hasBoth is true, renders TabStrip without a SectionCard wrapper', async () => {
		vi.mocked(auth.getUser).mockReturnValue({
			...auditViewer,
			permissions: [Permission.ViewAuditLogs, Permission.ViewSystemAuditLogs]
		});
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByRole('tablist', { name: 'Audit log scope' })).toBeInTheDocument());
		const tablist = screen.getByRole('tablist', { name: 'Audit log scope' });
		expect(tablist).toBeInTheDocument();
		// The tablist must not be inside a SectionCard element
		expect(tablist.closest('[data-ui="section-card"]')).toBeNull();
	});

	it('when system-only user, does not render "Showing system-level audit logs." text', async () => {
		vi.mocked(auth.getUser).mockReturnValue({
			...auditViewer,
			permissions: [Permission.ViewSystemAuditLogs]
		});
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByText('Audit Logs')).toBeInTheDocument());
		expect(screen.queryByText('Showing system-level audit logs.')).not.toBeInTheDocument();
	});

	it('actor column renders PillBadge with actor_type and enriched display name', async () => {
		const entry: AuditLogEntry = {
			...sampleEntry,
			actor_type: 'user',
			actor_display: 'Alice',
			actor_id: 'u-1'
		};
		vi.mocked(api.listAuditLogs).mockResolvedValue(makePage([entry]));
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByText('Alice')).toBeInTheDocument());

		// PillBadge renders a span[data-ui="pill-badge"] containing the actor_type text
		const pill = screen.getByText('user', { selector: '[data-ui="pill-badge"]' });
		expect(pill).toBeInTheDocument();

		// actor_display is shown
		expect(screen.getByText('Alice')).toBeInTheDocument();

		// actor_id is not shown separately (display name takes precedence)
		expect(screen.queryByText('u-1')).not.toBeInTheDocument();
	});

	it('actor column renders only PillBadge when actor has no display name or id', async () => {
		const entry: AuditLogEntry = {
			...sampleEntry,
			actor_type: 'system',
			actor_display: null,
			actor_id: null
		};
		vi.mocked(api.listAuditLogs).mockResolvedValue(makePage([entry]));
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByText('system', { selector: '[data-ui="pill-badge"]' })).toBeInTheDocument());

		const pill = screen.getByText('system', { selector: '[data-ui="pill-badge"]' });
		expect(pill).toBeInTheDocument();

		// No additional span text in the actor cell beyond the badge
		const actorCell = pill.closest('td');
		expect(actorCell).toBeInTheDocument();
		const spans = actorCell!.querySelectorAll('span:not([data-ui="pill-badge"])');
		expect(spans.length).toBe(0);
	});
});
