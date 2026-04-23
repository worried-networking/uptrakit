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

	it('modal Create button carries aria-busy=true while creating', async () => {
		let resolve!: (v: unknown) => void;
		vi.mocked(api.createEnrollmentToken).mockReturnValue(
			new Promise((r) => {
				resolve = r;
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
