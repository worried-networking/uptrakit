import type {
	AgentResponse,
	AuthResponse,
	LoginRequest,
	MessageResponse,
	RegisterRequest,
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
