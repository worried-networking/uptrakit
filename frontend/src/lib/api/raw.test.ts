import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { apiGet, authenticatedFetch, loginRaw } from './raw';
import { getAccessToken, setAccessToken, setSessionExpired } from '../token-store.svelte';
import type { RefreshResponse, UserResponse } from '$lib/api';

// raw.ts (via ./client) imports token state from '../token-store.svelte'; client.ts
// also registers an onTokenChange listener at module-init time — provide it.
vi.mock('../token-store.svelte', () => ({
	getAccessToken: vi.fn().mockReturnValue(null),
	setAccessToken: vi.fn(),
	onTokenChange: vi.fn(() => () => {}),
	getSessionExpired: vi.fn().mockReturnValue(false),
	setSessionExpired: vi.fn()
}));

const URL = 'http://localhost/api/v1/auth/me';

const sampleUser: UserResponse = {
	id: 'user-1',
	email: 'user@example.com',
	first_name: 'Test',
	last_name: 'User',
	has_pending_email_change: false,
	actions: [],
	authority: 'ok'
};

const sampleRefresh: RefreshResponse = {
	access_token: 'new-token',
	refresh_token: 'refresh-token',
	expires_in: 3600,
	token_type: 'Bearer'
};

// ── authenticatedFetch ────────────────────────────────────────────────────────
// Tested directly: authenticatedFetch takes a FULL URL and returns the raw Response.
// Auth/refresh/2fa plumbing is shared from ./client (single source of truth).

