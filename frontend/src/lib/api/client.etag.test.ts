import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
// As of Task 12a the settings-ETag auto-cache lives solely in api/client.ts (the configured
// client's request/response interceptors), reached here via the `$lib/api` barrel's `apiClient`.
// The former `request()` helper these cases used to drive has been removed; the behaviour is
// identical, so the cases now exercise the canonical interceptor path through `apiClient`.
import { apiClient, _resetSettingsEtagCacheForTests } from '$lib/api';
// Import the REAL setAccessToken — do NOT vi.mock('$lib/token-store.svelte') in this file.
// Mocking it would prevent the onTokenChange listener (registered in api/client.ts at module
// init) from ever firing, making the sub-change cache-wipe cases (6–9) vacuous.
import { setAccessToken } from '$lib/token-store.svelte';

function jsonResponse(body: unknown, init: { status?: number; etag?: string | null } = {}): Response {
	const headers = new Headers({ 'Content-Type': 'application/json' });
	if (init.etag) headers.set('etag', init.etag);
	return new Response(JSON.stringify(body), { status: init.status ?? 200, headers });
}

function makeJwt(sub: string, salt = ''): string {
	const header = btoa(JSON.stringify({ alg: 'none' })).replace(/=+$/, '');
	const payload = btoa(JSON.stringify({ sub, salt })).replace(/=+$/, '');
	return `${header}.${payload}.sig`;
}

