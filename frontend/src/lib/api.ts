// Configure the client + interceptors as a side-effect on first import.
import './api/client';
// Named imports provide local bindings so internal uses of ApiError and extractErrorMessage
// (after their local definitions are removed below) continue to resolve correctly.
import { ApiError, extractErrorMessage } from './api/errors';
import { apiClient } from './api/client';
// Raw-Response escape hatch lives in api/raw.ts (routes through the configured client).
// Imported for internal use by request()/requestVoid() and re-exported via the barrel.
import { apiGet, authenticatedFetch, loginRaw } from './api/raw';

// Generated SDK + types become reachable via `$lib/api` during migration.
// Both the old hand-written functions (below) and the generated SDK names are
// exported simultaneously so call sites can migrate domain-by-domain (Tasks 7-12).
export * from './api/generated';
export { ApiError, extractErrorMessage };
export { extractApiError } from './api/errors';
export { apiClient };
export { apiGet, authenticatedFetch, loginRaw };
export { executeBatchChunked } from './api/batch';
export { listSurfaces, listSurfaceProviders, getSurfaceRead, invokeSurfaceInteraction } from './api/surfaces';

import { onTokenChange } from './token-store.svelte';
import type {
	AuditLogEntry,
	AuditLogListParams,
	NotificationChannelSummary,
	NotificationRuleResponse,
	NotificationLogEntry,
	PaginatedResponse,
	RefreshResponse
} from './types';

const BASE: string = import.meta.env.VITE_API_BASE || '/api/v1';
const REFRESH_TIMEOUT_MS = 10_000;
const MAX_ERROR_LENGTH = 500;

// ── Settings ETag auto-cache ──────────────────────────────────────────
// The backend `etag_middleware` requires `If-Match` on every PUT/PATCH for
// `/global-settings/*` and `/settings/*`. We cache the most recently observed
// `ETag` per scope and auto-attach it to the next mutating request so callers
// don't have to plumb the value by hand. The cache is wiped when the
// authenticated subject (`sub` JWT claim) changes — silent token refreshes
// preserve the cache; cross-user sessions do not.

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

function subClaim(token: string | null): string | null {
	if (!token) return null;
	const parts = token.split('.');
	if (parts.length < 2) return null;
	try {
		let b64 = parts[1].replace(/-/g, '+').replace(/_/g, '/');
		b64 += '='.repeat((4 - (b64.length % 4)) % 4);
		const payload = JSON.parse(atob(b64));
		return typeof payload.sub === 'string' ? payload.sub : null;
	} catch {
		return null;
	}
}

function withHeader(init: HeadersInit | undefined, name: string, value: string): Headers {
	const h = new Headers(init);
	h.set(name, value);
	return h;
}

/** Test-only: clears the scope ETag cache. Do not call from production code. */
export function _resetSettingsEtagCacheForTests(): void {
	settingsEtagCache.global = null;
	settingsEtagCache.tenant = null;
}

onTokenChange((prev, next) => {
	if (subClaim(prev) !== subClaim(next)) {
		settingsEtagCache.global = null;
		settingsEtagCache.tenant = null;
	}
});

function truncateError(msg: string): string {
	return msg.length > MAX_ERROR_LENGTH ? msg.slice(0, MAX_ERROR_LENGTH) + '\u2026' : msg;
}

async function extractApiError(res: Response): Promise<ApiError> {
	const text = await res.text();
	let message: string = res.statusText;
	let errorCode: string | null = null;
	if (text) {
		try {
			const parsed = JSON.parse(text);
			if (typeof parsed === 'object' && parsed !== null) {
				if (typeof parsed.error === 'string') {
					message = truncateError(parsed.error);
				}
				if (typeof parsed.error_code === 'string') {
					errorCode = parsed.error_code;
				}
			}
		} catch {
			message = truncateError(text);
		}
	}
	return new ApiError(message, res.status, errorCode);
}