describe('authenticatedFetch', () => {
	beforeEach(() => {
		vi.mocked(getAccessToken).mockReturnValue(null);
		vi.mocked(setAccessToken).mockReset();
		vi.mocked(setSessionExpired).mockReset();
	});

	afterEach(() => {
		vi.unstubAllGlobals();
	});

	it('includes Authorization header when a token is set', async () => {
		vi.mocked(getAccessToken).mockReturnValue('my-token');
		const mockFetch = vi.fn().mockResolvedValue(new Response(JSON.stringify(sampleUser), { status: 200 }));
		vi.stubGlobal('fetch', mockFetch);

		await authenticatedFetch(URL);

		expect(mockFetch).toHaveBeenCalledTimes(1);
		const callOptions = mockFetch.mock.calls[0][1] as RequestInit;
		const headers = callOptions.headers as Headers;
		expect(headers.get('Authorization')).toBe('Bearer my-token');
	});

	it('does not include Authorization header when no token is set', async () => {
		vi.mocked(getAccessToken).mockReturnValue(null);
		const mockFetch = vi.fn().mockResolvedValue(new Response(JSON.stringify(sampleUser), { status: 200 }));
		vi.stubGlobal('fetch', mockFetch);

		await authenticatedFetch(URL);

		expect(mockFetch).toHaveBeenCalledTimes(1);
		const callOptions = mockFetch.mock.calls[0][1] as RequestInit;
		const headers = callOptions.headers as Headers;
		expect(headers.get('Authorization')).toBeNull();
	});

	it('does not prepend a base URL — the caller-supplied full URL is fetched verbatim', async () => {
		vi.mocked(getAccessToken).mockReturnValue(null);
		const mockFetch = vi.fn().mockResolvedValue(new Response('{}', { status: 200 }));
		vi.stubGlobal('fetch', mockFetch);

		await authenticatedFetch('/oauth/consent/abc');

		expect(mockFetch).toHaveBeenCalledTimes(1);
		expect(mockFetch.mock.calls[0][0]).toBe('/oauth/consent/abc');
	});

	it('retries with new token after a 401 (3 fetch calls total)', async () => {
		vi.mocked(getAccessToken).mockReturnValue('old-token');
		const mockFetch = vi
			.fn()
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockResolvedValueOnce(new Response(JSON.stringify(sampleRefresh), { status: 200 }))
			.mockResolvedValueOnce(new Response(JSON.stringify(sampleUser), { status: 200 }));
		vi.stubGlobal('fetch', mockFetch);

		const res = await authenticatedFetch(URL);

		expect(mockFetch).toHaveBeenCalledTimes(3);
		expect(vi.mocked(setAccessToken)).toHaveBeenCalledWith('new-token');
		const retryOptions = mockFetch.mock.calls[2][1] as RequestInit;
		const retryHeaders = retryOptions.headers as Headers;
		expect(retryHeaders.get('Authorization')).toBe('Bearer new-token');
		expect(res.status).toBe(200);
		expect(await res.json()).toEqual(sampleUser);
	});

	it('deduplicates concurrent 401 refresh calls', async () => {
		vi.mocked(getAccessToken).mockReturnValue('old-token');
		const mockFetch = vi
			.fn()
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockResolvedValueOnce(new Response(JSON.stringify(sampleRefresh), { status: 200 }))
			.mockResolvedValueOnce(new Response(JSON.stringify(sampleUser), { status: 200 }))
			.mockResolvedValueOnce(new Response(JSON.stringify(sampleUser), { status: 200 }));
		vi.stubGlobal('fetch', mockFetch);

		await Promise.all([authenticatedFetch(URL), authenticatedFetch(URL)]);

		// 2 original requests + 1 shared refresh + 2 retries = 5
		expect(mockFetch).toHaveBeenCalledTimes(5);
		const refreshCalls = mockFetch.mock.calls.filter((args: unknown[]) =>
			(args[0] as string).includes('/auth/refresh')
		);
		expect(refreshCalls).toHaveLength(1);
		expect(vi.mocked(setAccessToken)).toHaveBeenCalledWith('new-token');
	});

	it('clears token and sets sessionExpired on 401 with 4xx refresh failure (no hard redirect)', async () => {
		vi.mocked(getAccessToken).mockReturnValue('old-token');
		const mockFetch = vi
			.fn()
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockResolvedValueOnce(new Response('Forbidden', { status: 403, statusText: 'Forbidden' }));
		vi.stubGlobal('fetch', mockFetch);

		const locationDescriptor = Object.getOwnPropertyDescriptor(window, 'location');
		let hrefAssigned = false;
		Object.defineProperty(window, 'location', {
			value: {
				...window.location,
				set href(_v: string) {
					hrefAssigned = true;
				}
			},
			writable: true,
			configurable: true
		});

		await expect(authenticatedFetch(URL)).rejects.toThrow('Session expired. Please log in again.');

		expect(vi.mocked(setAccessToken)).toHaveBeenCalledWith(null);
		expect(vi.mocked(setSessionExpired)).toHaveBeenCalledWith(true);
		expect(hrefAssigned).toBe(false);

		if (locationDescriptor) {
			Object.defineProperty(window, 'location', locationDescriptor);
		}
	});

	it('preserves token and throws on 401 with 5xx refresh failure', async () => {
		vi.mocked(getAccessToken).mockReturnValue('old-token');
		const mockFetch = vi
			.fn()
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockResolvedValueOnce(
				new Response('Internal Server Error', { status: 500, statusText: 'Internal Server Error' })
			);
		vi.stubGlobal('fetch', mockFetch);

		await expect(authenticatedFetch(URL)).rejects.toThrow('Server error during token refresh');

		expect(vi.mocked(setAccessToken)).not.toHaveBeenCalledWith(null);
	});

	it('returns the raw 401 Response (no refresh) when no token is set', async () => {
		vi.mocked(getAccessToken).mockReturnValue(null);
		const mockFetch = vi
			.fn()
			.mockResolvedValue(
				new Response(JSON.stringify({ error: 'Unauthorized' }), { status: 401, statusText: 'Unauthorized' })
			);
		vi.stubGlobal('fetch', mockFetch);

		const res = await authenticatedFetch(URL);

		expect(res.status).toBe(401);
		// Only the original request — no refresh attempt
		expect(mockFetch).toHaveBeenCalledTimes(1);
		expect(vi.mocked(setAccessToken)).not.toHaveBeenCalled();
		expect(vi.mocked(setSessionExpired)).not.toHaveBeenCalled();
	});

	it('propagates the raw timeout DOMException when fetch is aborted by the timeout signal', async () => {
		vi.mocked(getAccessToken).mockReturnValue(null);
		const timeoutError = new DOMException('signal timed out', 'TimeoutError');
		const mockFetch = vi.fn().mockRejectedValue(timeoutError);
		vi.stubGlobal('fetch', mockFetch);

		const err = await authenticatedFetch(URL).catch((e: unknown) => e);
		expect(err).toBeInstanceOf(DOMException);
		expect((err as DOMException).name).toBe('TimeoutError');
	});

	it('redirects to /profile#security on a 403 2fa_setup_required response', async () => {
		vi.mocked(getAccessToken).mockReturnValue('my-token');
		const mockFetch = vi
			.fn()
			.mockResolvedValue(new Response(JSON.stringify({ error: '2fa_setup_required' }), { status: 403 }));
		vi.stubGlobal('fetch', mockFetch);

		const locationDescriptor = Object.getOwnPropertyDescriptor(window, 'location');
		let assignedHref: string | null = null;
		Object.defineProperty(window, 'location', {
			value: {
				...window.location,
				set href(v: string) {
					assignedHref = v;
				}
			},
			writable: true,
			configurable: true
		});

		const res = await authenticatedFetch(URL);

		expect(assignedHref).toBe('/profile#security');
		expect(res.status).toBe(403);

		if (locationDescriptor) {
			Object.defineProperty(window, 'location', locationDescriptor);
		}
	});

	// ── session-expired banner lifecycle ─────────────────────────────────────

	it('sets sessionExpired true immediately and clears it after successful refresh and retry', async () => {
		vi.mocked(getAccessToken).mockReturnValue('old-token');
		const mockFetch = vi
			.fn()
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockResolvedValueOnce(new Response(JSON.stringify(sampleRefresh), { status: 200 }))
			.mockResolvedValueOnce(new Response(JSON.stringify(sampleUser), { status: 200 }));
		vi.stubGlobal('fetch', mockFetch);

		const res = await authenticatedFetch(URL);

		const calls = vi.mocked(setSessionExpired).mock.calls.map((c) => c[0]);
		expect(calls).toContain(true);
		expect(calls[calls.length - 1]).toBe(false);
		expect(res.status).toBe(200);
	});

	it('clears sessionExpired via finally even when the retry fetch rejects', async () => {
		vi.mocked(getAccessToken).mockReturnValue('old-token');
		const mockFetch = vi
			.fn()
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockResolvedValueOnce(new Response(JSON.stringify(sampleRefresh), { status: 200 }))
			.mockRejectedValueOnce(new TypeError('Failed to fetch'));
		vi.stubGlobal('fetch', mockFetch);

		await expect(authenticatedFetch(URL)).rejects.toThrow('Failed to fetch');

		expect(vi.mocked(setAccessToken)).toHaveBeenCalledWith('new-token');
		const calls = vi.mocked(setSessionExpired).mock.calls.map((c) => c[0]);
		expect(calls).toEqual([true, false]);
	});

	it('keeps sessionExpired true when refresh fails with 4xx', async () => {
		vi.mocked(getAccessToken).mockReturnValue('old-token');
		const mockFetch = vi
			.fn()
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockResolvedValueOnce(new Response('Forbidden', { status: 403, statusText: 'Forbidden' }));
		vi.stubGlobal('fetch', mockFetch);

		await expect(authenticatedFetch(URL)).rejects.toThrow('Session expired. Please log in again.');

		expect(vi.mocked(setAccessToken)).toHaveBeenCalledWith(null);
		const calls = vi.mocked(setSessionExpired).mock.calls.map((c) => c[0]);
		expect(calls).toContain(true);
		expect(calls).not.toContain(false);
	});

	it('clears sessionExpired when refresh fails with TypeError', async () => {
		vi.mocked(getAccessToken).mockReturnValue('old-token');
		const mockFetch = vi
			.fn()
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockRejectedValueOnce(new TypeError('Failed to fetch'));
		vi.stubGlobal('fetch', mockFetch);

		await expect(authenticatedFetch(URL)).rejects.toThrow('Network error during token refresh. Check your connection.');

		const calls = vi.mocked(setSessionExpired).mock.calls.map((c) => c[0]);
		expect(calls).toEqual([true, false]);
	});

	it('clears sessionExpired when refresh fails with 503', async () => {
		vi.mocked(getAccessToken).mockReturnValue('old-token');
		const mockFetch = vi
			.fn()
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockResolvedValueOnce(new Response('Service Unavailable', { status: 503, statusText: 'Service Unavailable' }));
		vi.stubGlobal('fetch', mockFetch);

		await expect(authenticatedFetch(URL)).rejects.toThrow('Server error during token refresh. Please try again later.');

		const calls = vi.mocked(setSessionExpired).mock.calls.map((c) => c[0]);
		expect(calls).toEqual([true, false]);
	});

	it('clears sessionExpired when refresh times out with a DOMException', async () => {
		vi.mocked(getAccessToken).mockReturnValue('old-token');
		const mockFetch = vi
			.fn()
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockRejectedValueOnce(new DOMException('signal timed out', 'TimeoutError'));
		vi.stubGlobal('fetch', mockFetch);

		await expect(authenticatedFetch(URL)).rejects.toThrow('Token refresh timed out. Please try again.');

		const calls = vi.mocked(setSessionExpired).mock.calls.map((c) => c[0]);
		expect(calls).toEqual([true, false]);
	});

	it('concurrent 401s sharing one refresh both clear sessionExpired after retry', async () => {
		vi.mocked(getAccessToken).mockReturnValue('old-token');
		const mockFetch = vi
			.fn()
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockResolvedValueOnce(new Response(JSON.stringify(sampleRefresh), { status: 200 }))
			.mockResolvedValueOnce(new Response(JSON.stringify(sampleUser), { status: 200 }))
			.mockResolvedValueOnce(new Response(JSON.stringify(sampleUser), { status: 200 }));
		vi.stubGlobal('fetch', mockFetch);

		const results = await Promise.all([authenticatedFetch(URL), authenticatedFetch(URL)]);

		const calls = vi.mocked(setSessionExpired).mock.calls.map((c) => c[0]);
		const trueCalls = calls.filter((v) => v === true);
		const falseCalls = calls.filter((v) => v === false);
		expect(trueCalls.length).toBeGreaterThanOrEqual(2);
		expect(falseCalls.length).toBeGreaterThanOrEqual(2);
		expect(calls[calls.length - 1]).toBe(false);
		expect(results[0].status).toBe(200);
		expect(results[1].status).toBe(200);
	});
});

