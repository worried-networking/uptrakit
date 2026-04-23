import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
	listEnrollmentTokens: vi.fn().mockResolvedValue({ items: [], total: 0, page: 1, per_page: 20, pages: 0 }),
	createEnrollmentToken: vi.fn(),
	revokeEnrollmentToken: vi.fn()
}));

import * as api from '$lib/api';
import EnrollmentTokenSettings from './EnrollmentTokenSettings.svelte';

const props = {
	summary: { total: 0, active: 0, revoked: 0, expired: 0, active_count: 0 },
	onSuccess: vi.fn(),
	onError: vi.fn()
};

afterEach(() => vi.clearAllMocks());

describe('EnrollmentTokenSettings button variants', () => {
	it('has no raw preset-filled-primary-500 buttons', () => {
		const { container } = render(EnrollmentTokenSettings, props);
		expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
	});

	it('has no raw preset-tonal buttons (unqualified)', () => {
		const { container } = render(EnrollmentTokenSettings, props);
		expect(container.querySelector('button.preset-tonal')).toBeNull();
	});

	it('Revoke button has danger class', async () => {
		vi.mocked(api.listEnrollmentTokens).mockResolvedValue({
			items: [
				{
					id: 't1',
					name: 'Active Token',
					token: '',
					current_uses: 0,
					max_uses: null,
					expires_at: null,
					revoked_at: null,
					created_at: new Date().toISOString()
				} as never
			],
			total: 1,
			page: 1,
			per_page: 20,
			total_pages: 1
		});
		render(EnrollmentTokenSettings, props);
		// Load tokens to show the table
		await fireEvent.click(screen.getByRole('button', { name: 'Load Tokens' }));
		const revokeBtn = await screen.findByRole('button', { name: 'Revoke' });
		expect(revokeBtn.className).toContain('bg-[var(--color-danger-bg)]');
	});

	it('Copy button has ghost class and sm size after token creation', async () => {
		vi.mocked(api.createEnrollmentToken).mockResolvedValue({
			id: 't1',
			name: 'My Token',
			token: 'tok_xyz',
			current_uses: 0,
			max_uses: null,
			expires_at: null,
			revoked_at: null,
			created_at: new Date().toISOString()
		} as never);
		render(EnrollmentTokenSettings, props);
		await fireEvent.click(screen.getByRole('button', { name: 'Create Token' }));
		const nameInput = await screen.findByLabelText(/Name/i);
		await fireEvent.input(nameInput, { target: { value: 'My Token' } });
		await fireEvent.click(screen.getByRole('button', { name: 'Create' }));
		const copyBtn = await screen.findByRole('button', { name: 'Copy' });
		expect(copyBtn.className).toContain('bg-transparent');
		// size="sm" — no loading prop
		expect(copyBtn).not.toHaveAttribute('aria-busy');
	});

	it('modal Create button carries aria-busy=true while creating', async () => {
		let resolve!: (v: unknown) => void;
		vi.mocked(api.createEnrollmentToken).mockReturnValue(
			new Promise((r) => {
				resolve = r as unknown as (v: unknown) => void;
			})
		);
		render(EnrollmentTokenSettings, props);
		const createLauncher = screen.getByRole('button', { name: 'Create Token' });
		await fireEvent.click(createLauncher);
		const nameInput = await screen.findByLabelText(/Name/i);
		await fireEvent.input(nameInput, { target: { value: 'My Token' } });
		const createBtn = screen.getByRole('button', { name: 'Create' });
		await fireEvent.click(createBtn);
		await waitFor(() => expect(createBtn).toHaveAttribute('aria-busy', 'true'));
		resolve({
			token: 'tok_xyz',
			id: 't1',
			name: 'My Token',
			created_at: new Date().toISOString(),
			revoked_at: null,
			expires_at: null,
			max_uses: null,
			use_count: 0
		});
		// Modal closes after success — button removed from DOM; verify not still busy
		await waitFor(() => {
			const stillBusy = createBtn.getAttribute('aria-busy') === 'true';
			const stillInDom = document.body.contains(createBtn);
			expect(stillBusy && stillInDom).toBe(false);
		});
	});
});
