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
	AgentCertificateSettings,
	ApiTokenListResponse,
	CombinedSettingsResponse,
	CreateApiTokenRequest,
	CreateApiTokenResponse,
	CreateEnrollmentTokenRequest,
	CreateOidcProviderRequest,
	EnrollmentTokenCreatedResponse,
	EnrollmentTokenResponse,
	MessageResponse,
	PaginatedResponse,
	PluginConfigResponse,
	CreatePluginConfigRequest,
	NetworkSettings,
	OidcProviderResponse,
	RefreshResponse,
	RenewServerCertResponse,
	RotateCaResponse,
	SystemAlertsResponse,
	TriggerDiscoveryResponse,
	UpdateAgentCertificateSettings,
	UpdateNetworkSettings,
	UpdateOidcProviderRequest,
	UpdatePluginConfigRequest,
	GitHubProviderSettingsResponse,
	NatsSettingsResponse,
	UpdateNatsSettingsRequest,
	UpdateGitHubProviderSettingsRequest,
	ZeroconfSettingsResponse,
	UpdateZeroconfSettingsRequest,
	CreateSystemEnrollmentTokenRequest,
	SystemEnrollmentTokenCreatedResponse,
	SystemEnrollmentTokenResponse,
	PluginTypeInfo,
	PluginTypeSettingsResponse,
	AuditLogEntry,
	AuditLogListParams,
	NotificationChannelSummary,
	NotificationRuleResponse,
	NotificationLogEntry,
	ResetDataRequest,
	ResetDataResponse,
	TestPluginConfigRequest,
	TestPluginConfigResponse,
	InstancePluginSummary,
	ConfigStateResponse
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

// --- Settings APIs ---

export function getAgentCertificateSettings(): Promise<AgentCertificateSettings> {
	return request('/settings/agent-certificates');
}

export function updateAgentCertificateSettings(
	data: UpdateAgentCertificateSettings
): Promise<AgentCertificateSettings> {
	return request('/settings/agent-certificates', { method: 'PUT', body: JSON.stringify(data) });
}

export function getCombinedSettings(): Promise<CombinedSettingsResponse> {
	return request('/settings');
}

// --- Enrollment Token APIs ---

export function listEnrollmentTokens(
	page?: number,
	perPage?: number
): Promise<PaginatedResponse<EnrollmentTokenResponse>> {
	const params = new URLSearchParams();
	if (page != null) params.set('page', String(page));
	if (perPage != null) params.set('per_page', String(perPage));
	const query = params.toString();
	return request(`/enrollment-tokens${query ? `?${query}` : ''}`);
}

export function createEnrollmentToken(data: CreateEnrollmentTokenRequest): Promise<EnrollmentTokenCreatedResponse> {
	return request('/enrollment-tokens', { method: 'POST', body: JSON.stringify(data) });
}

export function getEnrollmentToken(id: string): Promise<EnrollmentTokenResponse> {
	return request(`/enrollment-tokens/${encodeURIComponent(id)}`);
}

