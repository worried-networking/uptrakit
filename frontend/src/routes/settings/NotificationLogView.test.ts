import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', async (importOriginal) => ({
	...(await importOriginal<typeof import('$lib/api')>()),
	listLog: vi.fn(),
	listChannels: vi.fn()
}));
vi.mock('$lib/auth.svelte', () => ({ getUser: vi.fn(() => null) }));

import * as api from '$lib/api';
import NotificationLogView from './NotificationLogView.svelte';

// The generated SDK fns resolve to `{ data }`. Wrap + type via the generated return shape.
function logPage(items: unknown[]): Awaited<ReturnType<typeof api.listLog>> {
	return {
		data: { items, total: items.length, page: 1, per_page: 20, total_pages: 1 }
	} as unknown as Awaited<ReturnType<typeof api.listLog>>;
}

function channelsPage(items: unknown[]): Awaited<ReturnType<typeof api.listChannels>> {
	return {
		data: { items, total: items.length, page: 1, per_page: 20, total_pages: 1 }
	} as unknown as Awaited<ReturnType<typeof api.listChannels>>;
}

afterEach(() => vi.clearAllMocks());

describe('NotificationLogView Retry button', () => {
	it('Retry button has no raw preset-filled-primary-500 class', async () => {
		vi.mocked(api.listLog).mockRejectedValue(new Error('network error'));
		vi.mocked(api.listChannels).mockResolvedValue(channelsPage([]));
		const { container } = render(NotificationLogView);
		await screen.findByRole('button', { name: 'Retry' });
		expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
	});

	it('Retry button has primary variant class (accent gradient)', async () => {
		vi.mocked(api.listLog).mockRejectedValue(new Error('network error'));
		vi.mocked(api.listChannels).mockResolvedValue(channelsPage([]));
		render(NotificationLogView);
		const btn = await screen.findByRole('button', { name: 'Retry' });
		expect(btn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
	});

	it('clicking Retry triggers a new loadData call (isRetrying state wires through)', async () => {
		// loadData sets loading=true synchronously, which hides the DataTable (and Retry button).
		// So aria-busy on the Retry button itself is not observable in the DOM after click.
		// We verify instead that clicking Retry triggers a second listLog call.
		vi.mocked(api.listLog).mockRejectedValueOnce(new Error('network error')).mockResolvedValueOnce(logPage([]));
		vi.mocked(api.listChannels).mockResolvedValue(channelsPage([]));
		render(NotificationLogView);
		const btn = await screen.findByRole('button', { name: 'Retry' });
		await fireEvent.click(btn);
		await waitFor(() => expect(api.listLog).toHaveBeenCalledTimes(2));
	});

	it('Retry button is not present while loading (skeleton shown instead)', async () => {
		vi.mocked(api.listLog)
			.mockRejectedValueOnce(new Error('network error'))
			.mockReturnValue(new Promise(() => {}) as unknown as ReturnType<typeof api.listLog>); // never resolves — keeps loading=true
		vi.mocked(api.listChannels).mockResolvedValue(channelsPage([]));
		render(NotificationLogView);
		// After error the Retry button appears
		const btn = await screen.findByRole('button', { name: 'Retry' });
		expect(btn).toBeDefined();
		// After clicking, loading kicks in and hides the DataTable (and Retry button)
		await fireEvent.click(btn);
		await waitFor(() => expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull());
	});
});
