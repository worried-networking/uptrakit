import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
	listNotificationLog: vi.fn(),
	listNotificationChannels: vi.fn()
}));
vi.mock('$lib/auth.svelte', () => ({ getUser: vi.fn(() => null) }));

import * as api from '$lib/api';
import NotificationLogView from './NotificationLogView.svelte';

afterEach(() => vi.clearAllMocks());

describe('NotificationLogView Retry button', () => {
	it('Retry button has no raw preset-filled-primary-500 class', async () => {
		vi.mocked(api.listNotificationLog).mockRejectedValue(new Error('network error'));
		vi.mocked(api.listNotificationChannels).mockResolvedValue({
			items: [],
			total: 0,
			page: 1,
			total_pages: 1
		});
		const { container } = render(NotificationLogView);
		await screen.findByRole('button', { name: 'Retry' });
		expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
	});

	it('Retry button has primary variant class (accent gradient)', async () => {
		vi.mocked(api.listNotificationLog).mockRejectedValue(new Error('network error'));
		vi.mocked(api.listNotificationChannels).mockResolvedValue({
			items: [],
			total: 0,
			page: 1,
			total_pages: 1
		});
		render(NotificationLogView);
		const btn = await screen.findByRole('button', { name: 'Retry' });
		expect(btn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
	});

	it('clicking Retry triggers a new loadData call (isRetrying state wires through)', async () => {
		// loadData sets loading=true synchronously, which hides the DataTable (and Retry button).
		// So aria-busy on the Retry button itself is not observable in the DOM after click.
		// We verify instead that clicking Retry triggers a second listNotificationLog call.
		vi.mocked(api.listNotificationLog)
			.mockRejectedValueOnce(new Error('network error'))
			.mockResolvedValueOnce({ items: [], total: 0, page: 1, total_pages: 1 });
		vi.mocked(api.listNotificationChannels).mockResolvedValue({
			items: [],
			total: 0,
			page: 1,
			total_pages: 1
		});
		render(NotificationLogView);
		const btn = await screen.findByRole('button', { name: 'Retry' });
		await fireEvent.click(btn);
		await waitFor(() => expect(api.listNotificationLog).toHaveBeenCalledTimes(2));
	});

	it('Retry button is not present while loading (skeleton shown instead)', async () => {
		vi.mocked(api.listNotificationLog)
			.mockRejectedValueOnce(new Error('network error'))
			.mockReturnValue(new Promise(() => {})); // never resolves — keeps loading=true
		vi.mocked(api.listNotificationChannels).mockResolvedValue({
			items: [],
			total: 0,
			page: 1,
			total_pages: 1
		});
		render(NotificationLogView);
		// After error the Retry button appears
		const btn = await screen.findByRole('button', { name: 'Retry' });
		expect(btn).toBeDefined();
		// After clicking, loading kicks in and hides the DataTable (and Retry button)
		await fireEvent.click(btn);
		await waitFor(() => expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull());
	});
});
