import { writable } from 'svelte/store';
import type { User, RegisterRequest, LoginRequest } from './types';
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
