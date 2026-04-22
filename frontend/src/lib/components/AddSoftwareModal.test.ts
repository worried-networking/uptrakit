import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import AddSoftwareModal from './AddSoftwareModal.svelte';
import type { SoftwareItemResponse } from '$lib/types';

vi.mock('$lib/api', () => ({
	createSoftwareItem: vi.fn()
}));

vi.mock('$lib/notifications.svelte', () => ({
	showError: vi.fn(),
	showSuccess: vi.fn()
}));

import * as api from '$lib/api';

function makeSoftwareItem(): SoftwareItemResponse {
	return {
		id: 'software-1',
		name: 'Firefox',
		plugins: ['generic_shell'],
		featured: true,
		last_checked_at: null,
		host_count: 0,
		installed_version: null,
		installed_display_version: null,
		latest_version: null,
		latest_release_metadata: null,
		update_available: false,
		created_at: '2024-01-01T00:00:00Z',
		updated_at: '2024-01-01T00:00:00Z',
		icon_url: null
	};
}

describe('AddSoftwareModal', () => {
	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
	});

	it('shows inline required validation for name instead of relying on toasts only', async () => {
		const user = userEvent.setup();
		render(AddSoftwareModal, {
			onclose: vi.fn(),
			onsuccess: vi.fn()
		});

		await user.click(screen.getByRole('button', { name: 'Register Software' }));

		expect(screen.getByText('Name is required.')).toBeInTheDocument();
		expect(screen.getByLabelText('Name')).toHaveAttribute('aria-invalid', 'true');
		expect(api.createSoftwareItem).not.toHaveBeenCalled();
	});

	it('blocks submission with inline icon URL validation when URL is not HTTPS', async () => {
		const user = userEvent.setup();
		render(AddSoftwareModal, {
			onclose: vi.fn(),
			onsuccess: vi.fn()
		});

		await user.type(screen.getByLabelText('Name'), 'Firefox');
		await user.type(screen.getByLabelText('Icon URL'), 'http://example.com/icon.png');
		await user.click(screen.getByRole('button', { name: 'Register Software' }));

		expect(screen.getByText('Icon URL must be a valid HTTPS URL.')).toBeInTheDocument();
		expect(screen.getByLabelText('Icon URL')).toHaveAttribute('aria-invalid', 'true');
		expect(api.createSoftwareItem).not.toHaveBeenCalled();
	});

	it('shows loading state while submit is in flight', async () => {
		const user = userEvent.setup();
		const onsuccess = vi.fn();
		let resolveCreate: ((value: SoftwareItemResponse) => void) | null = null;
		vi.mocked(api.createSoftwareItem).mockImplementation(
			() =>
				new Promise<SoftwareItemResponse>((resolve) => {
					resolveCreate = resolve;
				})
		);

		render(AddSoftwareModal, {
			onclose: vi.fn(),
			onsuccess
		});

		await user.type(screen.getByLabelText('Name'), 'Firefox');
		await user.click(screen.getByRole('button', { name: 'Register Software' }));

		// Button now uses loading={submitting} spinner — no "Registering..." text swap
		const submitBtn = screen.getByRole('button', { name: 'Register Software' });
		expect(submitBtn).toHaveAttribute('aria-busy', 'true');
		expect(submitBtn).toBeDisabled();
		expect(resolveCreate).not.toBeNull();
		resolveCreate!(makeSoftwareItem());

		await waitFor(() => expect(onsuccess).toHaveBeenCalledWith(makeSoftwareItem()));
	});
});
