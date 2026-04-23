import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
	listSystemEnrollmentTokens: vi.fn().mockResolvedValue({ items: [], total: 0, page: 1, per_page: 20, total_pages: 0 }),
	createSystemEnrollmentToken: vi.fn(),
	revokeSystemEnrollmentToken: vi.fn()
}));

import * as api from '$lib/api';
import SystemServicesSettings from './SystemServicesSettings.svelte';

const props = { onSuccess: vi.fn(), onError: vi.fn() };

afterEach(() => vi.clearAllMocks());

describe('SystemServicesSettings button variants', () => {
	it('has no raw preset-filled-primary-500 buttons', () => {
		const { container } = render(SystemServicesSettings, props);
		expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
	});

	it('has no raw preset-filled-error-500 buttons', () => {
		const { container } = render(SystemServicesSettings, props);
		expect(container.querySelector('button.preset-filled-error-500')).toBeNull();
	});

	it('has no raw preset-tonal buttons', () => {
		const { container } = render(SystemServicesSettings, props);
		expect(container.querySelector('button.preset-tonal')).toBeNull();
	});

	it('Revoke button has danger class', async () => {
		vi.mocked(api.listSystemEnrollmentTokens).mockResolvedValue({
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
		render(SystemServicesSettings, props);
		await fireEvent.click(screen.getByRole('button', { name: 'Load Tokens' }));
		const revokeBtn = await screen.findByRole('button', { name: 'Revoke' });
		expect(revokeBtn.className).toContain('bg-[var(--color-danger-bg)]');
	});

	it('Copy button has ghost class after token creation', async () => {
		vi.mocked(api.createSystemEnrollmentToken).mockResolvedValue({
			id: 't1',
			name: 'My Token',
			token: 'tok_abc',
			current_uses: 0,
			max_uses: null,
			expires_at: null,
			revoked_at: null,
			created_at: new Date().toISOString()
		} as never);
		render(SystemServicesSettings, props);
		await fireEvent.click(screen.getByRole('button', { name: 'Create Token' }));
		const nameInput = await screen.findByLabelText(/Name/i);
		await fireEvent.input(nameInput, { target: { value: 'My Token' } });
		await fireEvent.click(screen.getByRole('button', { name: 'Create' }));
		const copyBtn = await screen.findByRole('button', { name: 'Copy' });
		expect(copyBtn.className).toContain('bg-transparent');
		expect(copyBtn).not.toHaveAttribute('aria-busy');
	});

	it('modal Create button carries aria-busy=true while creating', async () => {
		let resolve!: (v: unknown) => void;
		vi.mocked(api.createSystemEnrollmentToken).mockReturnValue(
			new Promise((r) => {
				resolve = r as unknown as (v: unknown) => void;
			})
		);
		render(SystemServicesSettings, props);
		// Open modal
		const createLauncher = screen.getByRole('button', { name: 'Create Token' });
		await fireEvent.click(createLauncher);
		// Fill name
		const nameInput = await screen.findByLabelText(/Name/i);
		await fireEvent.input(nameInput, { target: { value: 'My Token' } });
		const createBtn = screen.getByRole('button', { name: 'Create' });
		await fireEvent.click(createBtn);
		await waitFor(() => expect(createBtn).toHaveAttribute('aria-busy', 'true'));
		resolve({
			token: 'tok_abc',
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
