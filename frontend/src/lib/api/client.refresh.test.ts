import { describe, it, expect, vi, beforeEach } from 'vitest';

// Mock the token store on the SAME specifier client.ts imports ('../token-store.svelte').
// The mock keeps an in-memory token so the refresh-retry can rotate it, and records
// every setSessionExpired call so tests can assert the LAST value (the banner's final
// observable state), not just that some clear/raise happened at some point. These are
// PLAIN functions (not vi.fn) so `vi.restoreAllMocks()` in beforeEach — which resets
// the fetch spy — cannot wipe the recorder or the token accessor.
const setSessionExpiredCalls: boolean[] = [];
let mockToken: string | null = 'old';
vi.mock('../token-store.svelte', () => ({
	getAccessToken: () => mockToken,
	setAccessToken: (t: string | null) => {
		mockToken = t;
	},
	setSessionExpired: (expired: boolean) => {
		setSessionExpiredCalls.push(expired);
	},
	// client.ts registers a listener at module-init time; return an unsubscribe fn.
	onTokenChange: () => () => {}
}));

import { apiClient } from './client';
// The DEFAULT singleton that generated SDK fns call as `(options?.client ?? client).get(...)`.
// Importing './client' first runs the module side effect that wraps this singleton's verbs.
import { client } from './generated/client.gen';
import { getAccessToken } from '../token-store.svelte';
import { ApiError } from './errors';

function lastSessionExpiredCall(): boolean | undefined {
	return setSessionExpiredCalls.at(-1);
}

// Reset the mock token-store state between tests so token mutations from one case
// (e.g. setAccessToken(null) on a dead session) don't leak into the next.
function resetTokenStoreMock(): void {
	mockToken = 'old';
	setSessionExpiredCalls.length = 0;
}

describe('S-A: refresh-retry + ApiError identity', () => {
	beforeEach(() => {
		vi.restoreAllMocks();
		resetTokenStoreMock();
	});

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

	it('on 401 → refresh succeeds → retry STILL 401s: typed ApiError, token cleared, banner stays raised', async () => {
		vi.spyOn(globalThis, 'fetch').mockImplementation(async (input, init) => {
			const isRequest = input instanceof Request;
			const url = isRequest ? input.url : String(input);

			if (url.endsWith('/auth/refresh')) {
				return new Response(JSON.stringify({ access_token: 'rotated-but-dead' }), { status: 200 });
			}

			const headers = isRequest ? input.headers : new Headers(init?.headers);
			// Every attempt 401s, even carrying the rotated token — session truly dead
			// (e.g. user deactivated, permissions revoked between call and retry).
			expect(headers.get('authorization')).toBeTruthy();
			return new Response(JSON.stringify({ error: 'user deactivated', error_code: 'user_deactivated' }), {
				status: 401
			});
		});

		await expect(apiClient.get({ url: '/still-dead' })).rejects.toSatisfy((e: unknown) => {
			expect(e).toBeInstanceOf(ApiError);
			const err = e as ApiError;
			expect(err.status).toBe(401);
			expect(err.errorCode).toBe('user_deactivated');
			return true;
		});

		expect(getAccessToken()).toBeNull();
		expect(lastSessionExpiredCall()).toBe(true);
	});

	it('on 401 → refresh succeeds → retry returns 403 (permissions revoked): typed ApiError, banner clears', async () => {
		vi.spyOn(globalThis, 'fetch').mockImplementation(async (input, init) => {
			const isRequest = input instanceof Request;
			const url = isRequest ? input.url : String(input);

			if (url.endsWith('/auth/refresh')) {
				return new Response(JSON.stringify({ access_token: 'rotated-limited' }), { status: 200 });
			}

			const headers = isRequest ? input.headers : new Headers(init?.headers);
			if (headers.get('authorization') === 'Bearer old') {
				return new Response('{}', { status: 401 });
			}
			// Retry carries the rotated token but the underlying permission was revoked —
			// a non-401 non-OK response, distinct from the auth-class 401 retry above.
			return new Response(JSON.stringify({ error: 'forbidden', error_code: 'forbidden' }), { status: 403 });
		});

		await expect(apiClient.get({ url: '/revoked' })).rejects.toSatisfy((e: unknown) => {
			expect(e).toBeInstanceOf(ApiError);
			const err = e as ApiError;
			expect(err.status).toBe(403);
			expect(err.errorCode).toBe('forbidden');
			return true;
		});

		expect(lastSessionExpiredCall()).toBe(false);
	});
});

describe('S-A: DEFAULT singleton path is refresh-aware (Task 6.5)', () => {
	beforeEach(() => {
		vi.restoreAllMocks();
		resetTokenStoreMock();
	});

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
