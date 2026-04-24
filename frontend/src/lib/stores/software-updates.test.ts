import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { PaginatedResponse, SoftwareItemResponse } from '$lib/types';

function makeResponse(total: number): PaginatedResponse<SoftwareItemResponse> {
	return { items: [], total, page: 1, per_page: 1, total_pages: total };
}

describe('software-updates store', () => {
	beforeEach(() => {
		vi.resetModules();
	});

	it('getUpdatableSoftwareCount is null before any fetch', async () => {
		vi.doMock('$lib/api', () => ({ getSoftwareItems: vi.fn() }));
		const { getUpdatableSoftwareCount } = await import('$lib/stores/software-updates.svelte');
		expect(getUpdatableSoftwareCount()).toBeNull();
	});

	it('fetchUpdatableSoftwareCount sets count from response total', async () => {
		const getSoftwareItems = vi.fn().mockResolvedValue(makeResponse(42));
		vi.doMock('$lib/api', () => ({ getSoftwareItems }));
		const { getUpdatableSoftwareCount, fetchUpdatableSoftwareCount } =
			await import('$lib/stores/software-updates.svelte');
		await fetchUpdatableSoftwareCount();
		expect(getUpdatableSoftwareCount()).toBe(42);
	});

	it('fetchUpdatableSoftwareCount calls getSoftwareItems with correct args', async () => {
		const getSoftwareItems = vi.fn().mockResolvedValue(makeResponse(3));
		vi.doMock('$lib/api', () => ({ getSoftwareItems }));
		const { fetchUpdatableSoftwareCount } = await import('$lib/stores/software-updates.svelte');
		await fetchUpdatableSoftwareCount();
		expect(getSoftwareItems).toHaveBeenCalledWith(undefined, 1, true, undefined, true);
	});

	it('fetchUpdatableSoftwareCount is idempotent — second call skips network', async () => {
		const getSoftwareItems = vi.fn().mockResolvedValue(makeResponse(5));
		vi.doMock('$lib/api', () => ({ getSoftwareItems }));
		const { fetchUpdatableSoftwareCount } = await import('$lib/stores/software-updates.svelte');
		await fetchUpdatableSoftwareCount();
		await fetchUpdatableSoftwareCount();
		expect(getSoftwareItems).toHaveBeenCalledTimes(1);
	});

	it('fetchUpdatableSoftwareCount silently swallows errors, count stays null', async () => {
		const getSoftwareItems = vi.fn().mockRejectedValue(new Error('network error'));
		vi.doMock('$lib/api', () => ({ getSoftwareItems }));
		const { getUpdatableSoftwareCount, fetchUpdatableSoftwareCount } =
			await import('$lib/stores/software-updates.svelte');
		await expect(fetchUpdatableSoftwareCount()).resolves.toBeUndefined();
		expect(getUpdatableSoftwareCount()).toBeNull();
	});
});
