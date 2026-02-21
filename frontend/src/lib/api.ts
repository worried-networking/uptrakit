import { getAccessToken, setAccessToken } from './auth';
import type {
	AgentCertificateSettings,
	AuthenticationSettings,
	AuthMethodsResponse,
	AuthResponse,
	CombinedSettingsResponse,
	CreateOidcProviderRequest,
	EnrollmentTokenResponse,
	EnrollmentTokenStatus,
	HostResponse,
	LoginRequest,
	MessageResponse,
	MqttClientResponse,
	MqttLimitResponse,
	ProviderConfigResponse,
	CreateMqttClient,
	CreateSoftwareItemRequest,
	NetworkSettings,
	OidcLinkRequest,
	OidcProviderResponse,
	PaginatedResponse,
	RefreshResponse,
	RegisterRequest,
	RegistrationSettings,
	RenewServerCertResponse,
	ServiceResponse,
	SoftwareItemResponse,
	SystemAlertsResponse,
	UpdateAgentCertificateSettings,
	UpdateAuthenticationSettings,
	UpdateHostRequest,
	UpdateMqttClient,
	UpdateMqttLimitRequest,
	UpdateNetworkSettings,
	UpdateOidcProviderRequest,
	UpdateRegistrationSettings,
	UpdateServiceRequest,
	User
} from './types';

const BASE = '/api/v1';
const DEFAULT_TIMEOUT_MS = 30_000;
const REFRESH_TIMEOUT_MS = 10_000;

export async function extractErrorMessage(res: Response): Promise<string> {
	const text = await res.text();
	if (!text) return res.statusText;
	try {
		const parsed = JSON.parse(text);
		if (typeof parsed === 'object' && parsed !== null && typeof parsed.error === 'string') {
			return parsed.error;
		}
	} catch {
		/* Not JSON */
	}
	return text;
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
			// Real auth failures (4xx) — session is truly invalid
			setAccessToken(null);
			window.location.href = '/login?redirect=' + encodeURIComponent(window.location.pathname + window.location.search);
			throw new Error('Session expired');
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
	type?: string;
	status?: string;
	page?: number;
	perPage?: number;
}): Promise<PaginatedResponse<ServiceResponse>> {
	const params = new URLSearchParams();
	if (options?.type) params.set('type', options.type);
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

export function getEnrollmentTokenStatus(
	type: 'agent' | 'mqtt' | 'ssh_agent' = 'agent'
): Promise<EnrollmentTokenStatus> {
	return request(`/services/enrollment-token/status?type=${type}`);
}

export function getCombinedSettings(): Promise<CombinedSettingsResponse> {
	return request('/settings');
}

export function createEnrollmentToken(
	type: 'agent' | 'mqtt' | 'ssh_agent' = 'agent'
): Promise<EnrollmentTokenResponse> {
	return request(`/services/enrollment-token?type=${type}`, { method: 'POST' });
}

export function revokeEnrollmentToken(type: 'agent' | 'mqtt' | 'ssh_agent' = 'agent'): Promise<MessageResponse> {
	return request(`/services/enrollment-token?type=${type}`, { method: 'DELETE' });
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

// --- Software Items ---

export function getProviderConfigs(
	page?: number,
	perPage?: number
): Promise<PaginatedResponse<ProviderConfigResponse>> {
	const params = new URLSearchParams();
	if (page != null) params.set('page', String(page));
	if (perPage != null) params.set('per_page', String(perPage));
	const query = params.toString();
	return request(`/provider-configs${query ? `?${query}` : ''}`);
}

export function getSoftwareItems(page?: number, perPage?: number): Promise<PaginatedResponse<SoftwareItemResponse>> {
	const params = new URLSearchParams();
	if (page != null) params.set('page', String(page));
	if (perPage != null) params.set('per_page', String(perPage));
	const query = params.toString();
	return request(`/software-items${query ? `?${query}` : ''}`);
}

export function createSoftwareItem(data: CreateSoftwareItemRequest): Promise<SoftwareItemResponse> {
	return request('/software-items', {
		method: 'POST',
		body: JSON.stringify(data)
	});
}