// ── loginRaw ────────────────────────────────────────────────────────────────
// Login needs NO auth/refresh; the raw Response is returned so callers can inspect
// the 202 MFA challenge without it being thrown.

describe('loginRaw', () => {
	beforeEach(() => {
		vi.mocked(getAccessToken).mockReturnValue(null);
		vi.mocked(setAccessToken).mockReset();
	});

	afterEach(() => {
		vi.unstubAllGlobals();
	});

	it('returns the raw 202 MFA-challenge Response without throwing', async () => {
		const challenge = { mfa_token: 'tok', methods: ['totp'] };
		const mockFetch = vi.fn().mockResolvedValue(new Response(JSON.stringify(challenge), { status: 202 }));
		vi.stubGlobal('fetch', mockFetch);

		const res = await loginRaw({ email: 'a@b.com', password: 'pw' });

		expect(res.status).toBe(202);
		expect(await res.json()).toEqual(challenge);
		expect(mockFetch).toHaveBeenCalledTimes(1);
		expect(String(mockFetch.mock.calls[0][0])).toContain('/auth/login');
	});

	it('returns the raw 200 Response on a successful login', async () => {
		const mockFetch = vi.fn().mockResolvedValue(new Response(JSON.stringify(sampleRefresh), { status: 200 }));
		vi.stubGlobal('fetch', mockFetch);

		const res = await loginRaw({ email: 'a@b.com', password: 'pw' });

		expect(res.status).toBe(200);
		const callOptions = mockFetch.mock.calls[0][1] as RequestInit;
		expect(callOptions.method).toBe('POST');
	});

	it('does not attempt token refresh on a 401 (login carries no auth)', async () => {
		vi.mocked(getAccessToken).mockReturnValue('some-token');
		const mockFetch = vi.fn().mockResolvedValue(new Response('Unauthorized', { status: 401 }));
		vi.stubGlobal('fetch', mockFetch);

		const res = await loginRaw({ email: 'a@b.com', password: 'bad' });

		expect(res.status).toBe(401);
		expect(mockFetch).toHaveBeenCalledTimes(1);
		expect(vi.mocked(setAccessToken)).not.toHaveBeenCalled();
	});
});

