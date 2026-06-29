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
	BatchActionResponse,
	AgentCertificateSettings,
	ApiTokenListResponse,
	AssignHostsRequest,
	AuthMethodsResponse,
	AuthResponse,
	SoftwareIgnoreResponse,
	CombinedSettingsResponse,
	CreateApiTokenRequest,
	CreateApiTokenResponse,
	CreateSoftwareIgnoreRequest,
	CreateEnrollmentTokenRequest,
	CreateOidcProviderRequest,
	EnrollmentTokenCreatedResponse,
	EnrollmentTokenResponse,
	HostResponse,
	LoginRequest,
	MessageResponse,
	PaginatedResponse,
	PluginConfigResponse,
	CreatePluginConfigRequest,
	CreateSoftwareItemRequest,
	NetworkSettings,
	OidcLinkRequest,
	OidcProviderResponse,
	RefreshResponse,
	RegisterRequest,
	RenewServerCertResponse,
	RotateCaResponse,
	ScheduledTaskResponse,
	ServiceResponse,
	SoftwareItemDetailResponse,
	SoftwareItemResponse,
	SystemAlertsResponse,
	TriggerDiscoveryResponse,
	TriggerScheduledTaskResponse,
	TriggerUpdateRequest,
	TriggerUpdateResponse,
	TriggerVersionCheckResponse,
	UpdateAgentCertificateSettings,
	UpdateHistoryResponse,
	UpdateHostAssignmentRequest,
	UpdateHostRequest,
	UpdateNetworkSettings,
	UpdateOidcProviderRequest,
	UpdatePluginConfigRequest,
	UpdateScheduledTaskRequest,
	UpdateServiceRequest,
	UpdateSoftwareItemRequest,
	User,
	TenantDiscoveryAllowlistEntry,
	HostDiscoveryAllowlistEntry,
	CreateDiscoveryAllowlistEntryRequest,
	GitHubProviderSettingsResponse,
	NatsSettingsResponse,
	UpdateNatsSettingsRequest,
	UpdateGitHubProviderSettingsRequest,
	ZeroconfSettingsResponse,
	UpdateZeroconfSettingsRequest,
	SystemServiceResponse,
	UpdateSystemServiceRequest,
	CreateSystemEnrollmentTokenRequest,
	SystemEnrollmentTokenCreatedResponse,
	SystemEnrollmentTokenResponse,
	PluginTypeInfo,
	PluginTypeSettingsResponse,
	AuditLogEntry,
	AuditLogListParams,
	HostTagResponse,
	CreateHostTagRequest,
	UpdateHostTagRequest,
	SetHostTagsRequest,
	HostTagSummary,
	NotificationChannelSummary,
	NotificationRuleResponse,
	NotificationLogEntry,
	ResetDataRequest,
	ResetDataResponse,
	TestPluginConfigRequest,
	TestPluginConfigResponse,
	MergeSoftwareItemsExecuteRequest,
	MergeSoftwareItemsExecuteResponse,
	MergeSoftwareItemsPreviewRequest,
	MergeSoftwareItemsPreviewResponse,
	InstancePluginSummary,
	ConfigStateResponse,
	MfaVerifyRequest,
	MfaEmailRequest,
	MfaStatusResponse,
	TotpEnrollResponse,
	TotpConfirmRequest,
	TotpConfirmResponse,
	DisableTotpRequest,
	RegenerateRecoveryCodesRequest,
	RegenerateRecoveryCodesResponse
} from './types';

const BASE: string = import.meta.env.VITE_API_BASE || '/api/v1';
const DEFAULT_TIMEOUT_MS = 30_000;
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

export function register(data: RegisterRequest): Promise<AuthResponse> {
	return request('/auth/register', { method: 'POST', body: JSON.stringify(data) });
}

export function login(data: LoginRequest): Promise<AuthResponse> {
	return request('/auth/login', { method: 'POST', body: JSON.stringify(data) });
}

export function logout(): Promise<void> {
	return requestVoid('/auth/logout', {
		method: 'POST',
		body: JSON.stringify({})
	});
}

