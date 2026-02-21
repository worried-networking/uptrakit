import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { extractErrorMessage, me } from './api';
import { getAccessToken, setAccessToken } from './auth.svelte';
import type { RefreshResponse, User } from './types';

// Mock ./auth.svelte to avoid Svelte rune initialization in test environment
vi.mock('./auth.svelte', () => ({
	getAccessToken: vi.fn().mockReturnValue(null),
	setAccessToken: vi.fn()
}));

const sampleUser: User = {
	id: 'user-1',
	email: 'user@example.com',
	first_name: 'Test',
	last_name: 'User',
	permissions: []
};

const sampleRefresh: RefreshResponse = {
	access_token: 'new-token',
	refresh_token: 'refresh-token',
	expires_in: 3600,
	token_type: 'Bearer'
};

// ── extractErrorMessage ───────────────────────────────────────────────────────

describe('extractErrorMessage', () => {
	it('extracts the error field from a JSON body', async () => {
		const res = new Response(JSON.stringify({ error: 'Not found' }), {
			status: 404,
			statusText: 'Not Found'
		});
		const msg = await extractErrorMessage(res);
		expect(msg).toBe('Not found');
	});

	it('returns the full JSON string when no error field is present', async () => {
		const body = JSON.stringify({ message: 'something went wrong' });
		const res = new Response(body, { status: 500, statusText: 'Internal Server Error' });
		const msg = await extractErrorMessage(res);
		expect(msg).toBe(body);
	});

	it('returns plain text body for non-JSON responses', async () => {
		const res = new Response('Internal Server Error', {
			status: 500,
			statusText: 'Internal Server Error'
		});
		const msg = await extractErrorMessage(res);
		expect(msg).toBe('Internal Server Error');
	});

	it('returns statusText when body is empty', async () => {
		const res = new Response('', { status: 401, statusText: 'Unauthorized' });
		const msg = await extractErrorMessage(res);
		expect(msg).toBe('Unauthorized');
	});

	it('ignores non-string error fields in JSON', async () => {
		const body = JSON.stringify({ error: 42 });
		const res = new Response(body, { status: 400, statusText: 'Bad Request' });
		const msg = await extractErrorMessage(res);
		// error is not a string, so falls back to the full JSON body
		expect(msg).toBe(body);
	});

	it('truncates messages longer than 500 characters', async () => {
		const longMessage = 'a'.repeat(600);
		const res = new Response(longMessage, { status: 500, statusText: 'Internal Server Error' });
		const msg = await extractErrorMessage(res);
		expect(msg).toHaveLength(501); // 500 chars + ellipsis character
		expect(msg.endsWith('\u2026')).toBe(true);
		expect(msg.startsWith('a'.repeat(500))).toBe(true);
	});

	it('truncates JSON error field longer than 500 characters', async () => {
		const longError = 'e'.repeat(600);
		const res = new Response(JSON.stringify({ error: longError }), {
			status: 400,
			statusText: 'Bad Request'
		});
		const msg = await extractErrorMessage(res);
		expect(msg).toHaveLength(501);
		expect(msg.endsWith('\u2026')).toBe(true);
	});

	it('does not truncate messages of exactly 500 characters', async () => {
		const exactMessage = 'b'.repeat(500);
		const res = new Response(exactMessage, { status: 500, statusText: 'Internal Server Error' });
		const msg = await extractErrorMessage(res);
		expect(msg).toBe(exactMessage);
	});
});

// ── authenticatedFetch ────────────────────────────────────────────────────────
// Tested indirectly via the exported `me()` function, which calls
// request('/auth/me') → authenticatedFetch → fetch.

