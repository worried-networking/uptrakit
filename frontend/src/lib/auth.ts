import { writable } from 'svelte/store';
import type { User, RegisterRequest, LoginRequest, OidcLinkRequest } from './types';
import * as api from './api';

export const user = writable<User | null>(null);
export const loading = writable(true);

export async function initialize() {
	const token = localStorage.getItem('token');
	if (!token) {
		loading.set(false);
		return;
	}
	try {
		const u = await api.me();
		user.set(u);
	} catch {
		localStorage.removeItem('token');
	} finally {
		loading.set(false);
	}
}

export async function handleLogin(data: LoginRequest) {
	const res = await api.login(data);
	localStorage.setItem('token', res.token);
	user.set(res.user);
}

export async function handleRegister(data: RegisterRequest) {
	const res = await api.register(data);
	localStorage.setItem('token', res.token);
	user.set(res.user);
}

export async function handleLogout() {
	try {
		await api.logout();
	} finally {
		localStorage.removeItem('token');
		user.set(null);
	}
}

export async function handleOidcLogin(providerId: string) {
	const { authorize_url } = await api.getOidcAuthorizeUrl(providerId);
	window.location.href = authorize_url;
}

export async function handleOidcCallback(token: string) {
	localStorage.setItem('token', token);
	const u = await api.me();
	user.set(u);
}

export async function handleOidcLink(linkToken: string, password?: string) {
	const data: OidcLinkRequest = { link_token: linkToken };
	if (password) {
		data.password = password;
	}
	const res = await api.oidcLink(data);
	localStorage.setItem('token', res.token);
	user.set(res.user);
}
