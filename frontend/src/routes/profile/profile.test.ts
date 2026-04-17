import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
	listApiTokens: vi.fn(),
	createApiToken: vi.fn(),
	revokeApiToken: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null)
}));

vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn()
}));

import ProfilePage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';

const user = {
	id: '00000000-0000-0000-0000-000000000106',
	email: 'profile@example.com',
	first_name: 'Profile',
	last_name: 'User',
	permissions: []
};

describe('Profile Route', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.listApiTokens).mockResolvedValue({
			tokens: [
				{
					id: 'token-1',
					name: 'Automation',
					created_at: '2026-03-10T12:00:00Z',
					revoked_at: null
				}
			]
		});
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('renders shared shell primitives for account and token tables', async () => {
		render(ProfilePage);

		await waitFor(() => expect(screen.getByText('Profile')).toBeInTheDocument());
		expect(screen.getByText('Automation')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="status-badge"]')).toBeInTheDocument();
	});
});
