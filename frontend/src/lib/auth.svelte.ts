import type { User, RegisterRequest, LoginRequest, OidcLinkRequest } from '$lib/api';
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
				const refreshed = await api.dedupedRefresh();
				setAccessToken(refreshed.access_token);
			} catch {
				user = null;
				return;
			}
		}
		const { data: u } = await api.me();
		user = u as unknown as User;
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
	const { data: res } = await api.login({ body: data });
	setAccessToken(res.access_token);
	user = res.user as unknown as User;
	setSessionExpired(false);
}

export async function handleRegister(data: RegisterRequest) {
	const { data: res } = await api.register({ body: data });
	setAccessToken(res.access_token);
	user = res.user as unknown as User;
	setSessionExpired(false);
}

export async function handleLogout() {
	try {
		await api.logout({ body: {} });
	} finally {
		setAccessToken(null);
		user = null;
		// Always clear a stale session-expired banner on logout so it doesn't
		// bleed into the next login attempt.
		setSessionExpired(false);
	}
}

export async function handleOidcLogin(providerId: string) {
	const {
		data: { authorize_url }
	} = await api.oidcAuthorize({ path: { provider_id: providerId } });
	if (!authorize_url.startsWith('https://')) {
		throw new Error('Invalid OIDC authorize URL: only HTTPS redirects are allowed');
	}
	window.location.href = authorize_url;
}

export async function handleOidcCallback(code: string) {
	const { data: res } = await api.oidcExchange({ body: { code } });
	setAccessToken(res.access_token);
	user = res.user as unknown as User;
	setSessionExpired(false);
}

export async function handleOidcCompleteRegistration(registrationCode: string, registrationToken: string) {
	const { data: res } = await api.oidcCompleteRegistration({
		body: { registration_code: registrationCode, registration_token: registrationToken }
	});
	setAccessToken(res.access_token);
	user = res.user as unknown as User;
	setSessionExpired(false);
}

export async function handleOidcLink(linkToken: string, password?: string) {
	const data: OidcLinkRequest = { link_token: linkToken };
	if (password) {
		data.password = password;
	}
	const { data: res } = await api.oidcLink({ body: data });
	setAccessToken(res.access_token);
	user = res.user as unknown as User;
	setSessionExpired(false);
}
