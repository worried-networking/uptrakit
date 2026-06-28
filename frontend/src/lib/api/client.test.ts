import { describe, it, expect, beforeEach, vi } from 'vitest';

// Mock the token store before importing client.ts.
// Must mock the same specifier that client.ts imports: '../token-store.svelte'.
vi.mock('../token-store.svelte', () => {
	let token: string | null = 'tok';
	return {
		getAccessToken: () => token,
		setAccessToken: (t: string | null) => (token = t),
		setSessionExpired: vi.fn(),
		// onTokenChange is called at module init time; return an unsubscribe fn per contract.
		onTokenChange: () => () => {}
	};
});

import { apiClient, _resetSettingsEtagCacheForTests } from './client';
import { ApiError } from './errors';

describe('client interceptors', () => {
	beforeEach(() => {
		vi.restoreAllMocks();
		_resetSettingsEtagCacheForTests();
	});

	it('attaches Bearer token on requests', async () => {
		vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response('{}', { status: 200 }));
		await apiClient.get({ url: '/auth/me' });
		// The client calls fetch(request: Request) with a single Request argument.
		const req = vi.mocked(fetch).mock.calls[0][0] as Request;
		expect(req.headers.get('authorization')).toBe('Bearer tok');
	});

	it('maps non-2xx to a thrown ApiError with status + errorCode', async () => {
		vi.spyOn(globalThis, 'fetch').mockResolvedValue(
			new Response(JSON.stringify({ error: 'x', error_code: 'y' }), {
				status: 422
			})
		);
		await expect(apiClient.get({ url: '/whatever' })).rejects.toMatchObject({
			constructor: ApiError,
			status: 422,
			errorCode: 'y'
		});
	});

	it('injects cached If-Match on settings PUT', async () => {
		const spy = vi
			.spyOn(globalThis, 'fetch')
			.mockResolvedValueOnce(new Response('{}', { status: 200, headers: { etag: 'W/"v1"' } }))
			.mockResolvedValueOnce(new Response('{}', { status: 200 }));

		// GET primes the ETag cache for the 'tenant' scope (/settings/*)
		await apiClient.get({ url: '/settings/agent-certificates' });
		// PUT should receive the cached ETag in If-Match
		await apiClient.put({ url: '/settings/agent-certificates', body: {} });

		// spy.mock.calls[0] = GET, spy.mock.calls[1] = PUT
		const putReq = spy.mock.calls[1][0] as Request;
		expect(putReq.headers.get('if-match')).toBe('W/"v1"');
	});
});