// ── apiGet ────────────────────────────────────────────────────────────────────
// Path-based GET routed through the configured client (reuses auth/refresh/ApiError).

describe('apiGet', () => {
	afterEach(() => {
		vi.restoreAllMocks();
	});

	it('returns the parsed JSON body and routes through the configured client', async () => {
		vi.mocked(getAccessToken).mockReturnValue(null);
		const payload = [{ id: 'team-1' }];
		const fetchSpy = vi
			.spyOn(globalThis, 'fetch')
			.mockResolvedValue(
				new Response(JSON.stringify(payload), { status: 200, headers: { 'Content-Type': 'application/json' } })
			);

		const data = await apiGet<unknown>('/api/v1/teams-a?page=1&per_page=1000');

		expect(data).toEqual(payload);
		// The configured client calls fetch(request: Request) with a single Request arg.
		const req = fetchSpy.mock.calls[0][0] as Request;
		expect(req.url).toContain('/api/v1/teams-a');
		expect(req.url).toContain('page=1&per_page=1000');
	});

	it('strips a leading /api/v1 base path before routing (no double prefix)', async () => {
		vi.mocked(getAccessToken).mockReturnValue(null);
		const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response('{}', { status: 200 }));

		await apiGet('/api/v1/surfaces/foo');

		const req = fetchSpy.mock.calls[0][0] as Request;
		expect(req.url).not.toContain('/api/v1/api/v1/');
		expect(req.url).toContain('/api/v1/surfaces/foo');
	});
});
