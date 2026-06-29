// Configured hey-api client + cross-cutting interceptors.
// Each concern lives in its own helper so functions stay small (CodeScene).
//
// Interceptor map:
//   request  → applyBearerAuth + mergeTimeoutSignal + applyIfMatch
//   response → captureEtag + handle2faRedirect + mapToApiError
// The exported `apiClient` additionally wraps every call in `requestWithRefresh`
// (Task 5) for the 401 deduped refresh-retry — see the "401 refresh-retry" section.

import { client } from './generated/client.gen';
import { getAccessToken, setAccessToken, setSessionExpired, onTokenChange } from '../token-store.svelte';
import { extractApiError } from './errors';
import type { Client, ResolvedRequestOptions } from './generated/client/types.gen';

// ── Base URL resolution ───────────────────────────────────────────────────────
// hey-api's generated client creates `new Request(url, init)` internally, which
// requires an absolute URL in undici (Node.js test env) and compliant browsers.
// Resolve relative env paths against `location.origin` so both production and
// jsdom test environments produce a valid absolute URL.

const BASE_ENV: string = import.meta.env.VITE_API_BASE ?? '/api/v1';

// Path portion only (used for ETag scope detection and stripping). Exported so
// api/raw.ts's path-based apiGet can strip a redundant base prefix before routing
// through the configured client (which re-prepends BASE).
export const BASE_PATH: string = (() => {
	if (BASE_ENV.startsWith('http://') || BASE_ENV.startsWith('https://')) {
		return new URL(BASE_ENV).pathname;
	}
	return BASE_ENV.startsWith('/') ? BASE_ENV : `/${BASE_ENV}`;
})();

// Full absolute URL for client.setConfig (must be absolute for new Request()).
// Exported so the raw-Response escape hatch (api/raw.ts) shares the same origin
// resolution for unauthenticated calls (e.g. loginRaw → `${BASE}/auth/login`).
export const BASE: string = (() => {
	if (BASE_ENV.startsWith('http://') || BASE_ENV.startsWith('https://')) return BASE_ENV;
	const origin =
		typeof globalThis.location !== 'undefined' && globalThis.location.origin !== 'null' && globalThis.location.origin
			? globalThis.location.origin
			: '';
	return origin ? `${origin}${BASE_PATH}` : BASE_ENV;
})();

// Exported so api/raw.ts applies the identical request timeout to its raw fetches.
export const DEFAULT_TIMEOUT_MS = 30_000;
const REFRESH_TIMEOUT_MS = 10_000;

// ── Settings ETag auto-cache ──────────────────────────────────────────────────
// The backend `etag_middleware` requires `If-Match` on every PUT/PATCH for
// `/global-settings/*` and `/settings/*`. We cache the most recently observed
// ETag per scope and auto-attach it so callers don't have to plumb the value.
// The cache is wiped when the authenticated subject (JWT `sub` claim) changes —
// silent token refreshes preserve it; cross-user sessions do not.

type SettingsScope = 'global' | 'tenant';

const settingsEtagCache: Record<SettingsScope, string | null> = {
	global: null,
	tenant: null
};

function settingsScope(path: string): SettingsScope | null {
	if (path.startsWith('/global-settings/')) return 'global';
	if (path.startsWith('/settings/')) return 'tenant';
	return null;
}

// `Request.url` is always absolute. Strip BASE_PATH from the pathname to recover
// the API path (e.g. 'http://host/api/v1/settings/foo' → '/settings/foo').
function apiPath(requestUrl: string): string {
	const u = new URL(requestUrl, globalThis.location?.origin ?? 'http://localhost');
	return u.pathname.startsWith(BASE_PATH) ? u.pathname.slice(BASE_PATH.length) : u.pathname;
}

function subClaim(token: string | null): string | null {
	if (!token) return null;
	const parts = token.split('.');
	if (parts.length < 2) return null;
	try {
		let b64 = parts[1].replace(/-/g, '+').replace(/_/g, '/');
		b64 += '='.repeat((4 - (b64.length % 4)) % 4);
		const payload = JSON.parse(atob(b64)) as Record<string, unknown>;
		return typeof payload.sub === 'string' ? payload.sub : null;
	} catch {
		return null;
	}
}

/** Test-only: clears the scope ETag cache. Do not call from production code. */
export function _resetSettingsEtagCacheForTests(): void {
	settingsEtagCache.global = null;
	settingsEtagCache.tenant = null;
}

// Wipe ETag cache when the authenticated user changes (cross-user sessions).
onTokenChange((prev, next) => {
	if (subClaim(prev) !== subClaim(next)) {
		settingsEtagCache.global = null;
		settingsEtagCache.tenant = null;
	}
});

