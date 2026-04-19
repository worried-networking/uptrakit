import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { buildSoftwareTabsParityFixture } from '$lib/test-fixtures/ui-parity';

vi.mock('$app/state', () => ({
	page: {
		url: new URL('http://localhost/software?tab=proxmox.hosts')
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
		permissions: ['view_software']
	}))
}));

vi.mock('$lib/api', () => ({
	getSoftwareItems: vi.fn(async () => ({
		items: [],
		page: 1,
		per_page: 50,
		total: 0,
		total_pages: 1
	})),
	deleteSoftwareItem: vi.fn(async () => undefined),
	checkSoftwareItemVersions: vi.fn(async () => undefined),
	updateSoftwareItem: vi.fn(async () => undefined),
	listPluginTypes: vi.fn(async () => []),
	getSoftwareItem: vi.fn(async () => undefined),
	triggerSoftwareUpdate: vi.fn(async () => undefined),
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
	getSurfaceRuntimeStatus: vi.fn(() => ({ active: true })),
	getSurfacesBySlot: vi.fn((slot: string) =>
		slot === 'software.tabs' ? buildSoftwareTabsParityFixture().surfaceTabs : []
	),
	loadSurfaceReadModels: vi.fn(async () => {})
}));

import SoftwarePage from './+page.svelte';
import { getSoftwareItems } from '$lib/api';
import { goto } from '$app/navigation';
import { page } from '$app/state';
import { getSurfaceRuntimeStatus, getSurfacesBySlot } from '$lib/surfaces/registry.svelte';

describe('/software shared-surface tabs', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		vi.mocked(getSoftwareItems).mockResolvedValue({
			items: [],
			page: 1,
			per_page: 50,
			total: 0,
			total_pages: 1
		});
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('shows surface-backed tabs while read models are still pending', () => {
		render(SoftwarePage);

		expect(screen.getByRole('tab', { name: 'Proxmox VE Hosts' })).toBeInTheDocument();
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="tab-strip"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
	});

	it('hides surface tabs when runtime rollout is inactive even if slot surfaces are present', () => {
		vi.mocked(getSurfaceRuntimeStatus).mockReturnValue({ active: false });
		vi.mocked(getSurfacesBySlot).mockImplementation((slot: string) =>
			slot === 'software.tabs' ? buildSoftwareTabsParityFixture().surfaceTabs : []
		);

		render(SoftwarePage);

		expect(screen.queryByRole('tab', { name: 'Proxmox VE Hosts' })).not.toBeInTheDocument();
		expect(screen.getByRole('tab', { name: 'Featured' })).toBeInTheDocument();
	});

	it('shows a direct retry action when foreground software loading fails', async () => {
		vi.mocked(getSoftwareItems).mockRejectedValueOnce(new Error('Foreground load failed')).mockResolvedValueOnce({
			items: [],
			page: 1,
			per_page: 50,
			total: 0,
			total_pages: 1
		});

		render(SoftwarePage);

		expect(await screen.findByText('Foreground load failed')).toBeInTheDocument();
		const retryButton = await screen.findByRole('button', { name: 'Retry' });

		await fireEvent.click(retryButton);

		await waitFor(() => {
			expect(vi.mocked(getSoftwareItems)).toHaveBeenCalledTimes(2);
		});
	});

	it('defaults a missing tab query to All instead of Featured', async () => {
		page.url = new URL('http://localhost/software') as typeof page.url;

		render(SoftwarePage);

		await waitFor(() => {
			expect(vi.mocked(getSoftwareItems)).toHaveBeenCalled();
		});

		expect(screen.getByRole('tab', { name: 'All' })).toHaveAttribute('aria-selected', 'true');
		expect(screen.getByRole('tab', { name: 'Featured' })).toHaveAttribute('aria-selected', 'false');
		expect(vi.mocked(getSoftwareItems)).toHaveBeenCalledWith(1, undefined, undefined, undefined, undefined, undefined);
		expect(vi.mocked(goto)).toHaveBeenCalled();
		expect(vi.mocked(goto).mock.calls[0]?.[0]).not.toContain('tab=');
	});
});
