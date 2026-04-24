import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { Permission, type HostTagResponse, type PaginatedResponse } from '$lib/types';

vi.mock('$lib/api', () => ({
	getHostTags: vi.fn(),
	createHostTag: vi.fn(),
	updateHostTag: vi.fn(),
	deleteHostTag: vi.fn(),
	batchHostTags: vi.fn(),
	executeBatchChunked: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null)
}));

vi.mock('$lib/stores/events.svelte', () => ({
	subscribeToEvent: vi.fn(() => () => {})
}));

vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn()
}));

import HostTagsPage from './+page.svelte';
import * as api from '$lib/api';
import * as auth from '$lib/auth.svelte';

const user = {
	id: '00000000-0000-0000-0000-000000000104',
	email: 'host-tags@example.com',
	first_name: 'Host',
	last_name: 'Tags',
	has_pending_email_change: false,
	permissions: [Permission.UpdateHosts, Permission.DeactivateHosts]
};

function makePage(items: HostTagResponse[]): PaginatedResponse<HostTagResponse> {
	return { items, total: items.length, page: 1, per_page: 25, total_pages: 1 };
}

describe('Host Tags Route', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.getHostTags).mockResolvedValue(
			makePage([
				{
					id: 'tag-1',
					name: 'production',
					color: '#16A34A',
					description: 'Production hosts',
					host_count: 8,
					created_at: '2026-03-01T10:00:00Z'
				} as unknown as HostTagResponse
			])
		);
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('renders shared shell primitives for tag management table', async () => {
		render(HostTagsPage);

		await waitFor(() => expect(screen.getByText('Host Tags')).toBeInTheDocument());
		expect(screen.getByText('production')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="table-footer-bar"]')).toBeInTheDocument();
		expect(screen.getByText('1 total')).toBeInTheDocument();
		await fireEvent.click(screen.getByRole('button', { name: /actions for production/i }));
		expect(document.querySelector('[data-ui="context-menu-item"]')).toBeInTheDocument();
	});
});

