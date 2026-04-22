import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({ updateRegistrationSettings: vi.fn() }));
vi.mock('$lib/stores/network.svelte', () => ({ getIsOnline: vi.fn(() => true) }));

import * as api from '$lib/api';
import RegistrationSettings from './RegistrationSettings.svelte';

const settingsProps = {
	settings: { mode: 'open' as const, require_token_for_oidc: false },
	onSuccess: vi.fn(),
	onError: vi.fn()
};

afterEach(() => vi.clearAllMocks());

describe('RegistrationSettings button', () => {
	it('Save button has no raw preset-filled-primary-500 class', () => {
		const { container } = render(RegistrationSettings, settingsProps);
		expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
	});

	it('Save button carries aria-busy=true while save is in flight', async () => {
		let resolve!: () => void;
		vi.mocked(api.updateRegistrationSettings).mockReturnValue(
			new Promise<{ mode: 'open'; require_token_for_oidc: boolean }>((r) => {
				resolve = () => r({ mode: 'open', require_token_for_oidc: false });
			})
		);

		render(RegistrationSettings, settingsProps);
		const btn = screen.getByRole('button', { name: 'Save' });
		await fireEvent.click(btn);

		await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));

		resolve();
		await waitFor(() => expect(btn).not.toHaveAttribute('aria-busy'));
	});

	it('Save button text is static "Save" during loading — no text swap', async () => {
		let resolve!: () => void;
		vi.mocked(api.updateRegistrationSettings).mockReturnValue(
			new Promise<{ mode: 'open'; require_token_for_oidc: boolean }>((r) => {
				resolve = () => r({ mode: 'open', require_token_for_oidc: false });
			})
		);

		render(RegistrationSettings, settingsProps);
		const btn = screen.getByRole('button', { name: 'Save' });
		await fireEvent.click(btn);

		await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));
		expect(btn).toHaveTextContent('Save');

		resolve();
	});
});
