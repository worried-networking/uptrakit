import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { PaginatedResponse, SoftwareItemResponse } from '$lib/api';

function makeResponse(total: number): { data: PaginatedResponse<SoftwareItemResponse> } {
	return { data: { items: [], total, page: 1, per_page: 1, total_pages: total } };
}

describe('software-updates store', () => {
	beforeEach(() => {
		vi.resetModules();
	});

	it('getUpdatableSoftwareCount is null before any fetch', async () => {
		vi.doMock('$lib/api', () => ({ listSoftwareItems: vi.fn() }));
		const { getUpdatableSoftwareCount } = await import('$lib/stores/software-updates.svelte');
		expect(getUpdatableSoftwareCount()).toBeNull();
	});

	it('fetchUpdatableSoftwareCount sets count from response total', async () => {
		const listSoftwareItems = vi.fn().mockResolvedValue(makeResponse(42));
		vi.doMock('$lib/api', () => ({ listSoftwareItems }));
		const { getUpdatableSoftwareCount, fetchUpdatableSoftwareCount } =
			await import('$lib/stores/software-updates.svelte');
		await fetchUpdatableSoftwareCount();
		expect(getUpdatableSoftwareCount()).toBe(42);
	});

	it('fetchUpdatableSoftwareCount calls listSoftwareItems with correct args', async () => {
		const listSoftwareItems = vi.fn().mockResolvedValue(makeResponse(3));
		vi.doMock('$lib/api', () => ({ listSoftwareItems }));
		const { fetchUpdatableSoftwareCount } = await import('$lib/stores/software-updates.svelte');
		await fetchUpdatableSoftwareCount();
		expect(listSoftwareItems).toHaveBeenCalledWith({ query: { per_page: 1, featured: true, updatable: true } });
	});

	it('fetchUpdatableSoftwareCount is idempotent — second call skips network', async () => {
		const listSoftwareItems = vi.fn().mockResolvedValue(makeResponse(5));
		vi.doMock('$lib/api', () => ({ listSoftwareItems }));
		const { fetchUpdatableSoftwareCount } = await import('$lib/stores/software-updates.svelte');
		await fetchUpdatableSoftwareCount();
		await fetchUpdatableSoftwareCount();
		expect(listSoftwareItems).toHaveBeenCalledTimes(1);
	});

	it('fetchUpdatableSoftwareCount silently swallows errors, count stays null', async () => {
		const listSoftwareItems = vi.fn().mockRejectedValue(new Error('network error'));
		vi.doMock('$lib/api', () => ({ listSoftwareItems }));
		const { getUpdatableSoftwareCount, fetchUpdatableSoftwareCount } =
			await import('$lib/stores/software-updates.svelte');
		await expect(fetchUpdatableSoftwareCount()).resolves.toBeUndefined();
		expect(getUpdatableSoftwareCount()).toBeNull();
	});
});
