import { describe, it, expect, vi, beforeEach } from 'vitest';

// Mock the token store on the SAME specifier client.ts imports ('../token-store.svelte').
// The mock keeps an in-memory token so the refresh-retry can rotate it.
vi.mock('../token-store.svelte', () => {
	let token: string | null = 'old';
	return {
		getAccessToken: () => token,
		setAccessToken: (t: string | null) => (token = t),
		setSessionExpired: vi.fn(),
		// client.ts registers a listener at module-init time; return an unsubscribe fn.
		onTokenChange: () => () => {}
	};
});

import { apiClient } from './client';
// The DEFAULT singleton that generated SDK fns call as `(options?.client ?? client).get(...)`.
// Importing './client' first runs the module side effect that wraps this singleton's verbs.
import { client } from './generated/client.gen';
import { getAccessToken } from '../token-store.svelte';
import { ApiError } from './errors';

describe('S-A: refresh-retry + ApiError identity', () => {
	beforeEach(() => vi.restoreAllMocks());

	it('on 401: refreshes once (deduped), retries with new token + body, succeeds', async () => {
		let refreshCalls = 0;
		// hey-api's fetch client calls fetch(request: Request) with a SINGLE Request arg,
		// while the ported refreshAccessToken() uses fetch(urlString, init). Handle both.
		const fetchSpy = vi.spyOn(globalThis, 'fetch').mockImplementation(async (input, init) => {
			const isRequest = input instanceof Request;
			const url = isRequest ? input.url : String(input);

			if (url.endsWith('/auth/refresh')) {
				refreshCalls++;
				return new Response(JSON.stringify({ access_token: 'new' }), { status: 200 });
			}

			const headers = isRequest ? input.headers : new Headers(init?.headers);
			if (headers.get('authorization') === 'Bearer old') {
				return new Response('{}', { status: 401 });
			}

			// retry carries the new token AND the reconstructed original body — assert the
			// SPECIFIC payload per URL (the R3 invariant: each retry sends its own call's body)
			const body = isRequest ? await input.clone().text() : String(init?.body ?? '');
			const parsed = JSON.parse(body);
			if (url.endsWith('/x')) expect(parsed).toEqual({ a: 1 });
			else if (url.endsWith('/y')) expect(parsed).toEqual({ b: 2 });
			else throw new Error(`unexpected retry url: ${url}`);
			return new Response(JSON.stringify({ ok: true }), { status: 200 });
		});

		// two concurrent calls → single shared refresh
		const [a, b] = await Promise.all([
			apiClient.post({ url: '/x', body: { a: 1 } }),
			apiClient.post({ url: '/y', body: { b: 2 } })
		]);

		expect(refreshCalls).toBe(1);
		expect(a.data ?? a).toBeTruthy();
		expect(b.data ?? b).toBeTruthy();
		expect(fetchSpy).toHaveBeenCalled();
	});

	it('a subsequent non-401 error still arrives as instanceof ApiError with status + errorCode', async () => {
		vi.spyOn(globalThis, 'fetch').mockResolvedValue(
			new Response(JSON.stringify({ error: 'e', error_code: 'c' }), { status: 422 })
		);
		await expect(apiClient.get({ url: '/z' })).rejects.toSatisfy(
			(e: unknown) => e instanceof ApiError && (e as ApiError).status === 422 && (e as ApiError).errorCode === 'c'
		);
	});
});

describe('S-A: DEFAULT singleton path is refresh-aware (Task 6.5)', () => {
	beforeEach(() => vi.restoreAllMocks());

	// Drive the singleton's verb methods directly — this is the exact call shape the
	// generated SDK fns use (`client.get/post/...`) when no explicit client is passed.
	// Order-independent: the first data request 401s for ANY current token; only a
	// request bearing the freshly-rotated token succeeds.
	it('on 401 via singleton client.get/post: refreshes once (deduped), retries, rotates token, succeeds', async () => {
		let refreshCalls = 0;
		const fetchSpy = vi.spyOn(globalThis, 'fetch').mockImplementation(async (input, init) => {
			const isRequest = input instanceof Request;
			const url = isRequest ? input.url : String(input);

			if (url.endsWith('/auth/refresh')) {
				refreshCalls++;
				return new Response(JSON.stringify({ access_token: 'rotated' }), { status: 200 });
			}

			const headers = isRequest ? input.headers : new Headers(init?.headers);
			// Anything not yet carrying the rotated token gets a 401 → forces refresh-retry.
			if (headers.get('authorization') !== 'Bearer rotated') {
				return new Response('{}', { status: 401 });
			}
			return new Response(JSON.stringify({ ok: true }), { status: 200 });
		});

		// Two concurrent singleton calls → a single shared (deduped) refresh.
		const [a, b] = await Promise.all([
			client.get({ url: '/singleton-get' }),
			client.post({ url: '/singleton-post', body: { n: 1 } })
		]);

		expect(refreshCalls).toBe(1);
		expect(a.data ?? a).toBeTruthy();
		expect(b.data ?? b).toBeTruthy();
		// Token was rotated by the singleton path's refresh-retry.
		expect(getAccessToken()).toBe('rotated');
		expect(fetchSpy).toHaveBeenCalled();
	});
});
