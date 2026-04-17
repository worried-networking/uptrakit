import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/svelte';
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
});