export function me(): Promise<User> {
	return request('/auth/me');
}

export function getAuthMethods(): Promise<AuthMethodsResponse> {
	return request('/auth/methods');
}

export function getOidcAuthorizeUrl(providerId: string): Promise<{ authorize_url: string }> {
	return request(`/auth/oidc/${encodeURIComponent(providerId)}/authorize`);
}

export function oidcLink(data: OidcLinkRequest): Promise<AuthResponse> {
	return request('/auth/oidc/link', { method: 'POST', body: JSON.stringify(data) });
}

export async function oidcCompleteRegistration(
	registrationCode: string,
	registrationToken: string
): Promise<AuthResponse> {
	const res = await fetch(`${BASE}/auth/oidc/complete-registration`, {
		method: 'POST',
		headers: { 'Content-Type': 'application/json' },
		credentials: 'same-origin',
		body: JSON.stringify({ registration_code: registrationCode, registration_token: registrationToken }),
		signal: AbortSignal.timeout(DEFAULT_TIMEOUT_MS)
	});
	if (!res.ok) {
		const message = await extractErrorMessage(res);
		throw new Error(message);
	}
	return res.json();
}

export async function oidcExchange(code: string): Promise<AuthResponse> {
	// Direct fetch without auth headers — this is a public endpoint
	const res = await fetch(`${BASE}/auth/oidc/exchange`, {
		method: 'POST',
		headers: { 'Content-Type': 'application/json' },
		credentials: 'same-origin',
		body: JSON.stringify({ code }),
		signal: AbortSignal.timeout(DEFAULT_TIMEOUT_MS)
	});
	if (!res.ok) {
		const message = await extractErrorMessage(res);
		throw new Error(message);
	}
	return res.json();
}

/** POST /auth/mfa/verify — complete MFA challenge (unauthenticated, uses mfa_token) */
export async function mfaVerify(data: MfaVerifyRequest): Promise<AuthResponse> {
	const res = await fetch(`${BASE}/auth/mfa/verify`, {
		method: 'POST',
		headers: { 'Content-Type': 'application/json' },
		credentials: 'same-origin',
		body: JSON.stringify(data),
		signal: AbortSignal.timeout(DEFAULT_TIMEOUT_MS)
	});
	if (!res.ok) throw new Error(await extractErrorMessage(res));
	return res.json();
}

/** POST /auth/mfa/email — trigger email OTP (unauthenticated, uses mfa_token) */
export async function mfaSendEmail(data: MfaEmailRequest): Promise<void> {
	const res = await fetch(`${BASE}/auth/mfa/email`, {
		method: 'POST',
		headers: { 'Content-Type': 'application/json' },
		credentials: 'same-origin',
		body: JSON.stringify(data),
		signal: AbortSignal.timeout(DEFAULT_TIMEOUT_MS)
	});
	if (!res.ok) throw new Error(await extractErrorMessage(res));
}

/** GET /auth/me/2fa — current 2FA status */
export function mfaStatus(): Promise<MfaStatusResponse> {
	return request('/auth/me/2fa');
}

/** POST /auth/me/2fa/totp/enroll — begin TOTP enrollment */
export function mfaEnroll(): Promise<TotpEnrollResponse> {
	return request('/auth/me/2fa/totp/enroll', { method: 'POST', body: JSON.stringify({}) });
}

/** POST /auth/me/2fa/totp/confirm — confirm TOTP code */
export function mfaConfirm(data: TotpConfirmRequest): Promise<TotpConfirmResponse> {
	return request('/auth/me/2fa/totp/confirm', { method: 'POST', body: JSON.stringify(data) });
}

/** POST /auth/me/2fa/totp/disable — disable TOTP */
export function mfaDisable(data: DisableTotpRequest): Promise<void> {
	return requestVoid('/auth/me/2fa/totp/disable', { method: 'POST', body: JSON.stringify(data) });
}

/** POST /auth/me/2fa/recovery-codes/regenerate — replace recovery codes */
export function mfaRegenerateCodes(data: RegenerateRecoveryCodesRequest): Promise<RegenerateRecoveryCodesResponse> {
	return request('/auth/me/2fa/recovery-codes/regenerate', { method: 'POST', body: JSON.stringify(data) });
}

