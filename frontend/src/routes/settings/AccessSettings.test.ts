import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
	getAccessSettings: vi.fn(async () => ({
		data: {
			mode: 'open',
			require_token_for_oidc: false,
			password_auth_enabled: true,
			two_factor_required: false
		}
	})),
	updateAccessSettings: vi.fn(async () => ({
		data: {
			mode: 'open',
			require_token_for_oidc: false,
			password_auth_enabled: true,
			two_factor_required: false
		}
	}))
}));
vi.mock('$lib/stores/network.svelte', () => ({ getIsOnline: vi.fn(() => true) }));

import * as api from '$lib/api';
import AccessSettings from './AccessSettings.svelte';

const defaultProps = { onSuccess: vi.fn(), onError: vi.fn() };

afterEach(() => vi.clearAllMocks());

describe('AccessSettings', () => {
	it('Save button is disabled when form is not dirty', async () => {
		render(AccessSettings, defaultProps);
		await waitFor(() => expect(screen.getByRole('button', { name: 'Save' })).toBeTruthy());
		const btn = screen.getByRole('button', { name: 'Save' });
		expect(btn).toBeDisabled();
	});

	it('Discard button hidden when not dirty', async () => {
		render(AccessSettings, defaultProps);
		await waitFor(() => screen.getByRole('button', { name: 'Save' }));
		expect(screen.queryByRole('button', { name: 'Discard' })).toBeNull();
	});

	it('calls updateAccessSettings on save', async () => {
		render(AccessSettings, defaultProps);
		await waitFor(() => screen.getByRole('button', { name: 'Save' }));
		// RadioCardGroup: click "Closed" card to make form dirty
		const closedCard = screen.getByRole('radio', { name: /closed/i });
		await fireEvent.click(closedCard);
		const btn = screen.getByRole('button', { name: 'Save' });
		expect(btn).not.toBeDisabled();
		await fireEvent.click(btn);
		await waitFor(() => expect(api.updateAccessSettings).toHaveBeenCalled());
	});

	it('shows registration token field only in invite mode', async () => {
		render(AccessSettings, defaultProps);
		await waitFor(() => screen.getByRole('button', { name: 'Save' }));
		expect(screen.queryByLabelText(/registration token/i)).toBeNull();
		const inviteCard = screen.getByRole('radio', { name: /invite only/i });
		await fireEvent.click(inviteCard);
		await waitFor(() => expect(screen.getByLabelText(/registration token/i)).toBeTruthy());
	});
});
