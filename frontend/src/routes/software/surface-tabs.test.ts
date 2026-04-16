import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';

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
		page_size: 50,
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
		slot === 'software.tabs'
			? [
					{
						surface_id: 'proxmox.hosts',
						label: 'Proxmox VE Hosts',
						priority: 100,
						slot: 'software.tabs',
						scope: 'tenant',
						targeting: 'universal',
						required_permission: 'view_software',
						provider_kind: 'plugin',
						required_capabilities: [],
						root_node: { kind: 'text_block', text: 'proxmox' },
						provider_count: 1
					}
				]
			: []
	),
	loadSurfaceReadModels: vi.fn(async () => {})
}));

import SoftwarePage from './+page.svelte';

describe('/software shared-surface tabs', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('shows surface-backed tabs while read models are still pending', () => {
		render(SoftwarePage);

		expect(screen.getByRole('button', { name: 'Proxmox VE Hosts' })).toBeInTheDocument();
	});
});
