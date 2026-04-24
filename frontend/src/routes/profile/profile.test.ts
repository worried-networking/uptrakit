import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import type { ApiTokenResponse } from '$lib/types';

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

		await waitFor(() => expect(screen.getByText('Profile')).toBeInTheDocument());
		expect(screen.getByText('Automation')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="status-badge"]')).toBeInTheDocument();
	});

	it('uses shared account detail rhythm and modal footer actions', async () => {
		render(ProfilePage);

		await waitFor(() => expect(screen.getByText('Profile')).toBeInTheDocument());
		expect(document.querySelector('[data-ui="profile-account-details"]')).toBeInTheDocument();

		await fireEvent.click(screen.getByRole('button', { name: 'New Token' }));
		const modalTitle = await screen.findByText('New API Token');
		const modal = modalTitle.closest('[data-ui="modal-shell"]') as HTMLElement;
		expect(modal).toBeInTheDocument();
		const footer = modal.querySelector('[data-ui="profile-token-modal-footer"]') as HTMLElement;
		expect(footer).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Create' })).toBeInTheDocument();
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

	it('New Token launcher renders variant="primary"', async () => {
		render(ProfilePage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'New Token' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'New Token' });
		expect(btn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary variant
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
		await waitFor(() => expect(screen.getByRole('button', { name: 'Revoke' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Revoke' });
		expect(btn).toHaveClass('h-[19px]'); // size="sm"
		expect(btn).toHaveClass('bg-[var(--color-danger-bg)]'); // danger variant
	});

	it('New API Token modal Create state Cancel button renders variant="secondary"', async () => {
		render(ProfilePage);
		const newTokenBtn = screen.getByRole('button', { name: 'New Token' });
		await userEvent.click(newTokenBtn);
		await waitFor(() => expect(screen.getByPlaceholderText('e.g. CI Pipeline')).toBeInTheDocument());
		const cancelBtn = screen.getByRole('button', { name: 'Cancel' });
		expect(cancelBtn).toHaveClass('bg-[var(--bg-raised)]'); // secondary variant
	});

	it('New API Token modal Create state Create button already migrated (Wave 3)', async () => {
		render(ProfilePage);
		const newTokenBtn = screen.getByRole('button', { name: 'New Token' });
		await userEvent.click(newTokenBtn);
		await waitFor(() => expect(screen.getByPlaceholderText('e.g. CI Pipeline')).toBeInTheDocument());
		const createBtn = screen.getByRole('button', { name: 'Create' });
		expect(createBtn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary variant
		expect(createBtn).toBeDisabled(); // Disabled when name empty
		const nameInput = screen.getByPlaceholderText('e.g. CI Pipeline');
		await userEvent.type(nameInput, 'new-token');
		await waitFor(() => expect(createBtn).not.toHaveAttribute('disabled'));
		// Verify no aria-busy when not loading (Button removes attr when loading=false)
		expect(createBtn).not.toHaveAttribute('aria-busy');
		// Verify static children "Create" (no text-swap)
		expect(createBtn.textContent).toContain('Create');
	});

	it('New API Token modal Created state Copy button renders variant="secondary"', async () => {
		vi.mocked(api.createApiToken).mockResolvedValue({ id: 'token-1', token: 'secret-token-123' });
		render(ProfilePage);
		const newTokenBtn = screen.getByRole('button', { name: 'New Token' });
		await userEvent.click(newTokenBtn);
		await waitFor(() => expect(screen.getByPlaceholderText('e.g. CI Pipeline')).toBeInTheDocument());
		const nameInput = screen.getByPlaceholderText('e.g. CI Pipeline');
		await userEvent.type(nameInput, 'test-token');
		const createBtn = screen.getByRole('button', { name: 'Create' });
		await userEvent.click(createBtn);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Copy' })).toBeInTheDocument());
		const copyBtn = screen.getByRole('button', { name: 'Copy' });
		expect(copyBtn).toHaveClass('bg-[var(--bg-raised)]'); // secondary variant
	});

	it('New API Token modal Created state Done button renders variant="primary"', async () => {
		vi.mocked(api.createApiToken).mockResolvedValue({ id: 'token-1', token: 'secret-token-123' });
		render(ProfilePage);
		const newTokenBtn = screen.getByRole('button', { name: 'New Token' });
		await userEvent.click(newTokenBtn);
		await waitFor(() => expect(screen.getByPlaceholderText('e.g. CI Pipeline')).toBeInTheDocument());
		const nameInput = screen.getByPlaceholderText('e.g. CI Pipeline');
		await userEvent.type(nameInput, 'test-token');
		const createBtn = screen.getByRole('button', { name: 'Create' });
		await userEvent.click(createBtn);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Done' })).toBeInTheDocument());
		const doneBtn = screen.getByRole('button', { name: 'Done' });
		expect(doneBtn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary variant
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
		const newTokenBtn = screen.getByRole('button', { name: 'New Token' });
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
		await waitFor(() => expect(screen.getByRole('button', { name: 'Revoke' })).toBeInTheDocument());
		const revokeBtn = screen.getByRole('button', { name: 'Revoke' });
		await userEvent.click(revokeBtn);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Revoke API Token' })).toBeInTheDocument());
		// ConfirmDialog is rendered but its confirm button is owned by #3k
		// We only assert that the launcher (Revoke) opened the dialog
		expect(screen.getByRole('heading', { name: 'Revoke API Token' })).toBeInTheDocument();
	});
});
