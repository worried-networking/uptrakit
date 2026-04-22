import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({ resetData: vi.fn() }));
vi.mock('$lib/stores/network.svelte', () => ({ getIsOnline: vi.fn(() => true) }));

import * as api from '$lib/api';
import DangerZone from './DangerZone.svelte';

const props = { onSuccess: vi.fn(), onError: vi.fn() };

afterEach(() => vi.clearAllMocks());

describe('DangerZone button variants', () => {
	it('launcher Reset Data button has no raw preset-filled-error-500 class', () => {
		const { container } = render(DangerZone, props);
		expect(container.querySelector('button.preset-filled-error-500')).toBeNull();
	});

	it('inline Cancel button inside modal has no raw preset-tonal-surface class', async () => {
		const { container } = render(DangerZone, props);
		await fireEvent.click(screen.getByRole('button', { name: 'Reset Data' }));
		await screen.findByRole('button', { name: 'Cancel' });
		expect(container.querySelector('button.preset-tonal-surface')).toBeNull();
	});

	it('inline Reset All Data button inside modal has no raw preset-filled-error-500 class', async () => {
		const { container } = render(DangerZone, props);
		await fireEvent.click(screen.getByRole('button', { name: 'Reset Data' }));
		await screen.findByRole('button', { name: 'Reset All Data' });
		expect(container.querySelector('button.preset-filled-error-500')).toBeNull();
	});

	it('inline Reset All Data button carries aria-busy=true while submitting', async () => {
		let resolve!: () => void;
		vi.mocked(api.resetData).mockReturnValue(
			new Promise<{
				deleted: {
					hosts: number;
					software_items: number;
					plugin_configs: number;
					host_tags: number;
					update_history: number;
					update_batches: number;
				};
			}>((r) => {
				resolve = () =>
					r({
						deleted: {
							hosts: 1,
							software_items: 0,
							plugin_configs: 0,
							host_tags: 0,
							update_history: 0,
							update_batches: 0
						}
					});
			})
		);

		render(DangerZone, props);
		await fireEvent.click(screen.getByRole('button', { name: 'Reset Data' }));
		await screen.findByRole('button', { name: 'Reset All Data' });

		const confirmInput = screen.getByPlaceholderText('Type RESET to confirm');
		await fireEvent.input(confirmInput, { target: { value: 'RESET' } });

		const confirmBtn = screen.getByRole('button', { name: 'Reset All Data' });
		await fireEvent.click(confirmBtn);

		await waitFor(() => expect(confirmBtn).toHaveAttribute('aria-busy', 'true'));

		resolve();
		// After resolve, the modal switches to result view — the confirm button is removed.
		// Verify the result view is shown (Close button appears), meaning submission completed.
		await screen.findByRole('button', { name: 'Close' });
	});

	it('inline Reset All Data button text is static "Reset All Data" during loading', async () => {
		let resolve!: () => void;
		vi.mocked(api.resetData).mockReturnValue(
			new Promise<{
				deleted: {
					hosts: number;
					software_items: number;
					plugin_configs: number;
					host_tags: number;
					update_history: number;
					update_batches: number;
				};
			}>((r) => {
				resolve = () =>
					r({
						deleted: {
							hosts: 0,
							software_items: 0,
							plugin_configs: 0,
							host_tags: 0,
							update_history: 0,
							update_batches: 0
						}
					});
			})
		);

		render(DangerZone, props);
		await fireEvent.click(screen.getByRole('button', { name: 'Reset Data' }));
		await screen.findByRole('button', { name: 'Reset All Data' });

		const confirmInput = screen.getByPlaceholderText('Type RESET to confirm');
		await fireEvent.input(confirmInput, { target: { value: 'RESET' } });

		const confirmBtn = screen.getByRole('button', { name: 'Reset All Data' });
		await fireEvent.click(confirmBtn);

		await waitFor(() => expect(confirmBtn).toHaveAttribute('aria-busy', 'true'));
		expect(confirmBtn).toHaveTextContent('Reset All Data');

		resolve();
	});
});
