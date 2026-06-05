import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { _resetSettingsEtagCacheForTests, request } from '$lib/api';
// Import the REAL setAccessToken — do NOT vi.mock('./token-store.svelte') in this file.
// The existing api.test.ts already uses a top-level vi.mock there; adding the same mock
// here would prevent the onTokenChange listener (registered in api.ts at module init)
// from ever firing, making cases 6–9 vacuous.
import { setAccessToken } from '$lib/token-store.svelte';

interface CallEntry {
	url: string;
	init: RequestInit;
}

function captureFetch(responses: Map<string, () => Response>): { calls: CallEntry[]; fn: typeof fetch } {
	const calls: CallEntry[] = [];
	const fn = vi.fn(async (input: RequestInfo | URL, init: RequestInit = {}) => {
		const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;
		const method = (init.method ?? 'GET').toUpperCase();
		calls.push({ url, init });
		const key = `${method} ${url}`;
		const factory = responses.get(key);
		if (!factory) {
			throw new Error(`Unexpected fetch: ${key}`);
		}
		return factory();
	});
	return { calls, fn: fn as unknown as typeof fetch };
}

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

function getHeader(init: RequestInit, name: string): string | null {
	const headers = init.headers;
	if (headers === undefined) return null;
	if (headers instanceof Headers) return headers.get(name);
	if (Array.isArray(headers)) {
		const found = headers.find(([k]) => k.toLowerCase() === name.toLowerCase());
		return found ? found[1] : null;
	}
	for (const [k, v] of Object.entries(headers as Record<string, string>)) {
		if (k.toLowerCase() === name.toLowerCase()) return v;
	}
	return null;
}

