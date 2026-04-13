import { getAccessToken, setAccessToken, setSessionExpired } from './token-store.svelte';
import type {
	BatchActionResponse,
	AgentCertificateSettings,
	ApiTokenListResponse,
	AssignHostsRequest,
	AuthenticationSettings,
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
	RegistrationSettings,
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
	UpdateAuthenticationSettings,
	UpdateHistoryResponse,
	UpdateHostAssignmentRequest,
	UpdateHostRequest,
	UpdateNetworkSettings,
	UpdateOidcProviderRequest,
	UpdatePluginConfigRequest,
	UpdateRegistrationSettings,
	UpdateScheduledTaskRequest,
	UpdateServiceRequest,
	UpdateSoftwareItemRequest,
	User,
	TenantDiscoveryAllowlistEntry,
	HostDiscoveryAllowlistEntry,
	CreateDiscoveryAllowlistEntryRequest,
	NatsSettingsResponse,
	UpdateNatsSettingsRequest,
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
	ExtensionResponse,
	ExtensionProviderInfo,
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
	MergeSoftwareItemsPreviewResponse
} from './types';

const BASE: string = import.meta.env.VITE_API_BASE || '/api/v1';
const DEFAULT_TIMEOUT_MS = 30_000;
const REFRESH_TIMEOUT_MS = 10_000;
const MAX_ERROR_LENGTH = 500;

function truncateError(msg: string): string {
	return msg.length > MAX_ERROR_LENGTH ? msg.slice(0, MAX_ERROR_LENGTH) + '\u2026' : msg;
}

export async function extractErrorMessage(res: Response): Promise<string> {
	const text = await res.text();
	if (!text) return res.statusText;
	try {
		const parsed = JSON.parse(text);
		if (typeof parsed === 'object' && parsed !== null && typeof parsed.error === 'string') {
			return truncateError(parsed.error);
		}
	} catch {
		/* Not JSON */
	}
	return truncateError(text);
}

function authHeaders(): Record<string, string> {
	const token = getAccessToken();
	return token ? { Authorization: `Bearer ${token}` } : {};
}

let refreshPromise: Promise<RefreshResponse> | null = null;

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

/**
 * Performs an authenticated fetch with automatic token refresh on 401.
 * Returns the raw Response for callers to handle body parsing.
 */
async function authenticatedFetch(path: string, options: RequestInit = {}): Promise<Response> {
	const timeoutSignal = AbortSignal.timeout(DEFAULT_TIMEOUT_MS);
	const combinedSignal = options.signal ? AbortSignal.any([options.signal, timeoutSignal]) : timeoutSignal;

	const res = await fetch(`${BASE}${path}`, {
		credentials: 'same-origin',
		...options,
		headers: {
			'Content-Type': 'application/json',
			...authHeaders(),
			...(options.headers as Record<string, string> | undefined)
		},
		signal: combinedSignal
	});

	if (res.status === 401 && getAccessToken()) {
		// Surface the session-expired banner immediately so the user sees
		// feedback before the refresh round-trip completes.
		setSessionExpired(true);

		// Attempt token refresh, deduplicating concurrent attempts.
		// The promise is cleared after it settles (not per-caller) so that
		// concurrent 401 handlers share a single refresh cycle.
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

		try {
			const refreshed = await refreshPromise;
			setAccessToken(refreshed.access_token);

			// Retry original request with new token
			const retryTimeoutSignal = AbortSignal.timeout(DEFAULT_TIMEOUT_MS);
			const retryCombinedSignal = options.signal
				? AbortSignal.any([options.signal, retryTimeoutSignal])
				: retryTimeoutSignal;

			// The retry is returned (not awaited) so that retry errors propagate
			// to request(), which maps them to the correct user-facing messages.
			// The .finally() clears the banner after the retry settles regardless
			// of whether it succeeds or fails.
			return fetch(`${BASE}${path}`, {
				credentials: 'same-origin',
				...options,
				headers: {
					'Content-Type': 'application/json',
					Authorization: `Bearer ${refreshed.access_token}`,
					...(options.headers as Record<string, string> | undefined)
				},
				signal: retryCombinedSignal
			}).finally(() => {
				setSessionExpired(false);
			});
		} catch (refreshErr) {
			// Network/timeout errors — keep session, surface the error
			if (
				refreshErr instanceof DOMException &&
				(refreshErr.name === 'TimeoutError' || refreshErr.name === 'AbortError')
			) {
				setSessionExpired(false);
				throw new Error('Token refresh timed out. Please try again.');
			}
			if (refreshErr instanceof TypeError) {
				setSessionExpired(false);
				throw new Error('Network error during token refresh. Check your connection.');
			}
			// Server errors (5xx) — keep session, don't force logout
			if (refreshErr instanceof RefreshError && refreshErr.status >= 500) {
				setSessionExpired(false);
				throw new Error('Server error during token refresh. Please try again later.');
			}
			// Real auth failures (4xx) — session is truly invalid.
			// Show a non-blocking banner instead of hard-redirecting so the
			// user can copy any unsaved form state before logging in again.
			setAccessToken(null);
			throw new Error('Session expired. Please log in again.');
		}
	}

	return res;
}

