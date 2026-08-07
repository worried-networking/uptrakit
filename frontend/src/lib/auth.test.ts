import { beforeEach, describe, expect, it, vi } from 'vitest';
import * as api from './api';
import {
	getAccessToken,
	getLoading,
	getUser,
	handleLogin,
	handleLogout,
	handleOidcCallback,
	initialize,
	setAccessToken,
	setLoading,
	setUser
} from './auth.svelte';
import type { AuthResponse, RefreshResponse, UserResponse } from '$lib/api';

vi.mock('./api', () => ({
	me: vi.fn(),
	dedupedRefresh: vi.fn(),
	login: vi.fn(),
	register: vi.fn(),
	logout: vi.fn(),
	oidcAuthorize: vi.fn(),
	oidcExchange: vi.fn(),
	oidcCompleteRegistration: vi.fn(),
	oidcLink: vi.fn()
}));

const sampleUser: UserResponse = {
	id: 'user-1',
	email: 'user@example.com',
	first_name: 'Test',
	last_name: 'User',
	has_pending_email_change: false,
	actions: [],
	authority: 'ok',
	permissions: []
};

const sampleRefresh: RefreshResponse = {
	access_token: 'access-token',
	refresh_token: 'refresh-token',
	expires_in: 3600,
	token_type: 'Bearer'
};

const sampleAuthResponse: AuthResponse = {
	access_token: 'auth-access-token',
	refresh_token: 'auth-refresh-token',
	expires_in: 3600,
	token_type: 'Bearer',
	user: sampleUser
};

beforeEach(() => {
	setAccessToken(null);
	setUser(null);
	setLoading(true);
	vi.clearAllMocks();
});

describe('initialize', () => {
	it('refreshes when no access token and loads user', async () => {
		vi.mocked(api.dedupedRefresh).mockResolvedValue(sampleRefresh);
		vi.mocked(api.me).mockResolvedValue({ data: sampleUser } as unknown as Awaited<ReturnType<typeof api.me>>);

		await initialize();

		expect(api.dedupedRefresh).toHaveBeenCalledTimes(1);
		expect(api.me).toHaveBeenCalledTimes(1);
		expect(getAccessToken()).toBe(sampleRefresh.access_token);
		expect(getUser()).toEqual(sampleUser);
		expect(getLoading()).toBe(false);
	});

	it('stays anonymous when refresh fails', async () => {
		vi.mocked(api.dedupedRefresh).mockRejectedValue(new Error('refresh failed'));

		await initialize();

		expect(api.dedupedRefresh).toHaveBeenCalledTimes(1);
		expect(api.me).not.toHaveBeenCalled();
		expect(getAccessToken()).toBeNull();
		expect(getUser()).toBeNull();
		expect(getLoading()).toBe(false);
	});

	it('uses existing access token without refresh', async () => {
		setAccessToken('existing-token');
		vi.mocked(api.me).mockResolvedValue({ data: sampleUser } as unknown as Awaited<ReturnType<typeof api.me>>);

		await initialize();

		expect(api.dedupedRefresh).not.toHaveBeenCalled();
		expect(api.me).toHaveBeenCalledTimes(1);
		expect(getAccessToken()).toBe('existing-token');
		expect(getUser()).toEqual(sampleUser);
		expect(getLoading()).toBe(false);
	});
});

describe('handleLogin', () => {
	it('sets accessToken and user on success', async () => {
		vi.mocked(api.login).mockResolvedValue({ data: sampleAuthResponse } as unknown as Awaited<
			ReturnType<typeof api.login>
		>);

		await handleLogin({ email: 'user@example.com', password: 'secret' });

		expect(getAccessToken()).toBe('auth-access-token');
		expect(getUser()).toEqual(sampleUser);
	});

	it('propagates errors without touching the token or user', async () => {
		vi.mocked(api.login).mockRejectedValue(new Error('Invalid credentials'));

		await expect(handleLogin({ email: 'user@example.com', password: 'wrong' })).rejects.toThrow('Invalid credentials');

		expect(getAccessToken()).toBeNull();
		expect(getUser()).toBeNull();
	});
});

describe('handleLogout', () => {
	it('clears accessToken and user on success', async () => {
		setAccessToken('some-token');
		setUser(sampleUser);
		vi.mocked(api.logout).mockResolvedValue({ data: undefined } as unknown as Awaited<ReturnType<typeof api.logout>>);

		await handleLogout();

		expect(getAccessToken()).toBeNull();
		expect(getUser()).toBeNull();
	});

	it('clears accessToken and user even when API call throws', async () => {
		setAccessToken('some-token');
		setUser(sampleUser);
		vi.mocked(api.logout).mockRejectedValue(new Error('Network error'));

		// handleLogout uses try/finally — the error propagates, but state is always cleared
		await expect(handleLogout()).rejects.toThrow('Network error');

		// State must be cleared regardless of API error (via finally block)
		expect(getAccessToken()).toBeNull();
		expect(getUser()).toBeNull();
	});
});

describe('handleOidcCallback', () => {
	it('sets accessToken and user on successful oidcExchange', async () => {
		vi.mocked(api.oidcExchange).mockResolvedValue({ data: sampleAuthResponse } as unknown as Awaited<
			ReturnType<typeof api.oidcExchange>
		>);

		await handleOidcCallback('oidc-auth-code');

		expect(api.oidcExchange).toHaveBeenCalledWith({ body: { code: 'oidc-auth-code' } });
		expect(getAccessToken()).toBe('auth-access-token');
		expect(getUser()).toEqual(sampleUser);
	});

	it('propagates errors without setting the token or user', async () => {
		vi.mocked(api.oidcExchange).mockRejectedValue(new Error('OIDC exchange failed'));

		await expect(handleOidcCallback('bad-code')).rejects.toThrow('OIDC exchange failed');

		expect(getAccessToken()).toBeNull();
		expect(getUser()).toBeNull();
	});
});
