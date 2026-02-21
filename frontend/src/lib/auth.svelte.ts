import type { User, RegisterRequest, LoginRequest, OidcLinkRequest } from './types';
import * as api from './api';

let user = $state<User | null>(null);
let loading = $state(true);

export function getUser(): User | null {
	return user;
}

export function setUser(u: User | null): void {
	user = u;
}

export function getLoading(): boolean {
	return loading;
}

export function setLoading(v: boolean): void {
	loading = v;
}

/** In-memory access token — never persisted to localStorage. */
let accessToken: string | null = null;

export function getAccessToken(): string | null {
	return accessToken;
}

export function setAccessToken(token: string | null): void {
	accessToken = token;
}

export async function initialize() {
	try {
		if (!accessToken) {
			try {
				const refreshed = await api.refreshAccessToken();
				accessToken = refreshed.access_token;
			} catch {
				user = null;
				return;
			}
		}
		const u = await api.me();
		user = u;
	} catch {
		accessToken = null;
		user = null;
	} finally {
		loading = false;
	}
}

export async function handleLogin(data: LoginRequest) {
	const res = await api.login(data);
	accessToken = res.access_token;
	user = res.user;
}

export async function handleRegister(data: RegisterRequest) {
	const res = await api.register(data);
	accessToken = res.access_token;
	user = res.user;
}

export async function handleLogout() {
	try {
		await api.logout();
	} finally {
		accessToken = null;
		user = null;
	}
}

export async function handleOidcLogin(providerId: string) {
	const { authorize_url } = await api.getOidcAuthorizeUrl(providerId);
	if (!authorize_url.startsWith('https://')) {
		throw new Error('Invalid OIDC authorize URL: only HTTPS redirects are allowed');
	}
	window.location.href = authorize_url;
}

export async function handleOidcCallback(code: string) {
	const res = await api.oidcExchange(code);
	accessToken = res.access_token;
	user = res.user;
}

export async function handleOidcCompleteRegistration(registrationCode: string, registrationToken: string) {
	const res = await api.oidcCompleteRegistration(registrationCode, registrationToken);
	accessToken = res.access_token;
	user = res.user;
}

export async function handleOidcLink(linkToken: string, password?: string) {
	const data: OidcLinkRequest = { link_token: linkToken };
	if (password) {
		data.password = password;
	}
	const res = await api.oidcLink(data);
	accessToken = res.access_token;
	user = res.user;
}