/** Performs an authenticated request and parses the JSON response body. */
async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
	let res: Response;
	try {
		res = await authenticatedFetch(path, options);
	} catch (err) {
		if (err instanceof DOMException && (err.name === 'AbortError' || err.name === 'TimeoutError')) {
			throw new Error('Request timed out. Please try again.');
		} else if (err instanceof TypeError) {
			throw new Error('Network error: Unable to connect to the server. Check your network connection.');
		}
		throw err;
	}
	if (!res.ok) {
		const message = await extractErrorMessage(res);
		throw new Error(message);
	}
	return res.json();
}

/** Performs an authenticated request expecting no response body (204 or empty). */
async function requestVoid(path: string, options: RequestInit = {}): Promise<void> {
	let res: Response;
	try {
		res = await authenticatedFetch(path, options);
	} catch (err) {
		if (err instanceof DOMException && (err.name === 'AbortError' || err.name === 'TimeoutError')) {
			throw new Error('Request timed out. Please try again.');
		} else if (err instanceof TypeError) {
			throw new Error('Network error: Unable to connect to the server. Check your network connection.');
		}
		throw err;
	}
	if (!res.ok) {
		const message = await extractErrorMessage(res);
		throw new Error(message);
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

export function getRegistrationSettings(): Promise<RegistrationSettings> {
	return request('/settings/registration');
}

export function updateRegistrationSettings(data: UpdateRegistrationSettings): Promise<RegistrationSettings> {
	return request('/settings/registration', { method: 'PUT', body: JSON.stringify(data) });
}

export function getAuthenticationSettings(): Promise<AuthenticationSettings> {
	return request('/settings/authentication');
}

export function updateAuthenticationSettings(data: UpdateAuthenticationSettings): Promise<AuthenticationSettings> {
	return request('/settings/authentication', { method: 'PUT', body: JSON.stringify(data) });
}

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
	if (query) params.set('query', query);
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
	return request('/software-items/merge', {
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
	if (params?.method) p.set('method', params.method);
	if (params?.status !== undefined) p.set('status', String(params.status));
	if (params?.from) p.set('from', params.from);
	if (params?.to) p.set('to', params.to);
	if (params?.actor_id) p.set('actor_id', params.actor_id);
	if (params?.page) p.set('page', String(params.page));
	if (params?.per_page) p.set('per_page', String(params.per_page));
	const qs = p.toString();
	return request<PaginatedResponse<AuditLogEntry>>(`/audit-logs${qs ? '?' + qs : ''}`);
}

export async function listSystemAuditLogs(params?: AuditLogListParams): Promise<PaginatedResponse<AuditLogEntry>> {
	const p = new URLSearchParams();
	if (params?.actor_type) p.set('actor_type', params.actor_type);
	if (params?.method) p.set('method', params.method);
	if (params?.status !== undefined) p.set('status', String(params.status));
	if (params?.from) p.set('from', params.from);
	if (params?.to) p.set('to', params.to);
	if (params?.actor_id) p.set('actor_id', params.actor_id);
	if (params?.page) p.set('page', String(params.page));
	if (params?.per_page) p.set('per_page', String(params.per_page));
	const qs = p.toString();
	return request<PaginatedResponse<AuditLogEntry>>(`/system-audit-logs${qs ? '?' + qs : ''}`);
}

// ── Batch Actions ─────────────────────────────────────────────────────

/**
 * Splits `ids` into chunks of at most 100 and calls `batchFn` sequentially,
 * aggregating the results. Use this whenever a selection may exceed the
 * server-side batch limit.
 */
export async function executeBatchChunked(
	action: string,
	ids: string[],
	batchFn: (action: string, ids: string[]) => Promise<BatchActionResponse>
): Promise<BatchActionResponse> {
	const CHUNK_SIZE = 100;
	const result: BatchActionResponse = { succeeded: [], failed: [] };
	for (let i = 0; i < ids.length; i += CHUNK_SIZE) {
		const r = await batchFn(action, ids.slice(i, i + CHUNK_SIZE));
		result.succeeded.push(...r.succeeded);
		result.failed.push(...r.failed);
	}
	return result;
}

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

// ── Extensions ────────────────────────────────────────────────────────

function bytesToBase64(bytes: Uint8Array): string {
	let binary = '';
	for (const byte of bytes) binary += String.fromCharCode(byte);
	return btoa(binary);
}

function base64ToBytes(b64: string): Uint8Array<ArrayBuffer> {
	const binary = atob(b64);
	const bytes = new Uint8Array(new ArrayBuffer(binary.length));
	for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
	return bytes;
}

/**
 * ECIES sealed-box encrypt using the Web Crypto API (P-256 ECDH + AES-256-GCM).
 *
 * Matches the Rust `sealed_box_encrypt_base64` algorithm exactly:
 * - Ephemeral P-256 keypair per message (forward secrecy).
 * - Shared secret = ECDH X-coordinate (32 bytes).
 * - AES-256 key = SHA-256(shared secret).
 * - AAD = ephemeral public key bytes (65 bytes, uncompressed).
 * - Sealed-box format: [ephemeral pubkey (65)] [nonce (12)] [ciphertext + GCM tag (N+16)].
 * - Returns standard (non-URL-safe) base64.
 *
 * @param plaintext - The UTF-8 string to encrypt.
 * @param recipientPublicKeyBase64 - Standard base64-encoded uncompressed P-256 public key (65 bytes).
 */
export async function sealedBoxEncrypt(plaintext: string, recipientPublicKeyBase64: string): Promise<string> {
	const recipientPubKeyBytes = base64ToBytes(recipientPublicKeyBase64);

	// Import recipient's static P-256 public key for ECDH.
	const recipientPublicKey = await crypto.subtle.importKey(
		'raw',
		recipientPubKeyBytes,
		{ name: 'ECDH', namedCurve: 'P-256' },
		false,
		[]
	);

	// Generate ephemeral P-256 keypair (fresh per message).
	const ephemeralKeyPair = await crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, false, [
		'deriveBits'
	]);

	// Export ephemeral public key (uncompressed, 65 bytes: 0x04 || x || y).
	const ephemeralPubKeyRaw = new Uint8Array(await crypto.subtle.exportKey('raw', ephemeralKeyPair.publicKey));

	// ECDH: derive 32-byte shared secret (X-coordinate of the shared point).
	const sharedSecretBits = await crypto.subtle.deriveBits(
		{ name: 'ECDH', public: recipientPublicKey },
		ephemeralKeyPair.privateKey,
		256
	);

	// Key derivation: AES-256 key = SHA-256(shared secret).
	const aesKeyMaterial = await crypto.subtle.digest('SHA-256', sharedSecretBits);
	const aesKey = await crypto.subtle.importKey('raw', aesKeyMaterial, 'AES-GCM', false, ['encrypt']);

	// AES-256-GCM encrypt with random nonce and ephemeral public key as AAD.
	const nonce = crypto.getRandomValues(new Uint8Array(12));
	const ciphertextWithTag = new Uint8Array(
		await crypto.subtle.encrypt(
			{ name: 'AES-GCM', iv: nonce, additionalData: ephemeralPubKeyRaw },
			aesKey,
			new TextEncoder().encode(plaintext)
		)
	);

	// Assemble: ephemeral_pub_key (65) || nonce (12) || ciphertext+tag.
	const sealed = new Uint8Array(65 + 12 + ciphertextWithTag.length);
	sealed.set(ephemeralPubKeyRaw, 0);
	sealed.set(nonce, 65);
	sealed.set(ciphertextWithTag, 77);

	return bytesToBase64(sealed);
}

export async function listExtensions(): Promise<ExtensionResponse[]> {
	return request<ExtensionResponse[]>('/extensions');
}

export async function listExtensionProviders(extensionId: string): Promise<ExtensionProviderInfo[]> {
	return request<ExtensionProviderInfo[]>(`/extensions/${encodeURIComponent(extensionId)}/providers`);
}

/**
 * Invoke an extension action via the controller proxy.
 *
 * When `sensitiveParams` is provided and non-empty, it is ECIES-encrypted
 * client-side using the service's P-256 public key before transmission.
 * The controller passes the ciphertext opaquely to the target service, which
 * holds the matching private key and decrypts it locally.
 *
 * @throws If sensitive params are present but no `encryptionPublicKey` is available.
 */
export async function invokeExtensionAction(
	extensionId: string,
	actionId: string,
	params: Record<string, unknown> = {},
	serviceId?: string,
	sensitiveParams?: Record<string, unknown>,
	encryptionPublicKey?: string
): Promise<unknown> {
	const qs = serviceId ? `?service_id=${encodeURIComponent(serviceId)}` : '';
	const path = `/extensions/${encodeURIComponent(extensionId)}/actions/${encodeURIComponent(actionId)}${qs}`;

	const body: Record<string, unknown> = { params };

	if (sensitiveParams && Object.keys(sensitiveParams).length > 0) {
		if (!encryptionPublicKey) {
			throw new Error('Cannot send sensitive parameters: no encryption key is available for this service.');
		}
		body.sensitive_params = await sealedBoxEncrypt(JSON.stringify(sensitiveParams), encryptionPublicKey);
	}

	const resp = await authenticatedFetch(path, {
		method: 'POST',
		headers: { 'Content-Type': 'application/json' },
		body: JSON.stringify(body)
	});
	if (!resp.ok) {
		const msg = await extractErrorMessage(resp);
		throw new Error(msg);
	}
	return resp.json();
}

/**
 * Calls an arbitrary authenticated REST API endpoint and returns the parsed JSON response.
 * Used by extension actions with `api_submit` to bypass the extension proxy.
 *
 * The path may be fully-qualified (e.g. `/api/v1/plugin-configs`) or relative
 * to the API base (e.g. `/plugin-configs`). The leading BASE prefix is stripped
 * before passing to `request()` which re-adds it via `authenticatedFetch`.
 */
export function apiSubmitRequest(
	path: string,
	method: string,
	body: Record<string, unknown>
): Promise<Record<string, unknown>> {
	const relativePath = path.startsWith(BASE) ? path.slice(BASE.length) : path;
	return request<Record<string, unknown>>(relativePath, { method, body: JSON.stringify(body) });
}

/** Performs an authenticated GET request and returns the parsed JSON body. */
export function apiGet<T = unknown>(path: string): Promise<T> {
	const relativePath = path.startsWith(BASE) ? path.slice(BASE.length) : path;
	return request<T>(relativePath);
}

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
