import { afterEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({ createSoftwareItem: vi.fn() }));
vi.mock('$lib/notifications.svelte', () => ({ showSuccess: vi.fn(), showError: vi.fn() }));
vi.mock('$lib/utils', () => ({
	isValidLogoUrl: vi.fn((url: string) => url.startsWith('https://'))
}));

import AddSoftwareModal from './AddSoftwareModal.svelte';
import * as api from '$lib/api';

describe('AddSoftwareModal Button primitive contracts', () => {
	afterEach(() => {
		vi.clearAllMocks();
	});

	it('"Register Software" button renders primary variant', () => {
		render(AddSoftwareModal, { onclose: vi.fn(), onsuccess: vi.fn() });
		const submitBtn = screen.getByRole('button', { name: 'Register Software' });
		expect(submitBtn.className).toContain('bg-[linear-gradient');
		expect(submitBtn).not.toHaveAttribute('aria-busy');
	});

	it('shows aria-busy and no "Registering..." text during submit', async () => {
		vi.mocked(api.createSoftwareItem).mockReturnValue(new Promise(() => {}));
		render(AddSoftwareModal, { onclose: vi.fn(), onsuccess: vi.fn() });

		await fireEvent.input(screen.getByRole('textbox', { name: /name/i }), {
			target: { value: 'Firefox' }
		});
		const submitBtn = screen.getByRole('button', { name: 'Register Software' });
		await fireEvent.click(submitBtn);

		await waitFor(() => expect(submitBtn).toHaveAttribute('aria-busy', 'true'));
		expect(document.body.textContent).not.toContain('Registering...');
		expect(submitBtn.textContent?.trim()).toContain('Register Software');
	});

	it('Cancel renders secondary variant', () => {
		render(AddSoftwareModal, { onclose: vi.fn(), onsuccess: vi.fn() });
		const cancelBtn = screen.getByRole('button', { name: 'Cancel' });
		expect(cancelBtn.className).toContain('var(--bg-raised)'); // secondary
	});
});
