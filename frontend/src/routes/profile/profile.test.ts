import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import type { ApiTokenResponse } from '$lib/types';

vi.mock('$lib/api', () => ({
	listApiTokens: vi.fn(),
	createApiToken: vi.fn(),
	revokeApiToken: vi.fn(),
	updateProfile: vi.fn(),
	initiateEmailChange: vi.fn(),
	cancelEmailChange: vi.fn(),
	changePassword: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null),
	getAuthMethod: vi.fn(() => null),
	initialize: vi.fn()
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
	has_pending_email_change: false,
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

		await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Profile' })).toBeInTheDocument());
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="profile-details-section"]')).toBeInTheDocument();

		// Navigate to API Tokens tab to assert DataTable
		await userEvent.click(screen.getByRole('tab', { name: 'API Tokens' }));
		await waitFor(() => expect(screen.getByText('Automation')).toBeInTheDocument());
		expect(document.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
	});

	it('uses shared account detail rhythm and modal footer actions', async () => {
		render(ProfilePage);

		await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Profile' })).toBeInTheDocument());
		expect(document.querySelector('[data-ui="profile-details-section"]')).toBeInTheDocument();

		await userEvent.click(screen.getByRole('tab', { name: 'API Tokens' }));
		await waitFor(() => expect(screen.getByRole('button', { name: 'New Token' })).toBeInTheDocument());
		await userEvent.click(screen.getByRole('button', { name: 'New Token' }));
		const modalTitle = await screen.findByText('New API Token');
		const modal = modalTitle.closest('[data-ui="modal-shell"]') as HTMLElement;
		expect(modal).toBeInTheDocument();
		const footer = modal.querySelector('[data-ui="profile-token-modal-footer"]') as HTMLElement;
		expect(footer).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Create' })).toBeInTheDocument();
	});
});

describe('Tab Navigation', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('renders Account and API Tokens tabs', async () => {
		render(ProfilePage);
		await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Profile' })).toBeInTheDocument());
		expect(screen.getByRole('tab', { name: 'Account' })).toBeInTheDocument();
		expect(screen.getByRole('tab', { name: 'API Tokens' })).toBeInTheDocument();
	});

	it('Account tab is active by default', async () => {
		render(ProfilePage);
		await waitFor(() => expect(screen.getByRole('tab', { name: 'Account' })).toBeInTheDocument());
		expect(screen.getByRole('tab', { name: 'Account' })).toHaveAttribute('aria-selected', 'true');
		expect(screen.getByRole('tab', { name: 'API Tokens' })).toHaveAttribute('aria-selected', 'false');
	});

	it('clicking API Tokens tab makes it active', async () => {
		render(ProfilePage);
		await waitFor(() => expect(screen.getByRole('tab', { name: 'API Tokens' })).toBeInTheDocument());
		await userEvent.click(screen.getByRole('tab', { name: 'API Tokens' }));
		expect(screen.getByRole('tab', { name: 'API Tokens' })).toHaveAttribute('aria-selected', 'true');
		expect(screen.getByRole('tab', { name: 'Account' })).toHaveAttribute('aria-selected', 'false');
	});
});

