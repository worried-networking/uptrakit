import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import { Permission } from '$lib/types';

vi.mock('$app/state', () => ({
	page: {
		url: new URL('http://localhost/software?query=foo')
	}
}));

vi.mock('$app/navigation', () => ({
	goto: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null)
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
	getSurfacesBySlot: vi.fn(() => []),
	loadSurfaceReadModels: vi.fn(async () => {})
}));

vi.mock('$lib/surfaces/read-model', () => ({
	filterSurfacesByPermission: vi.fn(() => []),
	isSurfaceTabPending: vi.fn(() => false)
}));

vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn()
}));

import SoftwarePage from './+page.svelte';
import * as auth from '$lib/auth.svelte';
import * as api from '$lib/api';
import { page } from '$app/state';

const viewUser = {
	id: '00000000-0000-0000-0000-000000000001',
	email: 'user@example.com',
	first_name: 'Test',
	last_name: 'User',
	has_pending_email_change: false,
	permissions: [Permission.ViewSoftware]
};

describe('Software page — name filter URL initialization', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(viewUser);
	});

	it('pre-populates the name filter input from ?query= in the URL', async () => {
		render(SoftwarePage);

		await waitFor(() => expect(screen.getByRole('heading', { name: 'Software' })).toBeInTheDocument());

		const input = screen.getByRole('searchbox') as HTMLInputElement;
		expect(input.value).toBe('foo');
	});

	it('calls getSoftwareItems with the query parameter as the 7th argument', async () => {
		// Create a new URL with the nginx query parameter
		const nginxUrl = new URL('http://localhost/software?query=nginx');
		Object.defineProperty(page.url, 'href', { value: nginxUrl.href, configurable: true });
		Object.defineProperty(page.url, 'search', { value: nginxUrl.search, configurable: true });
		Object.defineProperty(page.url, 'searchParams', { value: nginxUrl.searchParams, configurable: true });

		render(SoftwarePage);

		await waitFor(() => expect(screen.getByRole('heading', { name: 'Software' })).toBeInTheDocument());

		expect(vi.mocked(api.getSoftwareItems)).toHaveBeenCalledWith(
			expect.anything(),
			undefined,
			expect.anything(),
			undefined,
			undefined,
			undefined,
			'nginx'
		);
	});
});
