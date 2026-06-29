import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { buildSoftwareTabsParityFixture } from '$lib/test-fixtures/ui-parity';

vi.mock('$app/state', () => ({
	page: {
		url: new URL('http://localhost/software')
	}
}));

vi.mock('$app/navigation', () => ({
	goto: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => ({
		id: 'user-1',
		email: 'user@example.com',
		first_name: 'Test',
		last_name: 'User',
		has_pending_email_change: false,
		permissions: ['view_software']
	}))
}));

vi.mock('$lib/api', () => ({
	listSoftwareItems: vi.fn(async () => ({
		data: {
			items: [],
			page: 1,
			per_page: 50,
			total: 0,
			total_pages: 1
		}
	})),
	deleteSoftwareItem: vi.fn(async () => undefined),
	checkVersions: vi.fn(async () => undefined),
	updateSoftwareItem: vi.fn(async () => undefined),
	listPluginTypes: vi.fn(async () => ({ data: [] })),
	getSoftwareItem: vi.fn(async () => undefined),
	triggerUpdate: vi.fn(async () => undefined),
	batchSoftwareItems: vi.fn(async () => undefined),
	executeBatchChunked: vi.fn(async () => undefined),
	previewSoftwareItemMerge: vi.fn(async () => undefined),
	executeSoftwareItemMerge: vi.fn(async () => undefined)
}));

vi.mock('$lib/stores/events.svelte', () => ({
	subscribeToEvent: vi.fn(() => () => {})
}));

vi.mock('$lib/surfaces/registry.svelte', () => ({
	getSurfaceReadLoading: vi.fn(() => false),
	getSurfaceReadModel: vi.fn(() => undefined),
	getSurfaceReadRequested: vi.fn(() => false),
	getSurfacesBySlot: vi.fn((slot: string) =>
		slot === 'software.tabs' ? buildSoftwareTabsParityFixture().surfaceTabs : []
	),
	loadSurfaceReadModels: vi.fn(async () => {})
}));

import SoftwarePage from './+page.svelte';
import { listSoftwareItems } from '$lib/api';
import * as api from '$lib/api';
import { page } from '$app/state';

describe('/software shared-surface tabs', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		page.url = new URL('http://localhost/software') as typeof page.url;
		vi.mocked(listSoftwareItems).mockResolvedValue({
			data: {
				items: [],
				page: 1,
				per_page: 50,
				total: 0,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof api.listSoftwareItems>>);
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('shows surface-backed tabs while read models are still pending', () => {
		render(SoftwarePage);

		expect(screen.getByRole('tab', { name: 'Plugin Category' })).toBeInTheDocument();
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="tab-strip"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
	});

	it('shows a direct retry action when foreground software loading fails', async () => {
		vi.mocked(listSoftwareItems)
			.mockRejectedValueOnce(new Error('Foreground load failed'))
			.mockResolvedValueOnce({
				data: {
					items: [],
					page: 1,
					per_page: 50,
					total: 0,
					total_pages: 1
				}
			} as unknown as Awaited<ReturnType<typeof api.listSoftwareItems>>);

		render(SoftwarePage);

		expect(await screen.findByText('Foreground load failed')).toBeInTheDocument();
		const retryButton = await screen.findByRole('button', { name: 'Retry' });

		await fireEvent.click(retryButton);

		await waitFor(() => {
			expect(vi.mocked(listSoftwareItems)).toHaveBeenCalledTimes(2);
		});
	});
});
