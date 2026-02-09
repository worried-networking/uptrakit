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
	CreateMqttClient,
	NetworkSettings,
	OidcLinkRequest,
	OidcProviderResponse,
	PaginatedResponse,
	RefreshResponse,
	RegisterRequest,
	RegistrationSettings,
	RenewServerCertResponse,
	ServiceResponse,
	SystemAlertsResponse,
	UpdateAgentCertificateSettings,
	UpdateAuthenticationSettings,
	UpdateHostRequest,
	UpdateMqttClient,
	UpdateMqttLimitRequest,
	UpdateNetworkSettings,
	UpdateOidcProviderRequest,
	UpdateRegistrationSettings,
	User
} from './types';

const BASE = '/api/v1';

async function extractErrorMessage(res: Response): Promise<string> {
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

export async function refreshAccessToken(): Promise<RefreshResponse> {
	const res = await fetch(`${BASE}/auth/refresh`, {
		method: 'POST',
		headers: { 'Content-Type': 'application/json' },
		credentials: 'same-origin',
		body: JSON.stringify({})
	});

	if (!res.ok) {
		throw new Error('Refresh failed');
	}

	return res.json();
}

/**
 * Performs an authenticated fetch with automatic token refresh on 401.
 * Returns the raw Response for callers to handle body parsing.
 */
async function authenticatedFetch(path: string, options: RequestInit = {}): Promise<Response> {
	const res = await fetch(`${BASE}${path}`, {
		credentials: 'same-origin',
		...options,
		headers: {
			'Content-Type': 'application/json',
			...authHeaders(),
			...(options.headers as Record<string, string> | undefined)
		}
	});

	if (res.status === 401 && getAccessToken()) {
		// Attempt token refresh, deduplicating concurrent attempts.
		// The promise is cleared after it settles (not per-caller) so that
		// concurrent 401 handlers share a single refresh cycle.
		if (!refreshPromise) {
			refreshPromise = refreshAccessToken();
			refreshPromise.then(
				() => { refreshPromise = null; },
				() => { refreshPromise = null; }
			);
		}

		try {
			const refreshed = await refreshPromise;
			setAccessToken(refreshed.access_token);

			// Retry original request with new token
			return fetch(`${BASE}${path}`, {
				credentials: 'same-origin',
				...options,
				headers: {
					'Content-Type': 'application/json',
					Authorization: `Bearer ${refreshed.access_token}`,
					...(options.headers as Record<string, string> | undefined)
				}
			});
		} catch {
			// Refresh failed — clear token and redirect to login
			setAccessToken(null);
			window.location.href = '/login?redirect=' + encodeURIComponent(window.location.pathname + window.location.search);
			throw new Error('Session expired');
		}
	}

	return res;
}

/** Performs an authenticated request and parses the JSON response body. */
async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
	const res = await authenticatedFetch(path, options);
	if (!res.ok) {
		const message = await extractErrorMessage(res);
		throw new Error(message);
	}
	return res.json();
}

/** Performs an authenticated request expecting no response body (204 or empty). */
async function requestVoid(path: string, options: RequestInit = {}): Promise<void> {
	const res = await authenticatedFetch(path, options);
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
	return request(`/auth/oidc/${providerId}/authorize`);
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
		body: JSON.stringify({ registration_code: registrationCode, registration_token: registrationToken })
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
		body: JSON.stringify({ code })
	});
	if (!res.ok) {
		const message = await extractErrorMessage(res);
		throw new Error(message);
	}
	return res.json();
}

export function getServices(status?: string, page?: number, perPage?: number): Promise<PaginatedResponse<ServiceResponse>> {
	const params = new URLSearchParams();
	if (status) params.set('status', status);
	if (page != null) params.set('page', String(page));
	if (perPage != null) params.set('per_page', String(perPage));
	return request(`/services?${params.toString()}`);
}

export function approveService(id: string): Promise<ServiceResponse> {
	return request(`/services/${id}/approve`, { method: 'POST' });
}

export function rejectService(id: string): Promise<ServiceResponse> {
	return request(`/services/${id}/reject`, { method: 'POST' });
}

export function deleteService(id: string): Promise<MessageResponse> {
	return request(`/services/${id}`, { method: 'DELETE' });
}

export function mergeService(targetId: string, sourceId: string): Promise<ServiceResponse> {
	return request(`/services/${targetId}/merge`, {
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
	return request(`/hosts/${id}`);
}

export function updateHost(id: string, data: UpdateHostRequest): Promise<HostResponse> {
	return request(`/hosts/${id}`, { method: 'PUT', body: JSON.stringify(data) });
}

export function deactivateHost(id: string): Promise<MessageResponse> {
	return request(`/hosts/${id}`, { method: 'DELETE' });
}

// --- Settings APIs ---

export function getRegistrationSettings(): Promise<RegistrationSettings> {
	return request('/settings/registration');
}

export function updateRegistrationSettings(
	data: UpdateRegistrationSettings
): Promise<RegistrationSettings> {
	return request('/settings/registration', { method: 'PUT', body: JSON.stringify(data) });
}

export function getAuthenticationSettings(): Promise<AuthenticationSettings> {
	return request('/settings/authentication');
}

export function updateAuthenticationSettings(
	data: UpdateAuthenticationSettings
): Promise<AuthenticationSettings> {
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
	type: 'agent' | 'mqtt' = 'agent'
): Promise<EnrollmentTokenStatus> {
	return request(`/services/enrollment-token/status?type=${type}`);
}

export function getCombinedSettings(): Promise<CombinedSettingsResponse> {
	return request('/settings');
}

export function createEnrollmentToken(
	type: 'agent' | 'mqtt' = 'agent'
): Promise<EnrollmentTokenResponse> {
	return request(`/services/enrollment-token?type=${type}`, { method: 'POST' });
}

export function revokeEnrollmentToken(type: 'agent' | 'mqtt' = 'agent'): Promise<MessageResponse> {
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
	return request(`/settings/mqtt/${id}`, { method: 'PUT', body: JSON.stringify(data) });
}

export function deleteMqttClient(id: string): Promise<void> {
	return requestVoid(`/settings/mqtt/${id}`, { method: 'DELETE' });
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

export function updateOidcProvider(
	id: string,
	data: UpdateOidcProviderRequest
): Promise<OidcProviderResponse> {
	return request(`/settings/oidc-providers/${id}`, {
		method: 'PUT',
		body: JSON.stringify(data)
	});
}

export function deleteOidcProvider(id: string): Promise<void> {
	return requestVoid(`/settings/oidc-providers/${id}`, { method: 'DELETE' });
}

export function activateOidcProvider(id: string): Promise<OidcProviderResponse> {
	return request(`/settings/oidc-providers/${id}/activate`, { method: 'POST' });
}

export function deactivateOidcProvider(id: string): Promise<OidcProviderResponse> {
	return request(`/settings/oidc-providers/${id}/deactivate`, { method: 'POST' });
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