/**
 * Error thrown when the token refresh endpoint returns a non-OK response.
 * Carries the HTTP status so callers can distinguish real auth failures (4xx)
 * from transient server errors (5xx).
 */
class RefreshError extends Error {
	public readonly status: number;

	constructor(status: number) {
		super(`Refresh failed (${status})`);
		this.name = 'RefreshError';
		this.status = status;
	}
}

export async function refreshAccessToken(): Promise<RefreshResponse> {
	const res = await fetch(`${BASE}/auth/refresh`, {
		method: 'POST',
		headers: { 'Content-Type': 'application/json' },
		credentials: 'same-origin',
		body: JSON.stringify({}),
		signal: AbortSignal.timeout(REFRESH_TIMEOUT_MS)
	});

	if (!res.ok) {
		throw new RefreshError(res.status);
	}

	return res.json();
}

/** Performs an authenticated request and parses the JSON response body. */
export async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
	const scope = settingsScope(path);
	const method = (options.method ?? 'GET').toUpperCase();

	if (scope !== null && (method === 'PUT' || method === 'PATCH')) {
		const callerHas = new Headers(options.headers ?? {}).has('if-match');
		const cached = settingsEtagCache[scope];
		if (!callerHas && cached !== null) {
			options = {
				...options,
				headers: withHeader(options.headers, 'if-match', cached)
			};
		}
	}

	let res: Response;
	try {
		res = await authenticatedFetch(`${BASE}${path}`, options);
	} catch (err) {
		if (err instanceof DOMException && (err.name === 'AbortError' || err.name === 'TimeoutError')) {
			throw new Error('Request timed out. Please try again.');
		} else if (err instanceof TypeError) {
			throw new Error('Network error: Unable to connect to the server. Check your network connection.');
		}
		throw err;
	}

	if (scope !== null && res.ok) {
		const etag = res.headers.get('etag');
		if (etag !== null) settingsEtagCache[scope] = etag;
	}

	if (!res.ok) {
		throw await extractApiError(res);
	}
	return res.json();
}

/** Performs an authenticated request expecting no response body (204 or empty). */
async function requestVoid(path: string, options: RequestInit = {}): Promise<void> {
	let res: Response;
	try {
		res = await authenticatedFetch(`${BASE}${path}`, options);
	} catch (err) {
		if (err instanceof DOMException && (err.name === 'AbortError' || err.name === 'TimeoutError')) {
			throw new Error('Request timed out. Please try again.');
		} else if (err instanceof TypeError) {
			throw new Error('Network error: Unable to connect to the server. Check your network connection.');
		}
		throw err;
	}
	if (!res.ok) {
		throw await extractApiError(res);
	}
}

// Audit logs

export async function listAuditLogs(params?: AuditLogListParams): Promise<PaginatedResponse<AuditLogEntry>> {
	const p = new URLSearchParams();
	if (params?.actor_type) p.set('actor_type', params.actor_type);
	if (params?.action_type) p.set('action_type', params.action_type);
	if (params?.outcome) p.set('outcome', params.outcome);
	if (params?.target_type) p.set('target_type', params.target_type);
	if (params?.target_id) p.set('target_id', params.target_id);
	if (params?.from) p.set('from', params.from);
	if (params?.to) p.set('to', params.to);
	if (params?.actor_id) p.set('actor_id', params.actor_id);
	if (params?.correlation_id) p.set('correlation_id', params.correlation_id);
	if (params?.action_kind) p.set('action_kind', params.action_kind);
	if (params?.page) p.set('page', String(params.page));
	if (params?.per_page) p.set('per_page', String(params.per_page));
	const qs = p.toString();
	return request<PaginatedResponse<AuditLogEntry>>(`/audit-logs${qs ? '?' + qs : ''}`);
}

