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
