import type {
	AgentCertificateSettings,
	AgentResponse,
	AuthenticationSettings,
	AuthMethodsResponse,
	AuthResponse,
	CreateOidcProviderRequest,
	EnrollmentTokenResponse,
	EnrollmentTokenStatus,
	HostResponse,
	LoginRequest,
	MessageResponse,
	MqttClientResponse,
	CreateMqttClient,
	NetworkSettings,
	OidcLinkRequest,
	OidcProviderResponse,
	RefreshResponse,
	RegisterRequest,
	RegistrationSettings,
	RenewServerCertResponse,
	SystemAlertsResponse,
	UpdateAgentCertificateSettings,
	UpdateAuthenticationSettings,
	UpdateHostRequest,
	UpdateMqttClient,
	UpdateNetworkSettings,
	UpdateOidcProviderRequest,
	UpdateRegistrationSettings,
	User
} from './types';

const BASE = '/api/v1';

function authHeaders(): Record<string, string> {
	const token = localStorage.getItem('access_token');
	return token ? { Authorization: `Bearer ${token}` } : {};
}

let refreshPromise: Promise<RefreshResponse> | null = null;

async function refreshAccessToken(): Promise<RefreshResponse> {
	const refreshToken = localStorage.getItem('refresh_token');
	if (!refreshToken) {
		throw new Error('No refresh token');
	}

	const res = await fetch(`${BASE}/auth/refresh`, {
		method: 'POST',
		headers: { 'Content-Type': 'application/json' },
		body: JSON.stringify({ refresh_token: refreshToken })
	});

	if (!res.ok) {
		throw new Error('Refresh failed');
	}

	return res.json();
}

async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
	const res = await fetch(`${BASE}${path}`, {
		headers: {
			'Content-Type': 'application/json',
			...authHeaders(),
			...(options.headers as Record<string, string> | undefined)
		},
		...options
	});

	if (res.status === 401 && localStorage.getItem('refresh_token')) {
		// Attempt token refresh, deduplicating concurrent attempts
		try {
			if (!refreshPromise) {
				refreshPromise = refreshAccessToken();
			}
			const refreshed = await refreshPromise;
			localStorage.setItem('access_token', refreshed.access_token);

			// Retry original request with new token
			const retryRes = await fetch(`${BASE}${path}`, {
				headers: {
					'Content-Type': 'application/json',
					Authorization: `Bearer ${refreshed.access_token}`,
					...(options.headers as Record<string, string> | undefined)
				},
				...options
			});

			if (!retryRes.ok) {
				const text = await retryRes.text();
				throw new Error(text || retryRes.statusText);
			}
			if (retryRes.status === 204) return undefined as T;
			return retryRes.json();
		} catch {
			// Refresh failed — clear tokens and redirect to login
			localStorage.removeItem('access_token');
			localStorage.removeItem('refresh_token');
			window.location.href = '/login';
			throw new Error('Session expired');
		} finally {
			refreshPromise = null;
		}
	}

	if (!res.ok) {
		const text = await res.text();
		throw new Error(text || res.statusText);
	}
	if (res.status === 204) return undefined as T;
	return res.json();
}

export function register(data: RegisterRequest): Promise<AuthResponse> {
	return request('/auth/register', { method: 'POST', body: JSON.stringify(data) });
}

export function login(data: LoginRequest): Promise<AuthResponse> {
	return request('/auth/login', { method: 'POST', body: JSON.stringify(data) });
}

export function logout(): Promise<void> {
	const refreshToken = localStorage.getItem('refresh_token');
	return request('/auth/logout', {
		method: 'POST',
		body: JSON.stringify({ refresh_token: refreshToken || '' })
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

export async function oidcExchange(code: string): Promise<AuthResponse> {
	// Direct fetch without auth headers — this is a public endpoint
	const res = await fetch(`${BASE}/auth/oidc/exchange`, {
		method: 'POST',
		headers: { 'Content-Type': 'application/json' },
		body: JSON.stringify({ code })
	});
	if (!res.ok) {
		const text = await res.text();
		throw new Error(text || res.statusText);
	}
	return res.json();
}

export function getAgents(status?: string): Promise<AgentResponse[]> {
	const query = status ? `?status=${status}` : '';
	return request(`/agents${query}`);
}

export function approveAgent(id: string): Promise<AgentResponse> {
	return request(`/agents/${id}/approve`, { method: 'POST' });
}

export function rejectAgent(id: string): Promise<AgentResponse> {
	return request(`/agents/${id}/reject`, { method: 'POST' });
}

export function deleteAgent(id: string): Promise<MessageResponse> {
	return request(`/agents/${id}`, { method: 'DELETE' });
}

export function mergeAgent(targetId: string, sourceId: string): Promise<AgentResponse> {
	return request(`/agents/${targetId}/merge`, {
		method: 'POST',
		body: JSON.stringify({ source_id: sourceId })
	});
}

// --- Host APIs ---

export function getHosts(): Promise<HostResponse[]> {
	return request('/hosts');
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

export function getEnrollmentTokenStatus(): Promise<EnrollmentTokenStatus> {
	return request('/agents/enrollment-token/status');
}

export function createEnrollmentToken(): Promise<EnrollmentTokenResponse> {
	return request('/agents/enrollment-token', { method: 'POST' });
}

export function revokeEnrollmentToken(): Promise<MessageResponse> {
	return request('/agents/enrollment-token', { method: 'DELETE' });
}

// --- Network Settings APIs ---

export function getNetworkSettings(): Promise<NetworkSettings> {
	return request('/settings/network');
}

export function updateNetworkSettings(data: UpdateNetworkSettings): Promise<NetworkSettings> {
	return request('/settings/network', { method: 'PUT', body: JSON.stringify(data) });
}

// --- MQTT Client APIs ---

export function getMqttClient(): Promise<MqttClientResponse> {
	return request('/settings/mqtt');
}

export function createMqttClient(data: CreateMqttClient): Promise<MqttClientResponse> {
	return request('/settings/mqtt', { method: 'POST', body: JSON.stringify(data) });
}

export function updateMqttClient(data: UpdateMqttClient): Promise<MqttClientResponse> {
	return request('/settings/mqtt', { method: 'PUT', body: JSON.stringify(data) });
}

export function deleteMqttClient(): Promise<void> {
	return request('/settings/mqtt', { method: 'DELETE' });
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
	return request(`/settings/oidc-providers/${id}`, { method: 'DELETE' });
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