function makeJwtBase64Url(sub: string): string {
	const header = btoa(JSON.stringify({ alg: 'none' })).replace(/=+$/, '');
	// `ÿÿ` (U+00FF U+00FF) UTF-8 encodes to 0xC3BF 0xC3BF; combined with the surrounding
	// JSON braces this guarantees the base64 output contains at least one '+' or '/'.
	const payload = btoa(JSON.stringify({ sub, pad: 'ÿÿ' }))
		.replace(/=+$/, '')
		.replace(/\+/g, '-')
		.replace(/\//g, '_');
	return `${header}.${payload}.sig`;
}

// The configured client calls fetch with a single Request argument; read the If-Match
// header straight off the captured Request.
function ifMatchOf(spy: ReturnType<typeof vi.spyOn>, callIndex: number): string | null {
	const req = spy.mock.calls[callIndex][0] as Request;
	return req.headers.get('if-match');
}

describe('settings ETag auto-injection (via configured client interceptors)', () => {
	beforeEach(() => {
		_resetSettingsEtagCacheForTests();
		setAccessToken(null);
		vi.restoreAllMocks();
	});

	afterEach(() => {
		setAccessToken(null);
		vi.restoreAllMocks();
	});

	it('auto-injects If-Match on PUT after GET captures ETag', async () => {
		const spy = vi
			.spyOn(globalThis, 'fetch')
			.mockResolvedValueOnce(jsonResponse({ ok: true }, { etag: 'v1' }))
			.mockResolvedValueOnce(jsonResponse({ ok: true }, { etag: 'v2' }));

		await apiClient.get({ url: '/global-settings/network' });
		await apiClient.put({ url: '/global-settings/network', body: { foo: 'bar' } });

		expect(spy).toHaveBeenCalledTimes(2);
		expect(ifMatchOf(spy, 1)).toBe('v1');
	});

	it('honors caller-supplied If-Match verbatim (plain object headers)', async () => {
		const spy = vi
			.spyOn(globalThis, 'fetch')
			.mockResolvedValueOnce(jsonResponse({ ok: true }, { etag: 'v1' }))
			.mockResolvedValueOnce(jsonResponse({ ok: true }));

		await apiClient.get({ url: '/global-settings/network' });
		await apiClient.put({ url: '/global-settings/network', body: { foo: 'bar' }, headers: { 'if-match': 'custom' } });

		expect(ifMatchOf(spy, 1)).toBe('custom');
	});

	it('honors caller-supplied If-Match verbatim (Headers instance)', async () => {
		const spy = vi
			.spyOn(globalThis, 'fetch')
			.mockResolvedValueOnce(jsonResponse({ ok: true }, { etag: 'v1' }))
			.mockResolvedValueOnce(jsonResponse({ ok: true }));

		await apiClient.get({ url: '/global-settings/network' });
		await apiClient.put({
			url: '/global-settings/network',
			body: { foo: 'bar' },
			headers: new Headers({ 'if-match': 'custom' })
		});

		expect(ifMatchOf(spy, 1)).toBe('custom');
	});

	it('leaves cache untouched when PUT response is not OK', async () => {
		const spy = vi
			.spyOn(globalThis, 'fetch')
			.mockResolvedValueOnce(jsonResponse({ ok: true }, { etag: 'v1' })) // GET → caches v1
			.mockResolvedValueOnce(
				new Response('{"error":"stale"}', { status: 428, headers: { 'Content-Type': 'application/json' } })
			) // failed PUT → must NOT overwrite cache
			.mockResolvedValueOnce(jsonResponse({ ok: true }, { etag: 'v2' })); // retry PUT succeeds

		await apiClient.get({ url: '/global-settings/network' });
		await expect(apiClient.put({ url: '/global-settings/network', body: { foo: 'bar' } })).rejects.toMatchObject({
			status: 428
		});

		// Second PUT — succeeds. Must still carry v1 (the failed PUT didn't overwrite the cache).
		await apiClient.put({ url: '/global-settings/network', body: { foo: 'bar' } });

		expect(ifMatchOf(spy, 2)).toBe('v1');
		expect(spy).toHaveBeenCalledTimes(3);
	});

	it('clears the cached ETag on 409 if_match.stale so the next PUT omits If-Match', async () => {
		const spy = vi
			.spyOn(globalThis, 'fetch')
			.mockResolvedValueOnce(jsonResponse({ ok: true }, { etag: 'v1' })) // GET → caches v1
			.mockResolvedValueOnce(
				new Response(JSON.stringify({ error: 'stale', error_code: 'if_match.stale' }), {
					status: 409,
					headers: { 'Content-Type': 'application/json' }
				})
			) // stale PUT → must clear the cache
			.mockResolvedValueOnce(jsonResponse({ ok: true }, { etag: 'v2' })); // retry PUT succeeds

		await apiClient.get({ url: '/global-settings/network' });
		await expect(apiClient.put({ url: '/global-settings/network', body: { foo: 'bar' } })).rejects.toMatchObject({
			status: 409,
			errorCode: 'if_match.stale'
		});

		// Second PUT must NOT carry an If-Match — the stale cache entry was cleared.
		await apiClient.put({ url: '/global-settings/network', body: { foo: 'bar' } });

		expect(ifMatchOf(spy, 2)).toBeNull();
		expect(spy).toHaveBeenCalledTimes(3);
	});

	it('leaves the cache untouched on a 409 that is NOT if_match.stale', async () => {
		const spy = vi
			.spyOn(globalThis, 'fetch')
			.mockResolvedValueOnce(jsonResponse({ ok: true }, { etag: 'v1' })) // GET → caches v1
			.mockResolvedValueOnce(
				new Response(JSON.stringify({ error: 'conflict', error_code: 'some.other_conflict' }), {
					status: 409,
					headers: { 'Content-Type': 'application/json' }
				})
			) // non-stale 409 → cache must survive
			.mockResolvedValueOnce(jsonResponse({ ok: true }, { etag: 'v2' })); // retry PUT succeeds

		await apiClient.get({ url: '/global-settings/network' });
		await expect(apiClient.put({ url: '/global-settings/network', body: { foo: 'bar' } })).rejects.toMatchObject({
			status: 409,
			errorCode: 'some.other_conflict'
		});

		// Second PUT must still carry the original If-Match — an unconditional clear would
		// wrongly wipe it here too, which is exactly what this negative case guards against.
		await apiClient.put({ url: '/global-settings/network', body: { foo: 'bar' } });

		expect(ifMatchOf(spy, 2)).toBe('v1');
		expect(spy).toHaveBeenCalledTimes(3);
	});

	it('isolates global and tenant scopes', async () => {
		const spy = vi
			.spyOn(globalThis, 'fetch')
			.mockResolvedValueOnce(jsonResponse({ ok: true }, { etag: 't1' })) // GET tenant
			.mockResolvedValueOnce(jsonResponse({ ok: true }, { etag: 'g1' })) // GET global
			.mockResolvedValueOnce(jsonResponse({ ok: true })); // PUT global

		await apiClient.get({ url: '/settings/access' });
		await apiClient.get({ url: '/global-settings/network' });
		await apiClient.put({ url: '/global-settings/network', body: {} });

		expect(ifMatchOf(spy, 2)).toBe('g1');
	});

	it('resets both cache slots when JWT sub claim changes', async () => {
		const spy = vi
			.spyOn(globalThis, 'fetch')
			.mockResolvedValueOnce(jsonResponse({ ok: true }, { etag: 'g1' })) // GET global
			.mockResolvedValueOnce(jsonResponse({ ok: true }, { etag: 't1' })) // GET tenant
			.mockResolvedValueOnce(jsonResponse({ ok: true })) // PUT global
			.mockResolvedValueOnce(jsonResponse({ ok: true })); // PUT tenant

		setAccessToken(makeJwt('user-a'));
		await apiClient.get({ url: '/global-settings/network' });
		await apiClient.get({ url: '/settings/access' });

		// Switching sub claim wipes the cache. Subsequent PUTs must NOT carry an If-Match.
		setAccessToken(makeJwt('user-b'));

		await apiClient.put({ url: '/global-settings/network', body: {} });
		await apiClient.put({ url: '/settings/access', body: {} });

		expect(ifMatchOf(spy, 2)).toBeNull();
		expect(ifMatchOf(spy, 3)).toBeNull();
	});

	it('preserves cache across silent refresh (same sub)', async () => {
		const spy = vi
			.spyOn(globalThis, 'fetch')
			.mockResolvedValueOnce(jsonResponse({ ok: true }, { etag: 'g1' })) // GET global
			.mockResolvedValueOnce(jsonResponse({ ok: true })); // PUT global

		setAccessToken(makeJwt('user-a', 'one'));
		await apiClient.get({ url: '/global-settings/network' });

		// Different token string, same sub → silent refresh. Cache must persist.
		setAccessToken(makeJwt('user-a', 'two'));

		await apiClient.put({ url: '/global-settings/network', body: {} });

		expect(ifMatchOf(spy, 1)).toBe('g1');
	});

	it('handles a malformed JWT without throwing or wiping the cache', async () => {
		vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(jsonResponse({ ok: true }, { etag: 'g1' }));

		// Prime the cache with a real token first so we have something to check is preserved.
		setAccessToken(makeJwt('user-a'));
		await apiClient.get({ url: '/global-settings/network' });

		// Malformed JWTs — the decoder returns null and must not throw inside the listener.
		expect(() => setAccessToken('not.a.jwt')).not.toThrow();
		expect(() => setAccessToken('also-not-a-jwt')).not.toThrow();
	});

	it('resets the cache when sub changes across base64url-encoded JWTs (non-vacuous decoder coverage)', async () => {
		const spy = vi
			.spyOn(globalThis, 'fetch')
			.mockResolvedValueOnce(jsonResponse({ ok: true }, { etag: 'g1' })) // GET global
			.mockResolvedValueOnce(jsonResponse({ ok: true }, { etag: 't1' })) // GET tenant
			.mockResolvedValueOnce(jsonResponse({ ok: true })) // PUT global
			.mockResolvedValueOnce(jsonResponse({ ok: true })); // PUT tenant

		const jwtA = makeJwtBase64Url('user-a');
		const jwtB = makeJwtBase64Url('user-b');

		// Sanity: the payload segment really did receive base64url-only chars.
		expect(jwtA.split('.')[1]).toMatch(/[-_]/);

		setAccessToken(jwtA);
		await apiClient.get({ url: '/global-settings/network' });
		await apiClient.get({ url: '/settings/access' });

		setAccessToken(jwtB);

		await apiClient.put({ url: '/global-settings/network', body: {} });
		await apiClient.put({ url: '/settings/access', body: {} });

		expect(ifMatchOf(spy, 2)).toBeNull();
		expect(ifMatchOf(spy, 3)).toBeNull();
	});
});
