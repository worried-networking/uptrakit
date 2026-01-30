import type {
	AgentCertificateSettings,
	AgentResponse,
	AuthenticationSettings,
	AuthMethodsResponse,
	AuthResponse,
	CreateOidcProviderRequest,
	EnrollmentTokenResponse,
	EnrollmentTokenStatus,
	LoginRequest,
	MessageResponse,
	OidcLinkRequest,
	OidcProviderResponse,
	RegisterRequest,
	RegistrationSettings,
	UpdateAgentCertificateSettings,
	UpdateAuthenticationSettings,
	UpdateOidcProviderRequest,
	UpdateRegistrationSettings,
	User
} from './types';

const BASE = '/api/v1';

function authHeaders(): Record<string, string> {
	const token = localStorage.getItem('token');
	return token ? { Authorization: `Bearer ${token}` } : {};
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
	return request('/auth/logout', { method: 'POST' });
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
