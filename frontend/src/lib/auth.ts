import { writable } from 'svelte/store';
import type { User, RegisterRequest, LoginRequest, OidcLinkRequest } from './types';
import * as api from './api';

export const user = writable<User | null>(null);
export const loading = writable(true);

/** In-memory access token — never persisted to localStorage. */
let accessToken: string | null = null;

export function getAccessToken(): string | null {
	return accessToken;
}

export function setAccessToken(token: string | null): void {
	accessToken = token;
}

export async function initialize() {
	if (!accessToken) {
		loading.set(false);
		return;
	}
	try {
		const u = await api.me();
		user.set(u);
	} catch {
		accessToken = null;
	} finally {
		loading.set(false);
	}
}

export async function handleLogin(data: LoginRequest) {
	const res = await api.login(data);
	accessToken = res.access_token;
	user.set(res.user);
}

export async function handleRegister(data: RegisterRequest) {
	const res = await api.register(data);
	accessToken = res.access_token;
	user.set(res.user);
}

export async function handleLogout() {
	try {
		await api.logout();
	} finally {
		accessToken = null;
		user.set(null);
	}
}

export async function handleOidcLogin(providerId: string) {
	const { authorize_url } = await api.getOidcAuthorizeUrl(providerId);
	window.location.href = authorize_url;
}

export async function handleOidcCallback(code: string) {
	const res = await api.oidcExchange(code);
	accessToken = res.access_token;
	user.set(res.user);
}

export async function handleOidcCompleteRegistration(registrationCode: string, registrationToken: string) {
	const res = await api.oidcCompleteRegistration(registrationCode, registrationToken);
	accessToken = res.access_token;
	user.set(res.user);
}

export async function handleOidcLink(linkToken: string, password?: string) {
	const data: OidcLinkRequest = { link_token: linkToken };
	if (password) {
		data.password = password;
	}
	const res = await api.oidcLink(data);
	accessToken = res.access_token;
	user.set(res.user);
}
