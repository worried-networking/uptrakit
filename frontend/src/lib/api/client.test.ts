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
		const err = await apiClient.get({ url: '/whatever' }).catch((e: unknown) => e);
		expect(err).toBeInstanceOf(ApiError);
		expect((err as ApiError).status).toBe(422);
		expect((err as ApiError).errorCode).toBe('y');
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

	// Regression guard for the /api/v1 double-prefix bug: generated SDK op urls carry
	// the full `/api/v1/...` path. The client baseUrl must therefore be origin-only so
	// that buildUrl yields a SINGLE `/api/v1`, never `origin/api/v1/api/v1/...`.
	it('does not double-prefix /api/v1 when building a generated op url', () => {
		const built = apiClient.buildUrl({ url: '/api/v1/auth/me' });
		expect(built).not.toContain('/api/v1/api/v1/');
		expect(built).toContain('/api/v1/auth/me');
	});

	// The same guard exercised through the real fetch path (request URL the client
	// actually issues), proving auth bootstrap calls hit `/api/v1/auth/me` once.
	it('issues a single /api/v1 prefix on a generated op request', async () => {
		vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response('{}', { status: 200 }));
		await apiClient.get({ url: '/api/v1/auth/me' });
		const req = vi.mocked(fetch).mock.calls[0][0] as Request;
		expect(req.url).not.toContain('/api/v1/api/v1/');
		expect(req.url).toContain('/api/v1/auth/me');
	});
});
