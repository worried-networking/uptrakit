import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({ updateAuthenticationSettings: vi.fn() }));
vi.mock('$lib/stores/network.svelte', () => ({ getIsOnline: vi.fn(() => true) }));

import * as api from '$lib/api';
import AuthenticationSettings from './AuthenticationSettings.svelte';

const settingsProps = {
	settings: { password_auth_enabled: true },
	onSuccess: vi.fn(),
	onError: vi.fn()
};

afterEach(() => vi.clearAllMocks());

describe('AuthenticationSettings button', () => {
	it('Save button has no raw preset-filled-primary-500 class', () => {
		const { container } = render(AuthenticationSettings, settingsProps);
		expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
	});

	it('Save button carries aria-busy=true while save is in flight', async () => {
		let resolve!: () => void;
		vi.mocked(api.updateAuthenticationSettings).mockReturnValue(
			new Promise<{ password_auth_enabled: boolean }>((r) => {
				resolve = () => r({ password_auth_enabled: true });
			})
		);

		render(AuthenticationSettings, settingsProps);
		const btn = screen.getByRole('button', { name: 'Save' });
		await fireEvent.click(btn);

		await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));

		resolve();
		await waitFor(() => expect(btn).not.toHaveAttribute('aria-busy'));
	});

	it('Save button text is static "Save" during loading — no text swap', async () => {
		let resolve!: () => void;
		vi.mocked(api.updateAuthenticationSettings).mockReturnValue(
			new Promise<{ password_auth_enabled: boolean }>((r) => {
				resolve = () => r({ password_auth_enabled: true });
			})
		);

		render(AuthenticationSettings, settingsProps);
		const btn = screen.getByRole('button', { name: 'Save' });
		await fireEvent.click(btn);

		await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));
		expect(btn).toHaveTextContent('Save');

		resolve();
	});
});
