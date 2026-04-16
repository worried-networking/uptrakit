import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { extractErrorMessage, getSurfaceRuntimeStatus, me, sealedBoxEncrypt } from './api';
import { getAccessToken, setAccessToken, setSessionExpired } from './token-store.svelte';
import type { RefreshResponse, User } from './types';

// Mock ./token-store.svelte (where api.ts now imports token state from)
vi.mock('./token-store.svelte', () => ({
	getAccessToken: vi.fn().mockReturnValue(null),
	setAccessToken: vi.fn(),
	getSessionExpired: vi.fn().mockReturnValue(false),
	setSessionExpired: vi.fn()
}));

// Mock ./auth.svelte to avoid Svelte rune initialization in test environment
vi.mock('./auth.svelte', () => ({
	getAccessToken: vi.fn().mockReturnValue(null),
	setAccessToken: vi.fn(),
	setSessionExpired: vi.fn()
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
		vi.mocked(setSessionExpired).mockReset();
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

	it('clears token and sets sessionExpired on 401 with 4xx refresh failure (no hard redirect)', async () => {
		vi.mocked(getAccessToken).mockReturnValue('old-token');
		const mockFetch = vi
			.fn()
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockResolvedValueOnce(new Response('Forbidden', { status: 403, statusText: 'Forbidden' }));
		vi.stubGlobal('fetch', mockFetch);

		// Track whether window.location.href is assigned
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

		await expect(me()).rejects.toThrow('Session expired. Please log in again.');

		expect(vi.mocked(setAccessToken)).toHaveBeenCalledWith(null);
		expect(vi.mocked(setSessionExpired)).toHaveBeenCalledWith(true);
		// No hard redirect — the banner handles navigation
		expect(hrefAssigned).toBe(false);

		// Restore location descriptor
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

	// ── session-expired banner lifecycle ─────────────────────────────────────

	it('sets sessionExpired true immediately and clears it after successful refresh and retry', async () => {
		vi.mocked(getAccessToken).mockReturnValue('old-token');
		const mockFetch = vi
			.fn()
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockResolvedValueOnce(new Response(JSON.stringify(sampleRefresh), { status: 200 }))
			.mockResolvedValueOnce(new Response(JSON.stringify(sampleUser), { status: 200 }));
		vi.stubGlobal('fetch', mockFetch);

		const result = await me();

		const calls = vi.mocked(setSessionExpired).mock.calls.map((c) => c[0]);
		expect(calls).toContain(true);
		expect(calls[calls.length - 1]).toBe(false);
		expect(result).toEqual(sampleUser);
	});

	it('clears sessionExpired via finally even when retry fetch rejects', async () => {
		vi.mocked(getAccessToken).mockReturnValue('old-token');
		const mockFetch = vi
			.fn()
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockResolvedValueOnce(new Response(JSON.stringify(sampleRefresh), { status: 200 }))
			.mockRejectedValueOnce(new TypeError('Failed to fetch'));
		vi.stubGlobal('fetch', mockFetch);

		await expect(me()).rejects.toThrow('Network error: Unable to connect to the server.');

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

		await expect(me()).rejects.toThrow('Session expired. Please log in again.');

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

		await expect(me()).rejects.toThrow('Network error during token refresh. Check your connection.');

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

		await expect(me()).rejects.toThrow('Server error during token refresh. Please try again later.');

		const calls = vi.mocked(setSessionExpired).mock.calls.map((c) => c[0]);
		expect(calls).toEqual([true, false]);
	});

	it('clears sessionExpired when refresh times out with DOMException', async () => {
		vi.mocked(getAccessToken).mockReturnValue('old-token');
		const mockFetch = vi
			.fn()
			.mockResolvedValueOnce(new Response('', { status: 401, statusText: 'Unauthorized' }))
			.mockRejectedValueOnce(new DOMException('signal timed out', 'TimeoutError'));
		vi.stubGlobal('fetch', mockFetch);

		await expect(me()).rejects.toThrow('Token refresh timed out. Please try again.');

		const calls = vi.mocked(setSessionExpired).mock.calls.map((c) => c[0]);
		expect(calls).toEqual([true, false]);
	});

	it('does not call setSessionExpired when 401 is returned without an access token', async () => {
		vi.mocked(getAccessToken).mockReturnValue(null);
		const mockFetch = vi
			.fn()
			.mockResolvedValueOnce(
				new Response(JSON.stringify({ error: 'Unauthorized' }), { status: 401, statusText: 'Unauthorized' })
			);
		vi.stubGlobal('fetch', mockFetch);

		await expect(me()).rejects.toThrow('Unauthorized');

		expect(mockFetch).toHaveBeenCalledTimes(1);
		expect(vi.mocked(setSessionExpired)).not.toHaveBeenCalled();
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

		const results = await Promise.all([me(), me()]);

		const calls = vi.mocked(setSessionExpired).mock.calls.map((c) => c[0]);
		const trueCalls = calls.filter((v) => v === true);
		const falseCalls = calls.filter((v) => v === false);
		expect(trueCalls.length).toBeGreaterThanOrEqual(2);
		expect(falseCalls.length).toBeGreaterThanOrEqual(2);
		expect(calls[calls.length - 1]).toBe(false);
		expect(results[0]).toEqual(sampleUser);
		expect(results[1]).toEqual(sampleUser);
	});
});

describe('getSurfaceRuntimeStatus', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
	});

	it('reads surface runtime status from the surfaces endpoint', async () => {
		const mockFetch = vi.fn().mockResolvedValue(
			new Response(JSON.stringify({ active: true }), {
				status: 200,
				headers: { 'Content-Type': 'application/json' }
			})
		);
		vi.stubGlobal('fetch', mockFetch);

		const status = await getSurfaceRuntimeStatus();

		expect(status.active).toBe(true);
		expect(mockFetch).toHaveBeenCalledTimes(1);
		expect(mockFetch.mock.calls[0][0]).toContain('/surfaces/runtime-status');
	});

	it('throws the server message when runtime status endpoint fails', async () => {
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

// ── sealedBoxEncrypt ──────────────────────────────────────────────────────────

/** Generate a fresh P-256 key pair and return the public key as standard base64. */
async function generateTestPublicKeyBase64(): Promise<string> {
	const keyPair = await crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, false, ['deriveBits']);
	const raw = new Uint8Array(await crypto.subtle.exportKey('raw', keyPair.publicKey));
	let binary = '';
	for (const byte of raw) binary += String.fromCharCode(byte);
	return btoa(binary);
}

describe('sealedBoxEncrypt', () => {
	it('returns a non-empty base64 string', async () => {
		const pubKey = await generateTestPublicKeyBase64();
		const result = await sealedBoxEncrypt('hello', pubKey);
		expect(typeof result).toBe('string');
		expect(result.length).toBeGreaterThan(0);
		// Standard base64 character set (may include padding).
		expect(result).toMatch(/^[A-Za-z0-9+/]+=*$/);
	});

	it('produces a sealed box of the correct minimum binary length', async () => {
		const pubKey = await generateTestPublicKeyBase64();
		const plaintext = 'test message';
		const result = await sealedBoxEncrypt(plaintext, pubKey);
		// Decode and verify: 65 (ephemeral pubkey) + 12 (nonce) + plaintext.length + 16 (GCM tag).
		const decoded = atob(result);
		const minLen = 65 + 12 + new TextEncoder().encode(plaintext).length + 16;
		expect(decoded.length).toBe(minLen);
	});

	it('produces different ciphertext on each call (fresh ephemeral keypair)', async () => {
		const pubKey = await generateTestPublicKeyBase64();
		const result1 = await sealedBoxEncrypt('same plaintext', pubKey);
		const result2 = await sealedBoxEncrypt('same plaintext', pubKey);
		expect(result1).not.toBe(result2);
	});

	it('encodes the ephemeral uncompressed P-256 public key as the first 65 bytes (starts with 0x04)', async () => {
		const pubKey = await generateTestPublicKeyBase64();
		const result = await sealedBoxEncrypt('hello', pubKey);
		const decoded = atob(result);
		// Uncompressed P-256 points start with the 0x04 prefix byte.
		expect(decoded.charCodeAt(0)).toBe(0x04);
	});

	it('throws for invalid base64 input', async () => {
		await expect(sealedBoxEncrypt('hello', '!!not-valid-base64!!')).rejects.toThrow();
	});

	it('throws when the decoded public key is not a valid P-256 point', async () => {
		// 65 bytes of zeros is not a valid uncompressed P-256 point.
		const invalidKey = btoa(String.fromCharCode(...new Array(65).fill(0)));
		await expect(sealedBoxEncrypt('hello', invalidKey)).rejects.toThrow();
	});

	it('works with an empty plaintext', async () => {
		const pubKey = await generateTestPublicKeyBase64();
		const result = await sealedBoxEncrypt('', pubKey);
		// 65 + 12 + 0 + 16 = 93 bytes.
		const decoded = atob(result);
		expect(decoded.length).toBe(93);
	});
});
