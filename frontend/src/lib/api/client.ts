// Configured hey-api client + cross-cutting interceptors.
// Each concern lives in its own helper so functions stay small (CodeScene).
//
// Interceptor map:
//   request  → applyBearerAuth + mergeTimeoutSignal + applyIfMatch
//   response → captureEtag + handle2faRedirect + mapToApiError
//   error    → (Task 5 seam: refresh-retry interceptor attaches here)

import { client } from './generated/client.gen';
import { getAccessToken, onTokenChange } from '../token-store.svelte';
import { extractApiError } from './errors';
import type { ResolvedRequestOptions } from './generated/client/types.gen';

// ── Base URL resolution ───────────────────────────────────────────────────────
// hey-api's generated client creates `new Request(url, init)` internally, which
// requires an absolute URL in undici (Node.js test env) and compliant browsers.
// Resolve relative env paths against `location.origin` so both production and
// jsdom test environments produce a valid absolute URL.

const BASE_ENV: string = import.meta.env.VITE_API_BASE ?? '/api/v1';

// Path portion only (used for ETag scope detection and stripping).
const BASE_PATH: string = (() => {
	if (BASE_ENV.startsWith('http://') || BASE_ENV.startsWith('https://')) {
		return new URL(BASE_ENV).pathname;
	}
	return BASE_ENV.startsWith('/') ? BASE_ENV : `/${BASE_ENV}`;
})();

// Full absolute URL for client.setConfig (must be absolute for new Request()).
const BASE: string = (() => {
	if (BASE_ENV.startsWith('http://') || BASE_ENV.startsWith('https://')) return BASE_ENV;
	const origin =
		typeof globalThis.location !== 'undefined' && globalThis.location.origin !== 'null' && globalThis.location.origin
			? globalThis.location.origin
			: '';
	return origin ? `${origin}${BASE_PATH}` : BASE_ENV;
})();

const DEFAULT_TIMEOUT_MS = 30_000;

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

async function handle2faRedirect(response: Response): Promise<void> {
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
// NOTE — Task 5 seam: 401 is intentionally NOT thrown here. It passes through to
// the client's built-in error path (throws raw JSON body). Task 5 attaches a
// `client.interceptors.error` handler that intercepts 401, performs the
// token-refresh retry, updates the session-expired banner, and retries the
// original request on success.

client.interceptors.response.use(async (response: Response, request: Request): Promise<Response> => {
	captureEtag(response, request.url);
	await handle2faRedirect(response);
	if (!response.ok && response.status !== 401) {
		throw await extractApiError(response.clone());
	}
	return response;
});

// ── Exports ───────────────────────────────────────────────────────────────────

export { client as apiClient };
