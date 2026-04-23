import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
	listSystemEnrollmentTokens: vi.fn().mockResolvedValue({ items: [], total: 0, page: 1, per_page: 20, pages: 0 }),
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

	it('modal Create button carries aria-busy=true while creating', async () => {
		let resolve!: (v: unknown) => void;
		vi.mocked(api.createSystemEnrollmentToken).mockReturnValue(
			new Promise((r) => {
				resolve = r;
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
