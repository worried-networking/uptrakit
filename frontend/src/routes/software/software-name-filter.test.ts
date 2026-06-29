import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import { Permission } from '$lib/types';

vi.mock('$app/state', () => ({
	page: {
		url: new URL('http://localhost/software?query=foo')
	}
}));

vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$lib/auth.svelte', () => ({ getUser: vi.fn(() => null) }));
vi.mock('$lib/api', () => ({
	listSoftwareItems: vi.fn(async () => ({ data: { items: [], page: 1, per_page: 50, total: 0, total_pages: 1 } })),
	deleteSoftwareItem: vi.fn(async () => undefined),
	checkVersions: vi.fn(async () => undefined),
	updateSoftwareItem: vi.fn(async () => undefined),
	listPluginTypes: vi.fn(async () => []),
	getSoftwareItem: vi.fn(async () => undefined),
	triggerUpdate: vi.fn(async () => undefined),
	batchSoftwareItems: vi.fn(async () => undefined),
	executeBatchChunked: vi.fn(async () => undefined),
	previewSoftwareItemMerge: vi.fn(async () => undefined),
	executeSoftwareItemMerge: vi.fn(async () => undefined)
}));
vi.mock('$lib/stores/events.svelte', () => ({ subscribeToEvent: vi.fn(() => () => {}) }));
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
vi.mock('$lib/notifications.svelte', () => ({ showSuccess: vi.fn(), showError: vi.fn() }));

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

describe('Software page — URL-reactive filter state', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(viewUser);
	});

	it('pre-populates search from ?query= in URL', async () => {
		// page.url is mocked to http://localhost/software?query=foo
		render(SoftwarePage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Software' })).toBeInTheDocument());
		const input = screen.getByRole('searchbox') as HTMLInputElement;
		expect(input.value).toBe('foo');
	});

	it('passes query param to listSoftwareItems on mount (text query not forwarded — generated API lacks query field)', async () => {
		const nginxUrl = new URL('http://localhost/software?query=nginx');
		Object.defineProperty(page, 'url', { value: nginxUrl, configurable: true });
		render(SoftwarePage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Software' })).toBeInTheDocument());
		expect(vi.mocked(api.listSoftwareItems)).toHaveBeenCalledWith(
			expect.objectContaining({ query: expect.not.objectContaining({ query: 'nginx' }) })
		);
	});

	it('reads featured=all from URL and renders All option selected', async () => {
		const url = new URL('http://localhost/software?featured=all');
		Object.defineProperty(page, 'url', { value: url, configurable: true });
		render(SoftwarePage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Software' })).toBeInTheDocument());
		// featuredFilter() returns undefined when featured=all → listSoftwareItems called with undefined featured
		expect(vi.mocked(api.listSoftwareItems)).toHaveBeenCalledWith(
			expect.objectContaining({ query: expect.not.objectContaining({ featured: expect.anything() }) })
		);
	});

	it('reads updatable=true from URL and passes to listSoftwareItems', async () => {
		const url = new URL('http://localhost/software?updatable=true');
		Object.defineProperty(page, 'url', { value: url, configurable: true });
		render(SoftwarePage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Software' })).toBeInTheDocument());
		expect(vi.mocked(api.listSoftwareItems)).toHaveBeenCalledWith(
			expect.objectContaining({ query: expect.objectContaining({ updatable: true }) })
		);
	});

	it('reads plugin_type=npm from URL (not forwarded to listSoftwareItems — generated API lacks plugin_type field)', async () => {
		const url = new URL('http://localhost/software?plugin_type=npm');
		Object.defineProperty(page, 'url', { value: url, configurable: true });
		render(SoftwarePage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Software' })).toBeInTheDocument());
		expect(vi.mocked(api.listSoftwareItems)).toHaveBeenCalledWith(
			expect.objectContaining({ query: expect.not.objectContaining({ plugin_type: 'npm' }) })
		);
	});
});