export function getServices(options?: {
	capability?: string;
	status?: string;
	page?: number;
	perPage?: number;
}): Promise<PaginatedResponse<ServiceResponse>> {
	const params = new URLSearchParams();
	if (options?.capability) params.set('capability', options.capability);
	if (options?.status) params.set('status', options.status);
	if (options?.page != null) params.set('page', String(options.page));
	if (options?.perPage != null) params.set('per_page', String(options.perPage));
	const query = params.toString();
	return request(`/services${query ? `?${query}` : ''}`);
}

export function approveService(id: string): Promise<ServiceResponse> {
	return request(`/services/${encodeURIComponent(id)}/approve`, { method: 'POST' });
}

export function rejectService(id: string): Promise<ServiceResponse> {
	return request(`/services/${encodeURIComponent(id)}/reject`, { method: 'POST' });
}

export function deleteService(id: string): Promise<MessageResponse> {
	return request(`/services/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

export function updateService(id: string, data: UpdateServiceRequest): Promise<ServiceResponse> {
	return request(`/services/${encodeURIComponent(id)}`, { method: 'PUT', body: JSON.stringify(data) });
}

export function mergeService(targetId: string, sourceId: string): Promise<ServiceResponse> {
	return request(`/services/${encodeURIComponent(targetId)}/merge`, {
		method: 'POST',
		body: JSON.stringify({ source_id: sourceId })
	});
}

// --- Host APIs ---

export function getHosts(page?: number, perPage?: number): Promise<PaginatedResponse<HostResponse>> {
	const params = new URLSearchParams();
	if (page != null) params.set('page', String(page));
	if (perPage != null) params.set('per_page', String(perPage));
	const query = params.toString();
	return request(`/hosts${query ? `?${query}` : ''}`);
}

export function getHost(id: string): Promise<HostResponse> {
	return request(`/hosts/${encodeURIComponent(id)}`);
}

export function updateHost(id: string, data: UpdateHostRequest): Promise<HostResponse> {
	return request(`/hosts/${encodeURIComponent(id)}`, { method: 'PUT', body: JSON.stringify(data) });
}

export function deactivateHost(id: string): Promise<void> {
	return requestVoid(`/hosts/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

// --- Host Tag APIs ---

export function getHostTags(
	page?: number,
	perPage?: number,
	search?: string
): Promise<PaginatedResponse<HostTagResponse>> {
	const params = new URLSearchParams();
	if (page != null) params.set('page', String(page));
	if (perPage != null) params.set('per_page', String(perPage));
	if (search) params.set('search', search);
	const query = params.toString();
	return request(`/host-tags${query ? `?${query}` : ''}`);
}

export function getHostTag(id: string): Promise<HostTagResponse> {
	return request(`/host-tags/${encodeURIComponent(id)}`);
}

export function createHostTag(data: CreateHostTagRequest): Promise<HostTagResponse> {
	return request('/host-tags', { method: 'POST', body: JSON.stringify(data) });
}

export function updateHostTag(id: string, data: UpdateHostTagRequest): Promise<HostTagResponse> {
	return request(`/host-tags/${encodeURIComponent(id)}`, { method: 'PUT', body: JSON.stringify(data) });
}

export function deleteHostTag(id: string): Promise<void> {
	return requestVoid(`/host-tags/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

export function setHostTags(hostId: string, data: SetHostTagsRequest): Promise<HostTagSummary[]> {
	return request(`/hosts/${encodeURIComponent(hostId)}/tags`, { method: 'PUT', body: JSON.stringify(data) });
}

export function batchHostTags(action: string, ids: string[]): Promise<BatchActionResponse> {
	return request('/host-tags/batch', { method: 'POST', body: JSON.stringify({ action, ids }) });
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

// --- Device Authorization ---

export function approveDeviceAuth(userCode: string): Promise<{ message: string }> {
	return request('/auth/device/approve', {
		method: 'POST',
		body: JSON.stringify({ user_code: userCode })
	});
}

export interface DeviceLookup {
	client_name: string | null;
	expires_at: string; // RFC 3339
}

export async function denyDeviceAuth(userCode: string): Promise<{ message: string }> {
	return request('/auth/device/deny', {
		method: 'POST',
		body: JSON.stringify({ user_code: userCode })
	});
}

export async function lookupDeviceAuth(userCode: string): Promise<DeviceLookup> {
	const qs = new URLSearchParams({ user_code: userCode });
	return request(`/auth/device/lookup?${qs.toString()}`);
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

// --- System Services APIs ---

export function getSystemServices(options?: {
	capability?: string;
	status?: string;
	page?: number;
	perPage?: number;
}): Promise<PaginatedResponse<SystemServiceResponse>> {
	const params = new URLSearchParams();
	if (options?.capability) params.set('capability', options.capability);
	if (options?.status) params.set('status', options.status);
	if (options?.page != null) params.set('page', String(options.page));
	if (options?.perPage != null) params.set('per_page', String(options.perPage));
	const query = params.toString();
	return request(`/system-services${query ? `?${query}` : ''}`);
}

export function approveSystemService(id: string): Promise<SystemServiceResponse> {
	return request(`/system-services/${encodeURIComponent(id)}/approve`, { method: 'POST' });
}

export function rejectSystemService(id: string): Promise<SystemServiceResponse> {
	return request(`/system-services/${encodeURIComponent(id)}/reject`, { method: 'POST' });
}

export function deleteSystemService(id: string): Promise<void> {
	return requestVoid(`/system-services/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

export function updateSystemService(id: string, data: UpdateSystemServiceRequest): Promise<SystemServiceResponse> {
	return request(`/system-services/${encodeURIComponent(id)}`, {
		method: 'PUT',
		body: JSON.stringify(data)
	});
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

export function getSoftwareItems(
	page?: number,
	perPage?: number,
	featured?: boolean,
	hostId?: string,
	updatable?: boolean,
	pluginType?: string,
	query?: string
): Promise<PaginatedResponse<SoftwareItemResponse>> {
	const params = new URLSearchParams();
	if (page != null) params.set('page', String(page));
	if (perPage != null) params.set('per_page', String(perPage));
	if (featured != null) params.set('featured', String(featured));
	if (hostId != null) params.set('host_id', hostId);
	if (updatable != null) params.set('updatable', String(updatable));
	if (pluginType != null) params.set('plugin_type', pluginType);
	const normalizedQuery = query?.trim();
	if (normalizedQuery) params.set('query', normalizedQuery);
	const qs = params.toString();
	return request(`/software-items${qs ? `?${qs}` : ''}`);
}

export function previewSoftwareItemMerge(
	data: MergeSoftwareItemsPreviewRequest
): Promise<MergeSoftwareItemsPreviewResponse> {
	return request('/software-items/merge/preview', {
		method: 'POST',
		body: JSON.stringify(data)
	});
}

export function executeSoftwareItemMerge(
	data: MergeSoftwareItemsExecuteRequest
): Promise<MergeSoftwareItemsExecuteResponse> {
	return request('/software-items/merge/execute', {
		method: 'POST',
		body: JSON.stringify(data)
	});
}

export function createSoftwareItem(data: CreateSoftwareItemRequest): Promise<SoftwareItemResponse> {
	return request('/software-items', {
		method: 'POST',
		body: JSON.stringify({ name: data.name, featured: data.featured ?? true })
	});
}

export function getSoftwareItem(id: string): Promise<SoftwareItemDetailResponse> {
	return request(`/software-items/${encodeURIComponent(id)}`);
}

export function deleteSoftwareItem(id: string): Promise<void> {
	return requestVoid(`/software-items/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

export function assignHostsToSoftwareItem(id: string, data: AssignHostsRequest): Promise<SoftwareItemDetailResponse> {
	return request(`/software-items/${encodeURIComponent(id)}/hosts`, {
		method: 'POST',
		body: JSON.stringify(data)
	});
}

export function unassignHostFromSoftwareItem(itemId: string, hostId: string): Promise<void> {
	return requestVoid(`/software-items/${encodeURIComponent(itemId)}/hosts/${encodeURIComponent(hostId)}`, {
		method: 'DELETE'
	});
}

export function updateHostAssignment(
	itemId: string,
	hostId: string,
	data: UpdateHostAssignmentRequest
): Promise<SoftwareItemDetailResponse> {
	return request(`/software-items/${encodeURIComponent(itemId)}/hosts/${encodeURIComponent(hostId)}`, {
		method: 'PUT',
		body: JSON.stringify(data)
	});
}

export function deletePluginAssignment(
	itemId: string,
	hostId: string,
	role: string,
	ordinal: number
): Promise<SoftwareItemDetailResponse> {
	return request(
		`/software-items/${encodeURIComponent(itemId)}/hosts/${encodeURIComponent(hostId)}/plugins/${encodeURIComponent(role)}/${ordinal}`,
		{ method: 'DELETE' }
	);
}

export function checkSoftwareItemVersions(itemId: string): Promise<TriggerVersionCheckResponse> {
	return request(`/software-items/${encodeURIComponent(itemId)}/check-versions`, { method: 'POST' });
}

export function triggerHostDiscovery(hostId: string): Promise<TriggerDiscoveryResponse> {
	return request(`/hosts/${encodeURIComponent(hostId)}/discover`, { method: 'POST' });
}

export function getSoftwareIgnores(
	page?: number,
	perPage?: number
): Promise<PaginatedResponse<SoftwareIgnoreResponse>> {
	const params = new URLSearchParams();
	if (page != null) params.set('page', String(page));
	if (perPage != null) params.set('per_page', String(perPage));
	const query = params.toString();
	return request(`/autodiscovery/ignores${query ? `?${query}` : ''}`);
}

export function createSoftwareIgnore(req: CreateSoftwareIgnoreRequest): Promise<SoftwareIgnoreResponse> {
	return request('/autodiscovery/ignores', { method: 'POST', body: JSON.stringify(req) });
}

export function deleteSoftwareIgnore(id: string): Promise<void> {
	return requestVoid(`/autodiscovery/ignores/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

// Software items - update
export async function updateSoftwareItem(id: string, data: UpdateSoftwareItemRequest): Promise<SoftwareItemResponse> {
	return request<SoftwareItemResponse>(`/software-items/${encodeURIComponent(id)}`, {
		method: 'PUT',
		body: JSON.stringify(data)
	});
}

// Software items - trigger update on a specific host
export async function triggerSoftwareUpdate(
	itemId: string,
	hostId: string,
	req: TriggerUpdateRequest
): Promise<TriggerUpdateResponse> {
	return request<TriggerUpdateResponse>(
		`/software-items/${encodeURIComponent(itemId)}/hosts/${encodeURIComponent(hostId)}/update`,
		{
			method: 'POST',
			body: JSON.stringify(req)
		}
	);
}

// Software items - check versions on a specific host
export async function checkSoftwareItemVersionsHost(
	itemId: string,
	hostId: string
): Promise<TriggerVersionCheckResponse> {
	return request<TriggerVersionCheckResponse>(
		`/software-items/${encodeURIComponent(itemId)}/hosts/${encodeURIComponent(hostId)}/check-versions`,
		{ method: 'POST' }
	);
}

// Update history
export async function listUpdateHistory(opts?: {
	host_id?: string;
	software_item_id?: string;
	status?: string;
	page?: number;
	per_page?: number;
}): Promise<PaginatedResponse<UpdateHistoryResponse>> {
	const params = new URLSearchParams();
	if (opts?.host_id) params.set('host_id', opts.host_id);
	if (opts?.software_item_id) params.set('software_item_id', opts.software_item_id);
	if (opts?.status) params.set('status', opts.status);
	if (opts?.page) params.set('page', String(opts.page));
	if (opts?.per_page) params.set('per_page', String(opts.per_page));
	const qs = params.toString();
	return request<PaginatedResponse<UpdateHistoryResponse>>(`/update-history${qs ? '?' + qs : ''}`);
}

export async function getUpdateHistoryEntry(id: string): Promise<UpdateHistoryResponse> {
	return request<UpdateHistoryResponse>(`/update-history/${encodeURIComponent(id)}`);
}

// Scheduler tasks
export async function listSchedulerTasks(): Promise<ScheduledTaskResponse[]> {
	return request<ScheduledTaskResponse[]>('/scheduler/tasks');
}

export async function getSchedulerTask(id: string): Promise<ScheduledTaskResponse> {
	return request<ScheduledTaskResponse>(`/scheduler/tasks/${encodeURIComponent(id)}`);
}

export async function updateSchedulerTask(
	id: string,
	data: UpdateScheduledTaskRequest
): Promise<ScheduledTaskResponse> {
	return request<ScheduledTaskResponse>(`/scheduler/tasks/${encodeURIComponent(id)}`, {
		method: 'PUT',
		body: JSON.stringify(data)
	});
}

export async function triggerSchedulerTask(id: string): Promise<TriggerScheduledTaskResponse> {
	return request<TriggerScheduledTaskResponse>(`/scheduler/tasks/${encodeURIComponent(id)}/trigger`, {
		method: 'POST'
	});
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

// Discovery allowlist — tenant-wide

export async function listDiscoveryAllowlist(): Promise<TenantDiscoveryAllowlistEntry[]> {
	return request<TenantDiscoveryAllowlistEntry[]>('/discovery-allowlist');
}

export async function addDiscoveryAllowlistEntry(
	req: CreateDiscoveryAllowlistEntryRequest
): Promise<TenantDiscoveryAllowlistEntry> {
	return request<TenantDiscoveryAllowlistEntry>('/discovery-allowlist', {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

export async function deleteDiscoveryAllowlistEntry(id: string): Promise<void> {
	return requestVoid(`/discovery-allowlist/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

// Discovery allowlist — host-specific

export async function listHostDiscoveryAllowlist(hostId: string): Promise<HostDiscoveryAllowlistEntry[]> {
	return request<HostDiscoveryAllowlistEntry[]>(`/hosts/${encodeURIComponent(hostId)}/discovery-allowlist`);
}

export async function addHostDiscoveryAllowlistEntry(
	hostId: string,
	req: CreateDiscoveryAllowlistEntryRequest
): Promise<HostDiscoveryAllowlistEntry> {
	return request<HostDiscoveryAllowlistEntry>(`/hosts/${encodeURIComponent(hostId)}/discovery-allowlist`, {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

export async function deleteHostDiscoveryAllowlistEntry(hostId: string, entryId: string): Promise<void> {
	return requestVoid(`/hosts/${encodeURIComponent(hostId)}/discovery-allowlist/${encodeURIComponent(entryId)}`, {
		method: 'DELETE'
	});
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

// ── Batch Actions ─────────────────────────────────────────────────────

export function batchServices(action: string, ids: string[]): Promise<BatchActionResponse> {
	return request('/services/batch', { method: 'POST', body: JSON.stringify({ action, ids }) });
}

export function batchSystemServices(action: string, ids: string[]): Promise<BatchActionResponse> {
	return request('/system-services/batch', { method: 'POST', body: JSON.stringify({ action, ids }) });
}

export function batchSoftwareItems(action: string, ids: string[]): Promise<BatchActionResponse> {
	return request('/software-items/batch', { method: 'POST', body: JSON.stringify({ action, ids }) });
}

export function batchHosts(action: string, ids: string[]): Promise<BatchActionResponse> {
	return request('/hosts/batch', { method: 'POST', body: JSON.stringify({ action, ids }) });
}

export function batchSoftwareIgnores(action: string, ids: string[]): Promise<BatchActionResponse> {
	return request('/autodiscovery/ignores/batch', { method: 'POST', body: JSON.stringify({ action, ids }) });
}

export function batchPluginConfigs(action: string, ids: string[]): Promise<BatchActionResponse> {
	return request('/plugin-configs/batch', { method: 'POST', body: JSON.stringify({ action, ids }) });
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
