import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import { Permission, type AuditLogEntry, type PaginatedResponse } from '$lib/types';

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
		render(AuditLogsPage);
		await waitFor(() => expect(screen.getByRole('columnheader', { name: 'Action' })).toBeInTheDocument());
		expect(screen.getByRole('columnheader', { name: 'Target' })).toBeInTheDocument();
		expect(screen.getByRole('columnheader', { name: 'Outcome' })).toBeInTheDocument();
		expect(screen.getByRole('columnheader', { name: 'Actor' })).toBeInTheDocument();
		expect(screen.queryByRole('columnheader', { name: 'Method' })).not.toBeInTheDocument();
		expect(screen.queryByRole('columnheader', { name: 'Path' })).not.toBeInTheDocument();
		expect(screen.queryByRole('columnheader', { name: 'Status' })).not.toBeInTheDocument();
	});
});