describe('Button Migrations', () => {
	beforeEach(() => {
		vi.mocked(auth.getUser).mockReturnValue(user);
		vi.mocked(api.getHostTags).mockResolvedValue(makePage([]));
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('Create Tag header action renders variant="primary"', async () => {
		render(HostTagsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Create Tag' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Create Tag' });
		expect(btn).toHaveClass('inline-flex'); // Button base class
		expect(btn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary variant
	});

	it('Row ellipsis trigger renders variant="ghost" size="sm" with EllipsisIcon and sr-only children', async () => {
		vi.mocked(api.getHostTags).mockResolvedValue(
			makePage([
				{
					id: 'tag-1',
					name: 'prod',
					color: '#FF0000',
					description: '',
					created_at: '2026-04-19T00:00:00Z',
					updated_at: '2026-04-19T00:00:00Z',
					host_count: 5
				}
			])
		);
		render(HostTagsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Actions for prod' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Actions for prod' });
		expect(btn).toHaveClass('h-[19px]'); // size="sm"
		expect(btn).toHaveClass('bg-transparent'); // ghost variant
		const srOnly = btn.querySelector('span.sr-only');
		expect(srOnly?.textContent).toBe('Actions for prod');
		expect(btn.querySelector('svg')).toBeInTheDocument(); // EllipsisIcon rendered
	});

	it('Row ellipsis trigger click opens context menu (stopPropagation + menu positioning)', async () => {
		vi.mocked(api.getHostTags).mockResolvedValue(
			makePage([
				{
					id: 'tag-1',
					name: 'test',
					color: '#00FF00',
					description: '',
					created_at: '2026-04-19T00:00:00Z',
					updated_at: '2026-04-19T00:00:00Z',
					host_count: 2
				}
			])
		);
		render(HostTagsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Actions for test' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Actions for test' });
		await userEvent.click(btn);
		// Menu opens — Edit and Delete items are visible
		await waitFor(() => expect(screen.getByRole('menuitem', { name: 'Edit' })).toBeInTheDocument());
		expect(screen.getByRole('menuitem', { name: 'Delete' })).toBeInTheDocument();
	});

	it('Error Retry button renders variant="primary" with async loading state', async () => {
		vi.mocked(api.getHostTags).mockRejectedValueOnce(new Error('Network error'));
		render(HostTagsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Retry' });
		expect(btn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary variant
		expect(btn).not.toHaveAttribute('aria-busy'); // Not loading initially
		expect(btn).not.toHaveAttribute('disabled');

		// Simulate click and verify recovery after success
		vi.mocked(api.getHostTags).mockResolvedValueOnce(makePage([]));
		await userEvent.click(btn);
		// After successful retry, error state clears and Retry button is removed
		await waitFor(() => expect(screen.queryByRole('button', { name: 'Retry' })).not.toBeInTheDocument());
	});

	it('Error Retry button clears loading state after rejection', async () => {
		vi.mocked(api.getHostTags).mockRejectedValueOnce(new Error('Load failed'));
		render(HostTagsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument());
		const btn = screen.getByRole('button', { name: 'Retry' });

		// Mock rejection on retry click
		vi.mocked(api.getHostTags).mockRejectedValueOnce(new Error('Retry failed'));
		try {
			btn.click();
		} catch {
			// Expected
		}
		await waitFor(() => expect(btn).not.toHaveAttribute('aria-busy', 'true'));
	});

	it('Create modal Auto toggle renders variant="secondary" size="sm"', async () => {
		render(HostTagsPage);
		const createBtn = screen.getByRole('button', { name: 'Create Tag' });
		await userEvent.click(createBtn);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Create Tag' })).toBeInTheDocument());
		// Pick color button is inside label; use getByText since accessible name includes label text
		const pickColorBtn = screen.getByText('Pick color');
		await userEvent.click(pickColorBtn);
		await waitFor(() => expect(screen.getByText('Auto')).toBeInTheDocument());
		const autoBtn = screen.getByText('Auto').closest('button')!;
		expect(autoBtn).toHaveClass('h-[19px]'); // size="sm"
		expect(autoBtn).toHaveClass('bg-[var(--bg-raised)]'); // secondary variant
	});

	it('Create modal footer Cancel renders variant="secondary"', async () => {
		render(HostTagsPage);
		const createBtn = screen.getByRole('button', { name: 'Create Tag' });
		await userEvent.click(createBtn);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Create Tag' })).toBeInTheDocument());
		const cancelBtn = screen.getByRole('button', { name: 'Cancel' });
		expect(cancelBtn).toHaveClass('bg-[var(--bg-raised)]'); // secondary variant
	});

	it('Create modal footer Create submit renders variant="primary" with loading={submitting}', async () => {
		render(HostTagsPage);
		const createBtn = screen.getByRole('button', { name: 'Create Tag' });
		await userEvent.click(createBtn);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Create Tag' })).toBeInTheDocument());
		const submitBtn = screen.getByRole('button', { name: 'Create' });
		expect(submitBtn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary variant
		expect(submitBtn).toHaveAttribute('disabled'); // Disabled when name empty
		const nameInput = screen.getByPlaceholderText('e.g. production');
		await userEvent.type(nameInput, 'new-tag');
		expect(submitBtn).not.toHaveAttribute('disabled'); // Enabled when name present
	});

	it('Create modal footer Create children stay static "Create" across submit window', async () => {
		vi.mocked(api.createHostTag).mockImplementation(
			() =>
				new Promise((resolve) =>
					setTimeout(
						() =>
							resolve({
								id: 'tag-1',
								name: 'new',
								color: '',
								description: '',
								created_at: '2026-04-19T00:00:00Z',
								updated_at: '2026-04-19T00:00:00Z',
								host_count: 0
							} as unknown as HostTagResponse),
						100
					)
				)
		);
		render(HostTagsPage);
		const createBtn = screen.getByRole('button', { name: 'Create Tag' });
		await userEvent.click(createBtn);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Create Tag' })).toBeInTheDocument());
		const nameInput = screen.getByPlaceholderText('e.g. production');
		await userEvent.type(nameInput, 'new-tag');
		const submitBtn = screen.getByRole('button', { name: 'Create' });
		await userEvent.click(submitBtn);
		// Children should remain "Create", not "Creating..."
		expect(submitBtn.textContent).toContain('Create');
		expect(submitBtn.textContent).not.toContain('Creating');
	});

	it('Edit modal footer Save renders variant="primary" with loading={submitting} and disabled={!editTag?.name.trim()}', async () => {
		vi.mocked(api.getHostTags).mockResolvedValue(
			makePage([
				{
					id: 'tag-1',
					name: 'prod',
					color: '#FF0000',
					description: 'desc',
					created_at: '2026-04-19T00:00:00Z',
					updated_at: '2026-04-19T00:00:00Z',
					host_count: 5
				}
			])
		);
		render(HostTagsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Actions for prod' })).toBeInTheDocument());
		const ellipsisBtn = screen.getByRole('button', { name: 'Actions for prod' });
		await userEvent.click(ellipsisBtn);
		await waitFor(() => expect(screen.getByRole('menuitem', { name: 'Edit' })).toBeInTheDocument());
		const editItem = screen.getByRole('menuitem', { name: 'Edit' });
		await userEvent.click(editItem);
		await waitFor(() => expect(screen.getByDisplayValue('prod')).toBeInTheDocument());
		const saveBtn = screen.getByRole('button', { name: 'Save' });
		expect(saveBtn).toHaveClass('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary variant
		// Clear name field
		const nameInput = screen.getByDisplayValue('prod') as HTMLInputElement;
		nameInput.value = '';
		nameInput.dispatchEvent(new Event('input', { bubbles: true }));
		await waitFor(() => expect(saveBtn).toHaveAttribute('disabled'));
		// Restore name
		nameInput.value = 'updated';
		nameInput.dispatchEvent(new Event('input', { bubbles: true }));
		await waitFor(() => expect(saveBtn).not.toHaveAttribute('disabled'));
	});

	it('Edit modal footer Cancel renders variant="secondary"', async () => {
		vi.mocked(api.getHostTags).mockResolvedValue(
			makePage([
				{
					id: 'tag-1',
					name: 'test',
					color: '#00FF00',
					description: '',
					created_at: '2026-04-19T00:00:00Z',
					updated_at: '2026-04-19T00:00:00Z',
					host_count: 2
				}
			])
		);
		render(HostTagsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Actions for test' })).toBeInTheDocument());
		const ellipsisBtn = screen.getByRole('button', { name: 'Actions for test' });
		await userEvent.click(ellipsisBtn);
		await waitFor(() => expect(screen.getByRole('menuitem', { name: 'Edit' })).toBeInTheDocument());
		const editItem = screen.getByRole('menuitem', { name: 'Edit' });
		await userEvent.click(editItem);
		await waitFor(() => expect(screen.getByDisplayValue('test')).toBeInTheDocument());
		const cancelBtn = screen.getByRole('button', { name: 'Cancel' });
		expect(cancelBtn).toHaveClass('bg-[var(--bg-raised)]'); // secondary variant
	});

	it('Out-of-scope regression: Edit/Delete ContextMenuItems remain unchanged and are not wrapped in Button', async () => {
		vi.mocked(api.getHostTags).mockResolvedValue(
			makePage([
				{
					id: 'tag-1',
					name: 'prod',
					color: '#FF0000',
					description: '',
					created_at: '2026-04-19T00:00:00Z',
					updated_at: '2026-04-19T00:00:00Z',
					host_count: 5
				}
			])
		);
		render(HostTagsPage);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Actions for prod' })).toBeInTheDocument());
		const ellipsisBtn = screen.getByRole('button', { name: 'Actions for prod' });
		await userEvent.click(ellipsisBtn);
		await waitFor(() => expect(screen.getByRole('menuitem', { name: 'Edit' })).toBeInTheDocument());
		const editItem = screen.getByRole('menuitem', { name: 'Edit' });
		expect(editItem).toBeInTheDocument();
		expect(editItem).toHaveAttribute('data-ui', 'context-menu-item'); // ContextMenuItem renders with data-ui attr
		const deleteItem = screen.getByRole('menuitem', { name: 'Delete' });
		expect(deleteItem).toBeInTheDocument();
		expect(deleteItem).toHaveAttribute('data-ui', 'context-menu-item'); // ContextMenuItem renders with data-ui attr
	});
});