export async function listSystemAuditLogs(params?: AuditLogListParams): Promise<PaginatedResponse<AuditLogEntry>> {
	const p = new URLSearchParams();
	if (params?.actor_type) p.set('actor_type', params.actor_type);
	if (params?.action_type) p.set('action_type', params.action_type);
	if (params?.outcome) p.set('outcome', params.outcome);
	if (params?.target_type) p.set('target_type', params.target_type);
	if (params?.target_id) p.set('target_id', params.target_id);
	if (params?.from) p.set('from', params.from);
	if (params?.to) p.set('to', params.to);
	if (params?.actor_id) p.set('actor_id', params.actor_id);
	if (params?.correlation_id) p.set('correlation_id', params.correlation_id);
	if (params?.action_kind) p.set('action_kind', params.action_kind);
	if (params?.page) p.set('page', String(params.page));
	if (params?.per_page) p.set('per_page', String(params.per_page));
	const qs = p.toString();
	return request<PaginatedResponse<AuditLogEntry>>(`/system-audit-logs${qs ? '?' + qs : ''}`);
}

export { sealedBoxEncrypt } from './api/crypto';

// ── Notification Rules + Log ──

export function listNotificationChannels(
	page = 1,
	perPage = 50
): Promise<PaginatedResponse<NotificationChannelSummary>> {
	return request(`/notifications/channels?page=${page}&per_page=${perPage}`);
}

export function listNotificationRules(opts?: {
	channelId?: string;
	eventType?: string;
	page?: number;
	perPage?: number;
}): Promise<PaginatedResponse<NotificationRuleResponse>> {
	const params = new URLSearchParams();
	if (opts?.channelId) params.set('channel_id', opts.channelId);
	if (opts?.eventType) params.set('event_type', opts.eventType);
	params.set('page', String(opts?.page ?? 1));
	params.set('per_page', String(opts?.perPage ?? 50));
	return request(`/notifications/rules?${params.toString()}`);
}

export function createNotificationRule(data: {
	channel_id: string;
	event_type: string;
	host_id?: string;
	software_item_id?: string;
	plugin_type?: string;
	enabled?: boolean;
}): Promise<NotificationRuleResponse> {
	return request('/notifications/rules', { method: 'POST', body: JSON.stringify(data) });
}

export function updateNotificationRule(id: string, data: Record<string, unknown>): Promise<NotificationRuleResponse> {
	return request(`/notifications/rules/${encodeURIComponent(id)}`, { method: 'PUT', body: JSON.stringify(data) });
}

export function deleteNotificationRule(id: string): Promise<void> {
	return request(`/notifications/rules/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

export function listNotificationLog(page = 1, perPage = 50): Promise<PaginatedResponse<NotificationLogEntry>> {
	return request(`/notifications/log?page=${page}&per_page=${perPage}`);
}

// ── Profile management ────────────────────────────────────────────────

export interface UpdateProfileRequest {
	first_name: string;
	last_name: string;
}

export interface InitiateEmailChangeRequest {
	current_password: string;
	new_email: string;
}

export interface ChangePasswordRequest {
	current_password: string;
	new_password: string;
}

export function updateProfile(userId: string, data: UpdateProfileRequest): Promise<void> {
	return requestVoid(`/users/${encodeURIComponent(userId)}/profile`, {
		method: 'PUT',
		body: JSON.stringify(data)
	});
}

export function initiateEmailChange(userId: string, data: InitiateEmailChangeRequest): Promise<void> {
	return requestVoid(`/users/${encodeURIComponent(userId)}/email`, {
		method: 'POST',
		body: JSON.stringify(data)
	});
}

export function cancelEmailChange(userId: string): Promise<void> {
	return requestVoid(`/users/${encodeURIComponent(userId)}/email`, {
		method: 'DELETE'
	});
}

export function changePassword(userId: string, data: ChangePasswordRequest): Promise<void> {
	return requestVoid(`/users/${encodeURIComponent(userId)}/password`, {
		method: 'PUT',
		body: JSON.stringify(data)
	});
}

export function confirmEmailChange(token: string): Promise<{ message: string }> {
	return request<{ message: string }>(`/auth/email-change/confirm?token=${encodeURIComponent(token)}`);
}