export function revokeEnrollmentToken(id: string): Promise<MessageResponse> {
	return request(`/enrollment-tokens/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

// --- Network Settings APIs ---

export function getNetworkSettings(): Promise<NetworkSettings> {
	return request('/global-settings/network');
}

export function updateNetworkSettings(data: UpdateNetworkSettings): Promise<NetworkSettings> {
	return request('/global-settings/network', { method: 'PUT', body: JSON.stringify(data) });
}

// --- OIDC Provider APIs ---

export function getOidcProviders(): Promise<OidcProviderResponse[]> {
	return request('/settings/oidc-providers');
}

export function createOidcProvider(data: CreateOidcProviderRequest): Promise<OidcProviderResponse> {
	return request('/settings/oidc-providers', { method: 'POST', body: JSON.stringify(data) });
}

export function updateOidcProvider(id: string, data: UpdateOidcProviderRequest): Promise<OidcProviderResponse> {
	return request(`/settings/oidc-providers/${encodeURIComponent(id)}`, {
		method: 'PUT',
		body: JSON.stringify(data)
	});
}

export function deleteOidcProvider(id: string): Promise<void> {
	return requestVoid(`/settings/oidc-providers/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

export function activateOidcProvider(id: string): Promise<OidcProviderResponse> {
	return request(`/settings/oidc-providers/${encodeURIComponent(id)}/activate`, { method: 'POST' });
}

export function deactivateOidcProvider(id: string): Promise<OidcProviderResponse> {
	return request(`/settings/oidc-providers/${encodeURIComponent(id)}/deactivate`, { method: 'POST' });
}

// --- System Alerts ---

export function getSystemAlerts(): Promise<SystemAlertsResponse> {
	return request('/system/alerts');
}

// --- Server Certificate ---

export function renewServerCertificate(): Promise<RenewServerCertResponse> {
	return request('/settings/renew-server-certificate', { method: 'POST' });
}

// --- NATS Settings ---

export function getNatsSettings(): Promise<NatsSettingsResponse> {
	return request('/global-settings/nats');
}

export function updateNatsSettings(data: UpdateNatsSettingsRequest): Promise<NatsSettingsResponse> {
	return request('/global-settings/nats', { method: 'PUT', body: JSON.stringify(data) });
}

// --- Global GitHub Provider Settings ---

export function getGitHubProviderSettings(): Promise<GitHubProviderSettingsResponse> {
	return request('/global-settings/providers/github');
}

export function updateGitHubProviderSettings(
	data: UpdateGitHubProviderSettingsRequest
): Promise<GitHubProviderSettingsResponse> {
	return request('/global-settings/providers/github', { method: 'PUT', body: JSON.stringify(data) });
}

// --- Zeroconf Settings ---

export function getZeroconfSettings(): Promise<ZeroconfSettingsResponse> {
	return request('/global-settings/zeroconf');
}

export function updateZeroconfSettings(data: UpdateZeroconfSettingsRequest): Promise<ZeroconfSettingsResponse> {
	return request('/global-settings/zeroconf', { method: 'PUT', body: JSON.stringify(data) });
}

// --- System Enrollment Tokens APIs ---

export function listSystemEnrollmentTokens(options?: {
	page?: number;
	perPage?: number;
}): Promise<PaginatedResponse<SystemEnrollmentTokenResponse>> {
	const params = new URLSearchParams();
	if (options?.page != null) params.set('page', String(options.page));
	if (options?.perPage != null) params.set('per_page', String(options.perPage));
	const query = params.toString();
	return request(`/system-enrollment-tokens${query ? `?${query}` : ''}`);
}

export function createSystemEnrollmentToken(
	data: CreateSystemEnrollmentTokenRequest
): Promise<SystemEnrollmentTokenCreatedResponse> {
	return request('/system-enrollment-tokens', { method: 'POST', body: JSON.stringify(data) });
}

export function getSystemEnrollmentToken(id: string): Promise<SystemEnrollmentTokenResponse> {
	return request(`/system-enrollment-tokens/${encodeURIComponent(id)}`);
}

export function revokeSystemEnrollmentToken(id: string): Promise<void> {
	return requestVoid(`/system-enrollment-tokens/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

// --- Plugin Types & Configs ---

/** Fetch all known plugin types with display names and capabilities from the registry. */
export function listPluginTypes(): Promise<PluginTypeInfo[]> {
	return request<PluginTypeInfo[]>('/plugin-types');
}

export function listPluginTypeSettings(): Promise<PluginTypeSettingsResponse[]> {
	return request<PluginTypeSettingsResponse[]>('/plugin-type-settings');
}

export function getPluginTypeSettings(pluginType: string): Promise<PluginTypeSettingsResponse> {
	return request<PluginTypeSettingsResponse>(`/plugin-type-settings/${encodeURIComponent(pluginType)}`);
}

export function upsertPluginTypeSettings(
	pluginType: string,
	config: Record<string, unknown>
): Promise<PluginTypeSettingsResponse> {
	return request<PluginTypeSettingsResponse>(`/plugin-type-settings/${encodeURIComponent(pluginType)}`, {
		method: 'PUT',
		body: JSON.stringify({ config })
	});
}

export function deletePluginTypeSettings(pluginType: string): Promise<void> {
	return requestVoid(`/plugin-type-settings/${encodeURIComponent(pluginType)}`, { method: 'DELETE' });
}

// --- Instance-Scoped Plugins ---

export function listInstancePlugins(): Promise<InstancePluginSummary[]> {
	return request<InstancePluginSummary[]>('/instance-plugins');
}

export function setInstancePluginEnabled(pluginType: string, enabled: boolean): Promise<InstancePluginSummary> {
	return request<InstancePluginSummary>(`/instance-plugins/${encodeURIComponent(pluginType)}/enabled`, {
		method: 'PUT',
		body: JSON.stringify({ enabled })
	});
}

export function upsertInstancePluginConfig(
	pluginType: string,
	config: Record<string, unknown>
): Promise<InstancePluginSummary> {
	return request<InstancePluginSummary>(`/instance-plugins/${encodeURIComponent(pluginType)}/config`, {
		method: 'PUT',
		body: JSON.stringify({ config })
	});
}

export function getPluginConfigs(page?: number, perPage?: number): Promise<PaginatedResponse<PluginConfigResponse>> {
	const params = new URLSearchParams();
	if (page != null) params.set('page', String(page));
	if (perPage != null) params.set('per_page', String(perPage));
	const query = params.toString();
	return request(`/plugin-configs${query ? `?${query}` : ''}`);
}

// Plugin configs - CRUD
export async function getPluginConfig(id: string): Promise<PluginConfigResponse> {
	return request<PluginConfigResponse>(`/plugin-configs/${encodeURIComponent(id)}`);
}

export async function createPluginConfig(data: CreatePluginConfigRequest): Promise<PluginConfigResponse> {
	return request<PluginConfigResponse>('/plugin-configs', { method: 'POST', body: JSON.stringify(data) });
}

export async function updatePluginConfig(id: string, data: UpdatePluginConfigRequest): Promise<PluginConfigResponse> {
	return request<PluginConfigResponse>(`/plugin-configs/${encodeURIComponent(id)}`, {
		method: 'PUT',
		body: JSON.stringify(data)
	});
}

export async function deletePluginConfig(id: string): Promise<void> {
	return requestVoid(`/plugin-configs/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

export async function triggerPluginConfigDiscovery(id: string): Promise<TriggerDiscoveryResponse> {
	return request<TriggerDiscoveryResponse>(`/plugin-configs/${encodeURIComponent(id)}/discover`, { method: 'POST' });
}

export async function testPluginConfig(data: TestPluginConfigRequest): Promise<TestPluginConfigResponse> {
	return request<TestPluginConfigResponse>('/plugin-configs/test', { method: 'POST', body: JSON.stringify(data) });
}

// API tokens
export async function listApiTokens(): Promise<ApiTokenListResponse> {
	return request<ApiTokenListResponse>('/auth/api-tokens');
}

export async function createApiToken(data: CreateApiTokenRequest): Promise<CreateApiTokenResponse> {
	return request<CreateApiTokenResponse>('/auth/api-tokens', { method: 'POST', body: JSON.stringify(data) });
}

export async function revokeApiToken(id: string): Promise<void> {
	return requestVoid(`/auth/api-tokens/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

// CA rotation
export async function rotateCA(): Promise<RotateCaResponse> {
	return request<RotateCaResponse>('/global-settings/ca/rotate', { method: 'POST' });
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

// ── Reset Data ────────────────────────────────────────────────────────

export function resetData(data: ResetDataRequest): Promise<ResetDataResponse> {
	return request('/settings/reset-data', { method: 'POST', body: JSON.stringify(data) });
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

// ── Instance Config State ─────────────────────────────────────────────

export async function getConfigState(): Promise<ConfigStateResponse> {
	return request<ConfigStateResponse>('/instance/config-state');
}

export async function clearCoordinatorDegraded(): Promise<ConfigStateResponse> {
	return request<ConfigStateResponse>('/instance/config-reload/clear-degraded', { method: 'POST' });
}
