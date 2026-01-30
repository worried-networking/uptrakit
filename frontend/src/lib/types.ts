export interface User {
	id: string;
	email: string;
	first_name: string;
	last_name: string;
	roles: string[];
}

export interface AuthResponse {
	token: string;
	user: User;
}

export interface RegisterRequest {
	email: string;
	first_name: string;
	last_name: string;
	password: string;
	registration_token?: string;
}

export interface LoginRequest {
	email: string;
	password: string;
}

export interface AgentResponse {
	id: string;
	hostname: string;
	friendly_name: string;
	ip_address: string | null;
	status: 'pending' | 'approved' | 'rejected';
	last_seen_at: string | null;
	created_at: string;
	updated_at: string;
}

export interface MessageResponse {
	message: string;
}