// ── Client base config ────────────────────────────────────────────────────────
// setConfig MERGES into existing config, so the codegen-baked `throwOnError: true`
// is preserved. Re-stating it here is the runtime switch that makes the client throw.

client.setConfig({ baseUrl: BASE, throwOnError: true });

// ── Request helpers ───────────────────────────────────────────────────────────

function applyBearerAuth(request: Request): void {
	const token = getAccessToken();
	if (token) request.headers.set('Authorization', `Bearer ${token}`);
}

// Merges AbortSignal.timeout with any caller-supplied signal (ported from api.ts:272-287).
// Must create a new Request because Request.signal is readonly.
function mergeTimeoutSignal(request: Request, callerSignal: AbortSignal | null | undefined): Request {
	const timeoutSignal = AbortSignal.timeout(DEFAULT_TIMEOUT_MS);
	const signal = callerSignal ? AbortSignal.any([callerSignal, timeoutSignal]) : timeoutSignal;
	return new Request(request, { signal });
}

function applyIfMatch(request: Request): void {
	const scope = settingsScope(apiPath(request.url));
	if (!scope || (request.method !== 'PUT' && request.method !== 'PATCH')) return;
	if (request.headers.has('if-match')) return;
	const cached = settingsEtagCache[scope];
	if (cached) request.headers.set('If-Match', cached);
}

// ── Request interceptor ───────────────────────────────────────────────────────

client.interceptors.request.use((request: Request, options: ResolvedRequestOptions): Request => {
	applyBearerAuth(request);
	const req = mergeTimeoutSignal(request, options.signal);
	applyIfMatch(req);
	return req;
});

// ── Response helpers ──────────────────────────────────────────────────────────

function captureEtag(response: Response, requestUrl: string): void {
	if (!response.ok) return;
	const scope = settingsScope(apiPath(requestUrl));
	if (!scope) return;
	const etag = response.headers.get('etag');
	if (etag) settingsEtagCache[scope] = etag;
}

// Exported so the raw-fetch escape hatch (api/raw.ts) reuses the SAME 2FA-redirect
// rule rather than re-implementing the 403 → /profile#security branch.
export async function handle2faRedirect(response: Response): Promise<void> {
	if (response.status !== 403) return;
	try {
		const body = (await response.clone().json()) as Record<string, unknown>;
		if (body?.error === '2fa_setup_required' && typeof window !== 'undefined') {
			window.location.href = '/profile#security';
		}
	} catch {
		// Not JSON or no matching error field — fall through to ApiError mapping.
	}
}

// ── Response interceptor ──────────────────────────────────────────────────────
// NOTE — Task 5 seam: 401 is intentionally NOT mapped to ApiError here. It passes
// through to the client's built-in error path. The 401 refresh-retry is handled by
// the `requestWithRefresh` wrapper below (see the "401 refresh-retry" section) — an
// error interceptor cannot work, because its return value is always re-thrown under
// `throwOnError` and so can never convert a 401 into a successful retried response.

client.interceptors.response.use(async (response: Response, request: Request): Promise<Response> => {
	captureEtag(response, request.url);
	await handle2faRedirect(response);
	if (!response.ok && response.status !== 401) {
		throw await extractApiError(response.clone());
	}
	return response;
});

// ── 401 refresh-retry (Task 5, spike S-A) ───────────────────────────────────────
// This is the ONLY auth concern that lives outside the interceptors: a 401 surfaces
// from the response interceptor seam as a *thrown* value (not a Response), and the
// retry body must be rebuilt from the call OPTIONS (the consumed Request.body stream
// is gone). The hey-api error interceptor cannot help — its return value is always
// re-thrown under `throwOnError`, so it can never convert a 401 into a successful
// retry. Instead we wrap `client.request`, re-issuing the SAME call with
// `throwOnError: false` so we can inspect `response.status` directly, then re-throw
// the interceptor-mapped error to preserve the public `throwOnError: true` contract.
// ETag, 2FA, session-banner wiring and ApiError mapping all stay in the interceptors.

/** Minimal shape consumed from the refresh endpoint; backend returns more fields. */
interface RefreshResult {
	access_token: string;
}

/**
 * Thrown when `/auth/refresh` returns a non-OK response. Carries the HTTP status so
 * the failure mapper can separate real auth failures (4xx) from transient 5xx.
 */
class RefreshError extends Error {
	public readonly status: number;
	constructor(status: number) {
		super(`Refresh failed (${status})`);
		this.name = 'RefreshError';
		this.status = status;
	}
}

