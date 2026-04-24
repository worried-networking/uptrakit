import type { User, RegisterRequest, LoginRequest, OidcLinkRequest } from './types';
import * as api from './api';
import { getAccessToken, setAccessToken, getSessionExpired, setSessionExpired } from './token-store.svelte';

export { getAccessToken, setAccessToken, getSessionExpired, setSessionExpired };

/** Decode a JWT payload without verification (client-side only). */
function parseJwt(token: string): Record<string, unknown> {
	try {
		const base64 = token.split('.')[1].replace(/-/g, '+').replace(/_/g, '/');
		return JSON.parse(atob(base64)) as Record<string, unknown>;
	} catch {
		return {};
	}
}

/** Returns the auth_method claim from the current access token, or null. */
export function getAuthMethod(): string | null {
	const token = getAccessToken();
	if (!token) return null;
	const claims = parseJwt(token);
	return typeof claims['auth_method'] === 'string' ? claims['auth_method'] : null;
}

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

export async function initialize() {
	try {
		if (!getAccessToken()) {
			try {
				const refreshed = await api.refreshAccessToken();
				setAccessToken(refreshed.access_token);
			} catch {
				user = null;
				return;
			}
		}
		const u = await api.me();
		user = u;
		// Clear any prior session-expired banner on successful auth
		setSessionExpired(false);
	} catch {
		setAccessToken(null);
		user = null;
	} finally {
		loading = false;
	}
}

export async function handleLogin(data: LoginRequest) {
	const res = await api.login(data);
	setAccessToken(res.access_token);
	user = res.user;
	setSessionExpired(false);
}

export async function handleRegister(data: RegisterRequest) {
	const res = await api.register(data);
	setAccessToken(res.access_token);
	user = res.user;
	setSessionExpired(false);
}

export async function handleLogout() {
	try {
		await api.logout();
	} finally {
		setAccessToken(null);
		user = null;
		// Always clear a stale session-expired banner on logout so it doesn't
		// bleed into the next login attempt.
		setSessionExpired(false);
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
	setAccessToken(res.access_token);
	user = res.user;
	setSessionExpired(false);
}

export async function handleOidcCompleteRegistration(registrationCode: string, registrationToken: string) {
	const res = await api.oidcCompleteRegistration(registrationCode, registrationToken);
	setAccessToken(res.access_token);
	user = res.user;
	setSessionExpired(false);
}

export async function handleOidcLink(linkToken: string, password?: string) {
	const data: OidcLinkRequest = { link_token: linkToken };
	if (password) {
		data.password = password;
	}
	const res = await api.oidcLink(data);
	setAccessToken(res.access_token);
	user = res.user;
	setSessionExpired(false);
}