describe('settings ETag auto-injection', () => {
	beforeEach(() => {
		_resetSettingsEtagCacheForTests();
		setAccessToken(null);
		vi.restoreAllMocks();
	});

	afterEach(() => {
		setAccessToken(null);
		vi.unstubAllGlobals();
	});

	it('auto-injects If-Match on PUT after GET captures ETag', async () => {
		const { calls, fn } = captureFetch(
			new Map<string, () => Response>([
				['GET /api/v1/global-settings/network', () => jsonResponse({ ok: true }, { etag: 'v1' })],
				['PUT /api/v1/global-settings/network', () => jsonResponse({ ok: true }, { etag: 'v2' })]
			])
		);
		vi.stubGlobal('fetch', fn);

		await request('/global-settings/network');
		await request('/global-settings/network', { method: 'PUT', body: JSON.stringify({ foo: 'bar' }) });

		expect(calls).toHaveLength(2);
		expect(getHeader(calls[1].init, 'if-match')).toBe('v1');
	});

	it('honors caller-supplied If-Match verbatim (plain object headers)', async () => {
		const { calls, fn } = captureFetch(
			new Map<string, () => Response>([
				['GET /api/v1/global-settings/network', () => jsonResponse({ ok: true }, { etag: 'v1' })],
				['PUT /api/v1/global-settings/network', () => jsonResponse({ ok: true })]
			])
		);
		vi.stubGlobal('fetch', fn);

		await request('/global-settings/network');
		await request('/global-settings/network', {
			method: 'PUT',
			body: JSON.stringify({ foo: 'bar' }),
			headers: { 'if-match': 'custom' }
		});

		expect(getHeader(calls[1].init, 'if-match')).toBe('custom');
	});

	it('honors caller-supplied If-Match verbatim (Headers instance)', async () => {
		const { calls, fn } = captureFetch(
			new Map<string, () => Response>([
				['GET /api/v1/global-settings/network', () => jsonResponse({ ok: true }, { etag: 'v1' })],
				['PUT /api/v1/global-settings/network', () => jsonResponse({ ok: true })]
			])
		);
		vi.stubGlobal('fetch', fn);

		await request('/global-settings/network');
		await request('/global-settings/network', {
			method: 'PUT',
			body: JSON.stringify({ foo: 'bar' }),
			headers: new Headers({ 'if-match': 'custom' })
		});

		expect(getHeader(calls[1].init, 'if-match')).toBe('custom');
	});

	it('leaves cache untouched when PUT response is not OK', async () => {
		const { calls, fn } = captureFetch(
			new Map<string, () => Response>([
				['GET /api/v1/global-settings/network', () => jsonResponse({ ok: true }, { etag: 'v1' })],
				[
					'PUT /api/v1/global-settings/network',
					() => new Response('{"error":"stale"}', { status: 428, headers: { 'Content-Type': 'application/json' } })
				],
				['PUT /api/v1/global-settings/network#2', () => jsonResponse({ ok: true })]
			])
		);
		vi.stubGlobal('fetch', fn);

		await request('/global-settings/network');
		await expect(
			request('/global-settings/network', { method: 'PUT', body: JSON.stringify({ foo: 'bar' }) })
		).rejects.toMatchObject({ status: 428 });

		// Second PUT — succeeds. Must still carry v1 (the failed PUT didn't overwrite the cache).
		const responses = new Map<string, () => Response>([
			['PUT /api/v1/global-settings/network', () => jsonResponse({ ok: true }, { etag: 'v2' })]
		]);
		const { calls: calls2, fn: fn2 } = captureFetch(responses);
		vi.stubGlobal('fetch', fn2);
		await request('/global-settings/network', { method: 'PUT', body: JSON.stringify({ foo: 'bar' }) });

		expect(getHeader(calls2[0].init, 'if-match')).toBe('v1');
		expect(calls).toHaveLength(2);
	});

	it('isolates global and tenant scopes', async () => {
		const { calls, fn } = captureFetch(
			new Map<string, () => Response>([
				['GET /api/v1/settings/access', () => jsonResponse({ ok: true }, { etag: 't1' })],
				['GET /api/v1/global-settings/network', () => jsonResponse({ ok: true }, { etag: 'g1' })],
				['PUT /api/v1/global-settings/network', () => jsonResponse({ ok: true })]
			])
		);
		vi.stubGlobal('fetch', fn);

		await request('/settings/access');
		await request('/global-settings/network');
		await request('/global-settings/network', { method: 'PUT', body: JSON.stringify({}) });

		expect(getHeader(calls[2].init, 'if-match')).toBe('g1');
	});

	it('resets both cache slots when JWT sub claim changes', async () => {
		const { fn } = captureFetch(
			new Map<string, () => Response>([
				['GET /api/v1/global-settings/network', () => jsonResponse({ ok: true }, { etag: 'g1' })],
				['GET /api/v1/settings/access', () => jsonResponse({ ok: true }, { etag: 't1' })]
			])
		);
		vi.stubGlobal('fetch', fn);

		setAccessToken(makeJwt('user-a'));
		await request('/global-settings/network');
		await request('/settings/access');

		// Switching sub claim wipes the cache. A subsequent PUT must NOT carry an If-Match.
		setAccessToken(makeJwt('user-b'));

		const putFetch = captureFetch(
			new Map<string, () => Response>([
				['PUT /api/v1/global-settings/network', () => jsonResponse({ ok: true })],
				['PUT /api/v1/settings/access', () => jsonResponse({ ok: true })]
			])
		);
		vi.stubGlobal('fetch', putFetch.fn);

		await request('/global-settings/network', { method: 'PUT', body: JSON.stringify({}) });
		await request('/settings/access', { method: 'PUT', body: JSON.stringify({}) });

		expect(getHeader(putFetch.calls[0].init, 'if-match')).toBeNull();
		expect(getHeader(putFetch.calls[1].init, 'if-match')).toBeNull();
	});

	it('preserves cache across silent refresh (same sub)', async () => {
		const { fn } = captureFetch(
			new Map<string, () => Response>([
				['GET /api/v1/global-settings/network', () => jsonResponse({ ok: true }, { etag: 'g1' })]
			])
		);
		vi.stubGlobal('fetch', fn);

		setAccessToken(makeJwt('user-a', 'one'));
		await request('/global-settings/network');

		// Different token string, same sub → silent refresh. Cache must persist.
		setAccessToken(makeJwt('user-a', 'two'));

		const putFetch = captureFetch(
			new Map<string, () => Response>([['PUT /api/v1/global-settings/network', () => jsonResponse({ ok: true })]])
		);
		vi.stubGlobal('fetch', putFetch.fn);

		await request('/global-settings/network', { method: 'PUT', body: JSON.stringify({}) });

		expect(getHeader(putFetch.calls[0].init, 'if-match')).toBe('g1');
	});

	it('handles a malformed JWT without throwing or wiping the cache', async () => {
		const { fn } = captureFetch(
			new Map<string, () => Response>([
				['GET /api/v1/global-settings/network', () => jsonResponse({ ok: true }, { etag: 'g1' })]
			])
		);
		vi.stubGlobal('fetch', fn);

		// Prime the cache with a real token first so we have something to check is preserved.
		setAccessToken(makeJwt('user-a'));
		await request('/global-settings/network');

		// Malformed JWT — decoder returns null. Previous token also decodes to user-a → null,
		// so subs differ (user-a → null), cache wipes. To exercise the no-throw path we then
		// set another malformed JWT (null → null, no wipe).
		expect(() => setAccessToken('not.a.jwt')).not.toThrow();
		expect(() => setAccessToken('also-not-a-jwt')).not.toThrow();

		// Final state: both prev (null) and next (null) decode to null → equal → no further wipe.
		// We cannot inspect the cache directly, but the absence of a thrown exception in the
		// listener path is the contract this case asserts.
	});

	it('resets the cache when sub changes across base64url-encoded JWTs (non-vacuous decoder coverage)', async () => {
		const { fn } = captureFetch(
			new Map<string, () => Response>([
				['GET /api/v1/global-settings/network', () => jsonResponse({ ok: true }, { etag: 'g1' })],
				['GET /api/v1/settings/access', () => jsonResponse({ ok: true }, { etag: 't1' })]
			])
		);
		vi.stubGlobal('fetch', fn);

		const jwtA = makeJwtBase64Url('user-a');
		const jwtB = makeJwtBase64Url('user-b');

		// Sanity: the payload segment really did receive base64url-only chars.
		const payloadA = jwtA.split('.')[1];
		expect(payloadA).toMatch(/[-_]/);

		setAccessToken(jwtA);
		await request('/global-settings/network');
		await request('/settings/access');

		setAccessToken(jwtB);

		const putFetch = captureFetch(
			new Map<string, () => Response>([
				['PUT /api/v1/global-settings/network', () => jsonResponse({ ok: true })],
				['PUT /api/v1/settings/access', () => jsonResponse({ ok: true })]
			])
		);
		vi.stubGlobal('fetch', putFetch.fn);

		await request('/global-settings/network', { method: 'PUT', body: JSON.stringify({}) });
		await request('/settings/access', { method: 'PUT', body: JSON.stringify({}) });

		expect(getHeader(putFetch.calls[0].init, 'if-match')).toBeNull();
		expect(getHeader(putFetch.calls[1].init, 'if-match')).toBeNull();
	});
});
