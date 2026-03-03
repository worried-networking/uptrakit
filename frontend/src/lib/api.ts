import { getAccessToken, setAccessToken, setSessionExpired } from './auth.svelte';
import type {
	AgentCertificateSettings,
	ApiTokenListResponse,
	AssignHostsRequest,
	AuthenticationSettings,
	AuthMethodsResponse,
	AuthResponse,
	AutodiscoveryIgnoreResponse,
	CombinedSettingsResponse,
	CreateApiTokenRequest,
	CreateApiTokenResponse,
	CreateAutodiscoveryIgnoreRequest,
	CreateEnrollmentTokenRequest,
	CreateOidcProviderRequest,
	DiscardDiscoveredResponse,
	EnrollmentTokenCreatedResponse,
	EnrollmentTokenResponse,
	HostResponse,
	LoginRequest,
	MessageResponse,
	MqttClientResponse,
	MqttLimitResponse,
	PaginatedResponse,
	PluginConfigResponse,
	CreateMqttClient,
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
	UpdateMqttClient,
	UpdateMqttLimitRequest,
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
	SystemServiceResponse,
	SystemServicesSettingsResponse,
	UpdateSystemServicesSettingsRequest,
	UpdateSystemServiceRequest,
	PluginTypeInfo,
	HostPackageResponse,
	HostPackageDetailResponse,
	UpdateHostPackageRequest,
	HostPackageIgnoreResponse,
	CreateHostPackageIgnoreRequest,
	ListHostPackagesParams
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

			return fetch(`${BASE}${path}`, {
				credentials: 'same-origin',
				...options,
				headers: {
					'Content-Type': 'application/json',
					Authorization: `Bearer ${refreshed.access_token}`,
					...(options.headers as Record<string, string> | undefined)
				},
				signal: retryCombinedSignal
			});
		} catch (refreshErr) {
			// Network/timeout errors — keep session, surface the error
			if (
				refreshErr instanceof DOMException &&
				(refreshErr.name === 'TimeoutError' || refreshErr.name === 'AbortError')
			) {
				throw new Error('Token refresh timed out. Please try again.');
			}
			if (refreshErr instanceof TypeError) {
				throw new Error('Network error during token refresh. Check your connection.');
			}
			// Server errors (5xx) — keep session, don't force logout
			if (refreshErr instanceof RefreshError && refreshErr.status >= 500) {
				throw new Error('Server error during token refresh. Please try again later.');
			}
			// Real auth failures (4xx) — session is truly invalid.
			// Show a non-blocking banner instead of hard-redirecting so the
			// user can copy any unsaved form state before logging in again.
			setAccessToken(null);
			setSessionExpired(true);
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

export function deactivateHost(id: string): Promise<MessageResponse> {
	return request(`/hosts/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

// --- Host Package APIs ---

export function listHostPackages(
	hostId: string,
	opts?: ListHostPackagesParams
): Promise<PaginatedResponse<HostPackageResponse>> {
	const params = new URLSearchParams();
	if (opts?.page != null) params.set('page', String(opts.page));
	if (opts?.per_page != null) params.set('per_page', String(opts.per_page));
	if (opts?.enabled != null) params.set('enabled', String(opts.enabled));
	if (opts?.has_update != null) params.set('has_update', String(opts.has_update));
	if (opts?.category) params.set('category', opts.category);
	if (opts?.search) params.set('search', opts.search);
	const query = params.toString();
	return request(`/hosts/${encodeURIComponent(hostId)}/packages${query ? `?${query}` : ''}`);
}

export function getHostPackage(hostId: string, packageId: string): Promise<HostPackageDetailResponse> {
	return request(`/hosts/${encodeURIComponent(hostId)}/packages/${encodeURIComponent(packageId)}`);
}

export function updateHostPackage(
	hostId: string,
	packageId: string,
	data: UpdateHostPackageRequest
): Promise<HostPackageResponse> {
	return request(`/hosts/${encodeURIComponent(hostId)}/packages/${encodeURIComponent(packageId)}`, {
		method: 'PUT',
		body: JSON.stringify(data)
	});
}

export function deleteHostPackage(hostId: string, packageId: string, ignore = false): Promise<void> {
	const query = ignore ? '?ignore=true' : '';
	return requestVoid(`/hosts/${encodeURIComponent(hostId)}/packages/${encodeURIComponent(packageId)}${query}`, {
		method: 'DELETE'
	});
}

export function listHostPackageIgnores(hostId: string): Promise<HostPackageIgnoreResponse[]> {
	return request(`/hosts/${encodeURIComponent(hostId)}/package-ignores`);
}

export function createHostPackageIgnore(
	hostId: string,
	data: CreateHostPackageIgnoreRequest
): Promise<HostPackageIgnoreResponse> {
	return request(`/hosts/${encodeURIComponent(hostId)}/package-ignores`, {
		method: 'POST',
		body: JSON.stringify(data)
	});
}

export function deleteHostPackageIgnore(hostId: string, ignoreId: string): Promise<void> {
	return requestVoid(`/hosts/${encodeURIComponent(hostId)}/package-ignores/${encodeURIComponent(ignoreId)}`, {
		method: 'DELETE'
	});
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
	return request('/settings/network');
}

export function updateNetworkSettings(data: UpdateNetworkSettings): Promise<NetworkSettings> {
	return request('/settings/network', { method: 'PUT', body: JSON.stringify(data) });
}

// --- MQTT Client APIs ---

export function getMqttClients(): Promise<MqttClientResponse[]> {
	return request('/settings/mqtt');
}

export function createMqttClient(data: CreateMqttClient): Promise<MqttClientResponse> {
	return request('/settings/mqtt', { method: 'POST', body: JSON.stringify(data) });
}

export function updateMqttClient(id: string, data: UpdateMqttClient): Promise<MqttClientResponse> {
	return request(`/settings/mqtt/${encodeURIComponent(id)}`, { method: 'PUT', body: JSON.stringify(data) });
}

export function deleteMqttClient(id: string): Promise<void> {
	return requestVoid(`/settings/mqtt/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

export function getMqttLimit(): Promise<MqttLimitResponse> {
	return request('/settings/mqtt/limit');
}

export function updateMqttLimit(data: UpdateMqttLimitRequest): Promise<MqttLimitResponse> {
	return request('/settings/mqtt/limit', { method: 'PUT', body: JSON.stringify(data) });
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
	return request('/settings/nats');
}

export function updateNatsSettings(data: UpdateNatsSettingsRequest): Promise<NatsSettingsResponse> {
	return request('/settings/nats', { method: 'PUT', body: JSON.stringify(data) });
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

export function getSystemServicesSettings(): Promise<SystemServicesSettingsResponse> {
	return request('/settings/system-services');
}

export function updateSystemServicesSettings(
	data: UpdateSystemServicesSettingsRequest
): Promise<SystemServicesSettingsResponse> {
	return request('/settings/system-services', { method: 'PUT', body: JSON.stringify(data) });
}

// --- Plugin Types & Configs ---

/** Fetch all known plugin types with display names and capabilities from the registry. */
export function listPluginTypes(): Promise<PluginTypeInfo[]> {
	return request<PluginTypeInfo[]>('/plugin-types');
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
	discoveryState?: 'pending' | 'approved'
): Promise<PaginatedResponse<SoftwareItemResponse>> {
	const params = new URLSearchParams();
	if (page != null) params.set('page', String(page));
	if (perPage != null) params.set('per_page', String(perPage));
	if (discoveryState != null) params.set('discovery_state', discoveryState);
	const query = params.toString();
	return request(`/software-items${query ? `?${query}` : ''}`);
}

export function createSoftwareItem(data: CreateSoftwareItemRequest): Promise<SoftwareItemResponse> {
	return request('/software-items', {
		method: 'POST',
		body: JSON.stringify({ name: data.name, enabled: data.enabled })
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

export function unassignHostFromSoftwareItemWithIgnore(itemId: string, hostId: string): Promise<void> {
	return requestVoid(`/software-items/${encodeURIComponent(itemId)}/hosts/${encodeURIComponent(hostId)}?ignore=true`, {
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

export function checkSoftwareItemVersions(itemId: string): Promise<TriggerVersionCheckResponse> {
	return request(`/software-items/${encodeURIComponent(itemId)}/check-versions`, { method: 'POST' });
}

export function approveSoftwareItem(id: string): Promise<SoftwareItemResponse> {
	return request(`/software-items/${encodeURIComponent(id)}/approve`, { method: 'POST' });
}

export function triggerHostDiscovery(hostId: string): Promise<TriggerDiscoveryResponse> {
	return request(`/hosts/${encodeURIComponent(hostId)}/discover`, { method: 'POST' });
}

export function discardHostDiscovered(hostId: string, pluginConfigId?: string): Promise<DiscardDiscoveredResponse> {
	const params = new URLSearchParams();
	if (pluginConfigId != null) params.set('plugin_config_id', pluginConfigId);
	const query = params.toString();
	return request(`/hosts/${encodeURIComponent(hostId)}/discovered${query ? `?${query}` : ''}`, {
		method: 'DELETE'
	});
}

export function getAutodiscoveryIgnores(
	page?: number,
	perPage?: number,
	pluginConfigId?: string
): Promise<PaginatedResponse<AutodiscoveryIgnoreResponse>> {
	const params = new URLSearchParams();
	if (page != null) params.set('page', String(page));
	if (perPage != null) params.set('per_page', String(perPage));
	if (pluginConfigId != null) params.set('plugin_config_id', pluginConfigId);
	const query = params.toString();
	return request(`/autodiscovery/ignores${query ? `?${query}` : ''}`);
}

export function createAutodiscoveryIgnore(req: CreateAutodiscoveryIgnoreRequest): Promise<AutodiscoveryIgnoreResponse> {
	return request('/autodiscovery/ignores', { method: 'POST', body: JSON.stringify(req) });
}

export function deleteAutodiscoveryIgnore(id: string): Promise<void> {
	return requestVoid(`/autodiscovery/ignores/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

// Software items - update
export async function updateSoftwareItem(id: string, data: UpdateSoftwareItemRequest): Promise<SoftwareItemResponse> {
	return request<SoftwareItemResponse>(`/software-items/${id}`, { method: 'PUT', body: JSON.stringify(data) });
}

// Software items - trigger update on a specific host
export async function triggerSoftwareUpdate(
	itemId: string,
	hostId: string,
	req: TriggerUpdateRequest
): Promise<TriggerUpdateResponse> {
	return request<TriggerUpdateResponse>(`/software-items/${itemId}/hosts/${hostId}/update`, {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

// Software items - check versions on a specific host
export async function checkSoftwareItemVersionsHost(
	itemId: string,
	hostId: string
): Promise<TriggerVersionCheckResponse> {
	return request<TriggerVersionCheckResponse>(`/software-items/${itemId}/hosts/${hostId}/check-versions`, {
		method: 'POST'
	});
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
	return request<UpdateHistoryResponse>(`/update-history/${id}`);
}

// Scheduler tasks
export async function listSchedulerTasks(): Promise<ScheduledTaskResponse[]> {
	return request<ScheduledTaskResponse[]>('/scheduler/tasks');
}

export async function getSchedulerTask(id: string): Promise<ScheduledTaskResponse> {
	return request<ScheduledTaskResponse>(`/scheduler/tasks/${id}`);
}

export async function updateSchedulerTask(
	id: string,
	data: UpdateScheduledTaskRequest
): Promise<ScheduledTaskResponse> {
	return request<ScheduledTaskResponse>(`/scheduler/tasks/${id}`, { method: 'PUT', body: JSON.stringify(data) });
}

export async function triggerSchedulerTask(id: string): Promise<TriggerScheduledTaskResponse> {
	return request<TriggerScheduledTaskResponse>(`/scheduler/tasks/${id}/trigger`, { method: 'POST' });
}

// Plugin configs - CRUD
export async function getPluginConfig(id: string): Promise<PluginConfigResponse> {
	return request<PluginConfigResponse>(`/plugin-configs/${id}`);
}

export async function createPluginConfig(data: CreatePluginConfigRequest): Promise<PluginConfigResponse> {
	return request<PluginConfigResponse>('/plugin-configs', { method: 'POST', body: JSON.stringify(data) });
}

export async function updatePluginConfig(id: string, data: UpdatePluginConfigRequest): Promise<PluginConfigResponse> {
	return request<PluginConfigResponse>(`/plugin-configs/${id}`, { method: 'PUT', body: JSON.stringify(data) });
}

export async function deletePluginConfig(id: string): Promise<void> {
	return requestVoid(`/plugin-configs/${id}`, { method: 'DELETE' });
}

export async function triggerPluginConfigDiscovery(id: string): Promise<TriggerDiscoveryResponse> {
	return request<TriggerDiscoveryResponse>(`/plugin-configs/${id}/discover`, { method: 'POST' });
}

export async function discardPluginConfigDiscovered(id: string): Promise<DiscardDiscoveredResponse> {
	return request<DiscardDiscoveredResponse>(`/plugin-configs/${id}/discovered`, { method: 'DELETE' });
}

// API tokens
export async function listApiTokens(): Promise<ApiTokenListResponse> {
	return request<ApiTokenListResponse>('/auth/api-tokens');
}

export async function createApiToken(data: CreateApiTokenRequest): Promise<CreateApiTokenResponse> {
	return request<CreateApiTokenResponse>('/auth/api-tokens', { method: 'POST', body: JSON.stringify(data) });
}

export async function revokeApiToken(id: string): Promise<void> {
	return requestVoid(`/auth/api-tokens/${id}`, { method: 'DELETE' });
}

// CA rotation
export async function rotateCA(): Promise<RotateCaResponse> {
	return request<RotateCaResponse>('/settings/ca/rotate', { method: 'POST' });
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
