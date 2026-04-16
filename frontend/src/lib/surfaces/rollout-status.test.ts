import { afterEach, describe, expect, it, vi } from 'vitest';
import { getSurfaceRuntimeStatus } from '$lib/api';

vi.mock('$lib/token-store.svelte', () => ({
	getAccessToken: vi.fn().mockReturnValue(null),
	setAccessToken: vi.fn(),
	getSessionExpired: vi.fn().mockReturnValue(false),
	setSessionExpired: vi.fn()
}));

describe('surface rollout status api', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
	});

	it('returns active=true when controller reports enabled rollout', async () => {
		const mockFetch = vi.fn().mockResolvedValue(
			new Response(JSON.stringify({ active: true }), {
				status: 200,
				headers: { 'Content-Type': 'application/json' }
			})
		);
		vi.stubGlobal('fetch', mockFetch);

		await expect(getSurfaceRuntimeStatus()).resolves.toEqual({ active: true });
		expect(mockFetch.mock.calls[0][0]).toContain('/api/v1/surfaces/runtime-status');
	});

	it('propagates server error payloads', async () => {
		const mockFetch = vi.fn().mockResolvedValue(
			new Response(JSON.stringify({ error: 'runtime unavailable' }), {
				status: 503,
				statusText: 'Service Unavailable',
				headers: { 'Content-Type': 'application/json' }
			})
		);
		vi.stubGlobal('fetch', mockFetch);

		await expect(getSurfaceRuntimeStatus()).rejects.toThrow('runtime unavailable');
	});
});
