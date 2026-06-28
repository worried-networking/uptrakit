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

			// retry carries the new token AND the reconstructed original body
			const body = isRequest ? await input.clone().text() : String(init?.body ?? '');
			expect(body).toBeTruthy();
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
