// Raw-Response escape hatch over the configured client.
//
// These three functions exist because some callers need the *raw* `Response`
// (loginRaw's 202 MFA challenge), an arbitrary full URL outside `/api/v1/`
// (authenticatedFetch, used by OAuth flows), or an ad-hoc path-based GET (apiGet).
// They are thin wrappers: all cross-cutting auth/refresh/2FA/timeout logic has ONE
// source of truth in `./client` — this module imports those primitives rather than
// re-implementing them.

import {
	apiClient,
	BASE,
	BASE_PATH,
	DEFAULT_TIMEOUT_MS,
	dedupedRefresh,
	handle2faRedirect,
	mapRefreshFailure
} from './client';
import { getAccessToken, setAccessToken, setSessionExpired } from '../token-store.svelte';
import type { LoginRequest } from './generated';

// Merge the default request timeout with any caller-supplied abort signal.
function withTimeout(callerSignal: AbortSignal | null | undefined): AbortSignal {
	const timeoutSignal = AbortSignal.timeout(DEFAULT_TIMEOUT_MS);
	return callerSignal ? AbortSignal.any([callerSignal, timeoutSignal]) : timeoutSignal;
}

// Build the request headers: JSON content type, optional Bearer, then caller overrides.
function authHeaders(options: RequestInit, token: string | null): Headers {
	const headers = new Headers({ 'Content-Type': 'application/json' });
	if (token) headers.set('Authorization', `Bearer ${token}`);
	if (options.headers !== undefined) {
		for (const [key, value] of new Headers(options.headers).entries()) headers.set(key, value);
	}
	return headers;
}

function rawFetch(url: string, options: RequestInit, token: string | null): Promise<Response> {
	return fetch(url, {
		credentials: 'same-origin',
		...options,
		headers: authHeaders(options, token),
		signal: withTimeout(options.signal)
	});
}

// 401 handler: surface the banner, share the in-flight refresh, then retry once with
// the rotated token. Refresh failures are mapped by `./client`'s single policy.
async function refreshAndRetry(url: string, options: RequestInit): Promise<Response> {
	setSessionExpired(true);
	let refreshed;
	try {
		refreshed = await dedupedRefresh();
	} catch (refreshErr) {
		throw mapRefreshFailure(refreshErr);
	}
	setAccessToken(refreshed.access_token);
	// The retry promise is returned (not awaited) so its rejection propagates to the
	// caller; `.finally` clears the banner once the retry settles, success or failure.
	return rawFetch(url, options, refreshed.access_token).finally(() => setSessionExpired(false));
}

/**
 * Performs an authenticated fetch with automatic token refresh on 401.
 * Returns the raw Response for callers to handle body parsing.
 *
 * The `url` parameter must be the complete request URL (e.g. `/api/v1/foo` or
 * `/oauth/consent/xyz`). Unlike the path-based client helpers, this function does
 * NOT prepend a base URL — callers are responsible for constructing the full path.
 */
export async function authenticatedFetch(url: string, options: RequestInit = {}): Promise<Response> {
	const res = await rawFetch(url, options, getAccessToken());
	if (res.status === 401 && getAccessToken()) {
		return refreshAndRetry(url, options);
	}
	await handle2faRedirect(res);
	return res;
}

/** Performs an authenticated GET routed through the configured client; returns parsed JSON. */
export async function apiGet<T = unknown>(path: string): Promise<T> {
	// The client re-prepends BASE, so strip a redundant base prefix to avoid doubling.
	const relativePath = path.startsWith(BASE)
		? path.slice(BASE.length)
		: path.startsWith(BASE_PATH)
			? path.slice(BASE_PATH.length)
			: path;
	const { data } = await apiClient.get({ url: relativePath });
	return data as T;
}

/**
 * POST /auth/login — returns the raw Response so callers can inspect the 202 MFA
 * challenge. Login carries no auth and needs no refresh; it shares only BASE and the
 * request timeout with the configured client.
 */
export function loginRaw(data: LoginRequest): Promise<Response> {
	return fetch(`${BASE}/auth/login`, {
		method: 'POST',
		headers: { 'Content-Type': 'application/json' },
		credentials: 'same-origin',
		body: JSON.stringify(data),
		signal: AbortSignal.timeout(DEFAULT_TIMEOUT_MS)
	});
}