// Ported from api.ts:246-260 — same endpoint, timeout and credentials semantics.
async function refreshAccessToken(): Promise<RefreshResult> {
	const res = await fetch(`${BASE}/auth/refresh`, {
		method: 'POST',
		headers: { 'Content-Type': 'application/json' },
		credentials: 'same-origin',
		body: JSON.stringify({}),
		signal: AbortSignal.timeout(REFRESH_TIMEOUT_MS)
	});
	if (!res.ok) throw new RefreshError(res.status);
	// Validate the shape at runtime: a malformed 200 (missing access_token) must not
	// silently become setAccessToken(undefined). Treat it as a refresh failure.
	const data: unknown = await res.json();
	if (typeof data !== 'object' || data === null || typeof (data as RefreshResult).access_token !== 'string') {
		throw new RefreshError(res.status);
	}
	return data as RefreshResult;
}

// Deduped refresh (ported from api.ts:294-307): concurrent 401s share one in-flight
// refresh; the promise is cleared once it settles so the next cycle starts fresh.
let refreshPromise: Promise<RefreshResult> | null = null;

// Exported so api/raw.ts's authenticatedFetch shares ONE in-flight refresh with the
// configured client — concurrent 401s across both paths collapse to a single refresh.
export function dedupedRefresh(): Promise<RefreshResult> {
	if (!refreshPromise) {
		refreshPromise = refreshAccessToken();
		refreshPromise.then(
			() => {
				refreshPromise = null;
			},
			() => {
				refreshPromise = null;
			}
		);
	}
	return refreshPromise;
}

function isTimeoutOrAbort(err: unknown): boolean {
	return err instanceof DOMException && (err.name === 'TimeoutError' || err.name === 'AbortError');
}

// Ported from api.ts:338-360. Maps a refresh failure to a user-facing Error and
// performs the matching session side effect. The default (real 4xx auth failure)
// clears the token and leaves the session-expired banner raised.
// Exported so api/raw.ts maps refresh failures to the SAME user-facing errors and
// session side effects (single source of truth for the refresh-failure policy).
export function mapRefreshFailure(refreshErr: unknown): Error {
	if (isTimeoutOrAbort(refreshErr)) {
		setSessionExpired(false);
		return new Error('Token refresh timed out. Please try again.');
	}
	if (refreshErr instanceof TypeError) {
		setSessionExpired(false);
		return new Error('Network error during token refresh. Check your connection.');
	}
	if (refreshErr instanceof RefreshError && refreshErr.status >= 500) {
		setSessionExpired(false);
		return new Error('Server error during token refresh. Please try again later.');
	}
	setAccessToken(null);
	return new Error('Session expired. Please log in again.');
}

// Result shape of `client.request` under responseStyle 'fields' + throwOnError false.
type FieldsResult = { data?: unknown; error?: unknown; request?: Request; response?: Response };
type RequestArgs = Parameters<Client['request']>[0];

/** Re-throws the interceptor-mapped error, or returns the success fields object. */
function unwrap(result: FieldsResult): unknown {
	if (result.error !== undefined) throw result.error;
	return result;
}

async function refreshAndRetry(options: RequestArgs): Promise<unknown> {
	setSessionExpired(true);
	let refreshed: RefreshResult;
	try {
		refreshed = await dedupedRefresh();
	} catch (refreshErr) {
		throw mapRefreshFailure(refreshErr);
	}
	setAccessToken(refreshed.access_token);
	try {
		// Single retry. The request interceptor re-applies the new Bearer; the client
		// rebuilds the body from options.body, so the consumed stream is irrelevant.
		return unwrap((await client.request({ ...options, throwOnError: false })) as FieldsResult);
	} finally {
		setSessionExpired(false);
	}
}

async function requestWithRefresh(options: RequestArgs): Promise<unknown> {
	const first = (await client.request({ ...options, throwOnError: false })) as FieldsResult;
	if (first.response?.status === 401 && getAccessToken()) {
		return refreshAndRetry(options);
	}
	return unwrap(first);
}

// ── Exports ───────────────────────────────────────────────────────────────────
// `apiClient` is the configured client with refresh-retry layered over every HTTP
// method (and `request`). A Proxy forwards method calls through `requestWithRefresh`
// while leaving non-call members (interceptors, setConfig, buildUrl, sse, …) intact.

const HTTP_METHOD_VERBS: Readonly<Record<string, string>> = {
	connect: 'CONNECT',
	delete: 'DELETE',
	get: 'GET',
	head: 'HEAD',
	options: 'OPTIONS',
	patch: 'PATCH',
	post: 'POST',
	put: 'PUT',
	trace: 'TRACE'
};

const apiClient: Client = new Proxy(client, {
	get(target, prop, receiver) {
		if (prop === 'request') return requestWithRefresh;
		const verb = typeof prop === 'string' ? HTTP_METHOD_VERBS[prop] : undefined;
		if (verb)
			return (options: Omit<RequestArgs, 'method'>) => requestWithRefresh({ ...options, method: verb } as RequestArgs);
		return Reflect.get(target, prop, receiver);
	}
});

export { apiClient };