describe('authenticatedFetch', () => {
	beforeEach(() => {
		vi.mocked(getAccessToken).mockReturnValue(null);
		vi.mocked(setAccessToken).mockReset();
	});

	afterEach(() => {
		vi.unstubAllGlobals();
	});

	it('includes Authorization header when a token is set', async () => {
		vi.mocked(getAccessToken).mockReturnValue('my-token');
		const mockFetch = vi.fn().mockResolvedValue(new Response(JSON.stringify(sampleUser), { status: 200 }));
		vi.stubGlobal('fetch', mockFetch);

		await me();

		expect(mockFetch).toHaveBeenCalledTimes(1);
		const callOptions = mockFetch.mock.calls[0][1] as RequestInit;
		const headers = callOptions.headers as Record<string, string>;
		expect(headers['Authorization']).toBe('Bearer my-token');
	});

	it('does not include Authorization header when no token is set', async () => {
		vi.mocked(getAccessToken).mockReturnValue(null);
		const mockFetch = vi.fn().mockResolvedValue(new Response(JSON.stringify(sampleUser), { status: 200 }));
		vi.stubGlobal('fetch', mockFetch);

		await me();

		expect(mockFetch).toHaveBeenCalledTimes(1);
		const callOptions = mockFetch.mock.calls[0][1] as RequestInit;
		const headers = callOptions.headers as Record<string, string>;
		expect(headers['Authorization']).toBeUndefined();
	});

	it('retries with new token after a 401 (3 fetch calls total)', async () => {
		vi.mocked(getAccessToken).mockReturnValue('old-token');
		const mockFetch = vi
			.fn()
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockResolvedValueOnce(new Response(JSON.stringify(sampleRefresh), { status: 200 }))
			.mockResolvedValueOnce(new Response(JSON.stringify(sampleUser), { status: 200 }));
		vi.stubGlobal('fetch', mockFetch);

		const result = await me();

		expect(mockFetch).toHaveBeenCalledTimes(3);
		expect(vi.mocked(setAccessToken)).toHaveBeenCalledWith('new-token');
		// Retry uses the new token in Authorization header
		const retryOptions = mockFetch.mock.calls[2][1] as RequestInit;
		const retryHeaders = retryOptions.headers as Record<string, string>;
		expect(retryHeaders['Authorization']).toBe('Bearer new-token');
		expect(result).toEqual(sampleUser);
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

		await Promise.all([me(), me()]);

		// 2 original requests + 1 shared refresh + 2 retries = 5
		expect(mockFetch).toHaveBeenCalledTimes(5);
		// Verify that only 1 call went to the refresh endpoint
		const refreshCalls = mockFetch.mock.calls.filter((args: unknown[]) =>
			(args[0] as string).includes('/auth/refresh')
		);
		expect(refreshCalls).toHaveLength(1);
		// setAccessToken should be called twice (once per concurrent request receiving the result)
		expect(vi.mocked(setAccessToken)).toHaveBeenCalledWith('new-token');
	});

	it('clears token and redirects on 401 with 4xx refresh failure', async () => {
		vi.mocked(getAccessToken).mockReturnValue('old-token');
		const mockFetch = vi
			.fn()
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockResolvedValueOnce(new Response('Forbidden', { status: 403, statusText: 'Forbidden' }));
		vi.stubGlobal('fetch', mockFetch);

		// Stub window.location.href to capture the redirect
		let capturedHref = '';
		Object.defineProperty(window, 'location', {
			value: {
				...window.location,
				set href(v: string) {
					capturedHref = v;
				}
			},
			writable: true,
			configurable: true
		});

		await expect(me()).rejects.toThrow('Session expired');

		expect(vi.mocked(setAccessToken)).toHaveBeenCalledWith(null);
		expect(capturedHref).toMatch(/^\/login\?redirect=/);
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

		await expect(me()).rejects.toThrow('Server error during token refresh');

		// Token must NOT be cleared for transient server errors
		expect(vi.mocked(setAccessToken)).not.toHaveBeenCalledWith(null);
	});

	it('does not attempt refresh when no token is set and 401 is returned', async () => {
		vi.mocked(getAccessToken).mockReturnValue(null);
		const mockFetch = vi
			.fn()
			.mockResolvedValue(
				new Response(JSON.stringify({ error: 'Unauthorized' }), { status: 401, statusText: 'Unauthorized' })
			);
		vi.stubGlobal('fetch', mockFetch);

		await expect(me()).rejects.toThrow('Unauthorized');

		// Only the original request — no refresh attempt
		expect(mockFetch).toHaveBeenCalledTimes(1);
		expect(vi.mocked(setAccessToken)).not.toHaveBeenCalled();
	});

	it('throws a timeout error when fetch is aborted by timeout signal', async () => {
		vi.mocked(getAccessToken).mockReturnValue(null);
		const timeoutError = new DOMException('signal timed out', 'TimeoutError');
		const mockFetch = vi.fn().mockRejectedValue(timeoutError);
		vi.stubGlobal('fetch', mockFetch);

		await expect(me()).rejects.toThrow('Request timed out');
	});
});
