import { get } from 'svelte/store';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import * as api from './api';
import { getAccessToken, initialize, loading, setAccessToken, user } from './auth';
import type { RefreshResponse, User } from './types';

vi.mock('./api', () => ({
	me: vi.fn(),
	refreshAccessToken: vi.fn(),
	login: vi.fn(),
	register: vi.fn(),
	logout: vi.fn(),
	getOidcAuthorizeUrl: vi.fn(),
	oidcExchange: vi.fn(),
	oidcCompleteRegistration: vi.fn(),
	oidcLink: vi.fn()
}));

const sampleUser: User = {
	id: 'user-1',
	email: 'user@example.com',
	first_name: 'Test',
	last_name: 'User',
	permissions: []
};

const sampleRefresh: RefreshResponse = {
	access_token: 'access-token',
	refresh_token: 'refresh-token',
	expires_in: 3600,
	token_type: 'Bearer'
};

beforeEach(() => {
	setAccessToken(null);
	user.set(null);
	loading.set(true);
	vi.clearAllMocks();
});

describe('initialize', () => {
	it('refreshes when no access token and loads user', async () => {
		vi.mocked(api.refreshAccessToken).mockResolvedValue(sampleRefresh);
		vi.mocked(api.me).mockResolvedValue(sampleUser);

		await initialize();

		expect(api.refreshAccessToken).toHaveBeenCalledTimes(1);
		expect(api.me).toHaveBeenCalledTimes(1);
		expect(getAccessToken()).toBe(sampleRefresh.access_token);
		expect(get(user)).toEqual(sampleUser);
		expect(get(loading)).toBe(false);
	});

	it('stays anonymous when refresh fails', async () => {
		vi.mocked(api.refreshAccessToken).mockRejectedValue(new Error('refresh failed'));

		await initialize();

		expect(api.refreshAccessToken).toHaveBeenCalledTimes(1);
		expect(api.me).not.toHaveBeenCalled();
		expect(getAccessToken()).toBeNull();
		expect(get(user)).toBeNull();
		expect(get(loading)).toBe(false);
	});

	it('uses existing access token without refresh', async () => {
		setAccessToken('existing-token');
		vi.mocked(api.me).mockResolvedValue(sampleUser);

		await initialize();

		expect(api.refreshAccessToken).not.toHaveBeenCalled();
		expect(api.me).toHaveBeenCalledTimes(1);
		expect(getAccessToken()).toBe('existing-token');
		expect(get(user)).toEqual(sampleUser);
		expect(get(loading)).toBe(false);
	});
});