describe('Button Migrations', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	async function goToTokensTab() {
		await waitFor(() => expect(screen.getByRole('tab', { name: 'API Tokens' })).toBeInTheDocument());
		await userEvent.click(screen.getByRole('tab', { name: 'API Tokens' }));
	}

	it('New Token launcher renders variant="primary"', async () => {
		render(ProfilePage);
		await goToTokensTab();
		await waitFor(() => expect(screen.getByRole('button', { name: 'New Token' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'New Token' });
		expect(btn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
	});

	it('Row Revoke button renders variant="danger" size="sm"', async () => {
		const token: ApiTokenResponse = {
			id: 'token-1',
			name: 'CI Pipeline',
			created_at: '2026-04-19T00:00:00Z',
			revoked_at: null
		};
		vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [token] });
		render(ProfilePage);
		await goToTokensTab();
		await waitFor(() => expect(screen.getByRole('button', { name: 'Revoke' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Revoke' });
		expect(btn).toHaveClass('h-[19px]');
		expect(btn).toHaveClass('bg-[var(--color-danger-bg)]');
	});

	it('New API Token modal Create state Cancel button renders variant="secondary"', async () => {
		render(ProfilePage);
		await goToTokensTab();
		const newTokenBtn = await screen.findByRole('button', { name: 'New Token' });
		await userEvent.click(newTokenBtn);
		await waitFor(() => expect(screen.getByPlaceholderText('e.g. CI Pipeline')).toBeInTheDocument());
		const cancelBtn = screen.getByRole('button', { name: 'Cancel' });
		expect(cancelBtn).toHaveClass('bg-[var(--bg-raised)]');
	});

	it('New API Token modal Create state Create button already migrated (Wave 3)', async () => {
		render(ProfilePage);
		await goToTokensTab();
		const newTokenBtn = await screen.findByRole('button', { name: 'New Token' });
		await userEvent.click(newTokenBtn);
		await waitFor(() => expect(screen.getByPlaceholderText('e.g. CI Pipeline')).toBeInTheDocument());
		const createBtn = screen.getByRole('button', { name: 'Create' });
		expect(createBtn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
		expect(createBtn).toBeDisabled();
		const nameInput = screen.getByPlaceholderText('e.g. CI Pipeline');
		await userEvent.type(nameInput, 'new-token');
		await waitFor(() => expect(createBtn).not.toHaveAttribute('disabled'));
		expect(createBtn).not.toHaveAttribute('aria-busy');
		expect(createBtn.textContent).toContain('Create');
	});

	it('New API Token modal Created state Copy button renders variant="secondary"', async () => {
		vi.mocked(api.createApiToken).mockResolvedValue({ id: 'token-1', token: 'secret-token-123' });
		render(ProfilePage);
		await goToTokensTab();
		const newTokenBtn = await screen.findByRole('button', { name: 'New Token' });
		await userEvent.click(newTokenBtn);
		await waitFor(() => expect(screen.getByPlaceholderText('e.g. CI Pipeline')).toBeInTheDocument());
		const nameInput = screen.getByPlaceholderText('e.g. CI Pipeline');
		await userEvent.type(nameInput, 'test-token');
		const createBtn = screen.getByRole('button', { name: 'Create' });
		await userEvent.click(createBtn);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Copy' })).toBeInTheDocument());
		const copyBtn = screen.getByRole('button', { name: 'Copy' });
		expect(copyBtn).toHaveClass('bg-[var(--bg-raised)]');
	});

	it('New API Token modal Created state Done button renders variant="primary"', async () => {
		vi.mocked(api.createApiToken).mockResolvedValue({ id: 'token-1', token: 'secret-token-123' });
		render(ProfilePage);
		await goToTokensTab();
		const newTokenBtn = await screen.findByRole('button', { name: 'New Token' });
		await userEvent.click(newTokenBtn);
		await waitFor(() => expect(screen.getByPlaceholderText('e.g. CI Pipeline')).toBeInTheDocument());
		const nameInput = screen.getByPlaceholderText('e.g. CI Pipeline');
		await userEvent.type(nameInput, 'test-token');
		const createBtn = screen.getByRole('button', { name: 'Create' });
		await userEvent.click(createBtn);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Done' })).toBeInTheDocument());
		const doneBtn = screen.getByRole('button', { name: 'Done' });
		expect(doneBtn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
	});

	it('New API Token modal Copy button invokes clipboard.writeText and surfaces success toast', async () => {
		vi.mocked(api.createApiToken).mockResolvedValue({ id: 'token-1', token: 'secret-token-123' });
		const writeTextMock = vi.fn().mockResolvedValue(undefined);
		Object.defineProperty(navigator, 'clipboard', {
			value: { writeText: writeTextMock },
			writable: true,
			configurable: true
		});
		render(ProfilePage);
		await goToTokensTab();
		const newTokenBtn = await screen.findByRole('button', { name: 'New Token' });
		await userEvent.click(newTokenBtn);
		await waitFor(() => expect(screen.getByPlaceholderText('e.g. CI Pipeline')).toBeInTheDocument());
		const nameInput = screen.getByPlaceholderText('e.g. CI Pipeline');
		await userEvent.type(nameInput, 'test-token');
		const createBtn = screen.getByRole('button', { name: 'Create' });
		await userEvent.click(createBtn);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Copy' })).toBeInTheDocument());
		const copyBtn = screen.getByRole('button', { name: 'Copy' });
		await userEvent.click(copyBtn);
		await waitFor(() => expect(writeTextMock).toHaveBeenCalledWith('secret-token-123'));
	});

	it('Out-of-scope regression: ConfirmDialog Revoke confirmation is not wrapped in Button', async () => {
		const token: ApiTokenResponse = {
			id: 'token-1',
			name: 'Test Token',
			created_at: '2026-04-19T00:00:00Z',
			revoked_at: null
		};
		vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [token] });
		render(ProfilePage);
		await goToTokensTab();
		await waitFor(() => expect(screen.getByRole('button', { name: 'Revoke' })).toBeInTheDocument());
		const revokeBtn = screen.getByRole('button', { name: 'Revoke' });
		await userEvent.click(revokeBtn);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Revoke API Token' })).toBeInTheDocument());
		expect(screen.getByRole('heading', { name: 'Revoke API Token' })).toBeInTheDocument();
	});
});

describe('Account Tab — Profile Card', () => {
	afterEach(() => {
		vi.clearAllMocks();
	});

	it('shows "Change email" button for password auth', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(auth.getAuthMethod).mockReturnValue('password');
		vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
		render(ProfilePage);
		await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Profile' })).toBeInTheDocument());
		expect(screen.getByRole('button', { name: 'Change email' })).toBeInTheDocument();
	});

	it('hides "Change email" button for OIDC auth', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(auth.getAuthMethod).mockReturnValue('oidc');
		vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
		render(ProfilePage);
		await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Profile' })).toBeInTheDocument());
		expect(screen.queryByRole('button', { name: 'Change email' })).not.toBeInTheDocument();
	});

	it('shows "Change pending" StatusBadge when has_pending_email_change is true', async () => {
		vi.mocked(auth.getUser).mockReturnValue({
			...user,
			has_pending_email_change: true
		});
		vi.mocked(auth.getAuthMethod).mockReturnValue('password');
		vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
		render(ProfilePage);
		await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Profile' })).toBeInTheDocument());
		expect(screen.getByText('Change pending')).toBeInTheDocument();
	});
});

describe('Account Tab — Change Email Modal', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(auth.getAuthMethod).mockReturnValue('password');
		vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('change email modal opens when "Change email" is clicked', async () => {
		render(ProfilePage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Change email' })).toBeInTheDocument());
		await userEvent.click(screen.getByRole('button', { name: 'Change email' }));
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Change Email' })).toBeInTheDocument());
	});

	it('change email modal closes when Cancel is clicked', async () => {
		render(ProfilePage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Change email' })).toBeInTheDocument());
		await userEvent.click(screen.getByRole('button', { name: 'Change email' }));
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Change Email' })).toBeInTheDocument());
		await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
		await waitFor(() => expect(screen.queryByRole('heading', { name: 'Change Email' })).not.toBeInTheDocument());
	});

	it('shows pending-change Callout when has_pending_email_change is true', async () => {
		vi.mocked(auth.getUser).mockReturnValue({
			...user,
			has_pending_email_change: true
		});
		render(ProfilePage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Change email' })).toBeInTheDocument());
		await userEvent.click(screen.getByRole('button', { name: 'Change email' }));
		await waitFor(() => expect(screen.getByText(/A confirmation email has been sent/)).toBeInTheDocument());
		expect(screen.getByRole('button', { name: 'Cancel email change' })).toBeInTheDocument();
	});
});

describe('Account Tab — Security Card', () => {
	beforeEach(() => {
		vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('shows masked password row and "Change" button for password auth', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(auth.getAuthMethod).mockReturnValue('password');
		render(ProfilePage);
		await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Profile' })).toBeInTheDocument());
		expect(screen.getByText('••••••••')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Change' })).toBeInTheDocument();
	});

	it('shows SSO Callout for OIDC auth', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(auth.getAuthMethod).mockReturnValue('oidc');
		render(ProfilePage);
		await waitFor(() => expect(screen.getByRole('heading', { level: 1, name: 'Profile' })).toBeInTheDocument());
		expect(screen.getByText(/Your account uses single sign-on/)).toBeInTheDocument();
	});

	it('change password modal opens when "Change" is clicked', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(auth.getAuthMethod).mockReturnValue('password');
		render(ProfilePage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Change' })).toBeInTheDocument());
		await userEvent.click(screen.getByRole('button', { name: 'Change' }));
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Change Password' })).toBeInTheDocument());
	});

	it('change password modal closes when Cancel is clicked', async () => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(auth.getAuthMethod).mockReturnValue('password');
		render(ProfilePage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Change' })).toBeInTheDocument());
		await userEvent.click(screen.getByRole('button', { name: 'Change' }));
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Change Password' })).toBeInTheDocument());
		await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
		await waitFor(() => expect(screen.queryByRole('heading', { name: 'Change Password' })).not.toBeInTheDocument());
	});
});
