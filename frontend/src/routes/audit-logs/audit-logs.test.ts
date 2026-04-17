import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import { Permission, type AuditLogEntry } from '$lib/types';

vi.mock('$lib/api', () => ({
	listAuditLogs: vi.fn(),
	listSystemAuditLogs: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null)
}));

import AuditLogsPage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';

const user = {
	id: '00000000-0000-0000-0000-000000000105',
	email: 'audit@example.com',
	first_name: 'Audit',
	last_name: 'User',
	permissions: [Permission.ViewAuditLogs, Permission.ViewSystemAuditLogs]
};

const entry: AuditLogEntry = {
	id: 'audit-1',
	occurred_at: '2026-03-15T11:20:00Z',
	http_method: 'GET',
	http_path: '/api/v1/hosts',
	http_status: 200,
	actor_type: 'user',
	auth_method: 'session',
	duration_ms: 42,
	client_ip: '10.0.0.10'
} as unknown as AuditLogEntry;

describe('Audit Logs Route', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.listAuditLogs).mockResolvedValue({
			items: [entry],
			total: 1,
			page: 1,
			per_page: 25,
			total_pages: 1
		});
		vi.mocked(api.listSystemAuditLogs).mockResolvedValue({
			items: [],
			total: 0,
			page: 1,
			per_page: 25,
			total_pages: 1
		});
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('renders shared shell, table, and status badge primitives', async () => {
		render(AuditLogsPage);

		await waitFor(() => expect(screen.getByText('Audit Logs')).toBeInTheDocument());
		expect(screen.getByText('/api/v1/hosts')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="status-badge"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="table-footer-bar"]')).toBeInTheDocument();
		expect(screen.getByText('1 total')).toBeInTheDocument();
	});
});
