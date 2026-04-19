import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
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
