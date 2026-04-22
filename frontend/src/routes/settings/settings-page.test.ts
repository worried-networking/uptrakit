import { describe, expect, it, vi } from 'vitest';
import { render, waitFor } from '@testing-library/svelte';

// Heavy mocks — +page.svelte pulls in many modules
vi.mock('$app/state', () => ({ page: { url: { searchParams: { get: () => null } } } }));
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$lib/auth.svelte', () => ({ getUser: vi.fn(() => null) }));
vi.mock('$lib/api', () => ({
	getCombinedSettings: vi.fn().mockRejectedValue(new Error('network error')),
	getOidcProviders: vi.fn().mockRejectedValue(new Error('network error'))
}));
vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn()
}));
vi.mock('$lib/surfaces/registry.svelte', () => ({
	getSurfaceReadLoading: vi.fn(() => false),
	getSurfaceReadModel: vi.fn(() => undefined),
	getSurfaceReadRequested: vi.fn(() => false),
	getSurfaceRegistryLoaded: vi.fn(() => true),
	getSurfacesBySlot: vi.fn(() => []),
	loadSurfaceReadModels: vi.fn()
}));
vi.mock('$lib/surfaces/read-model', () => ({
	filterSurfacesByPermission: vi.fn(() => []),
	isSurfaceTabPending: vi.fn(() => false)
}));

import * as auth from '$lib/auth.svelte';
import { Permission } from '$lib/types';
import SettingsPage from './+page.svelte';

function makeUser() {
	return {
		id: 'u1',
		email: 'a@b.com',
		first_name: 'A',
		last_name: 'B',
		permissions: [Permission.ManageAuthSettings]
	};
}

describe('+page.svelte Retry All buttons', () => {
	it('Retry All buttons render as Button variant="primary" size="sm" (h-[19px] class)', async () => {
		vi.mocked(auth.getUser).mockReturnValue(makeUser());

		// getCombinedSettings is mocked to reject — this triggers the error states
		// that make the Retry All buttons visible inside each <Callout> block.
		const { container } = render(SettingsPage);

		// Wait for Retry All buttons to appear (error state settles on next tick)
		await waitFor(() => {
			const retryBtns = container.querySelectorAll('button');
			const retryAll = [...retryBtns].filter((b) => b.textContent?.trim() === 'Retry All');
			expect(retryAll.length).toBeGreaterThan(0);
		});

		// Positive: each Retry All button uses Button primitive size=sm → h-[19px]
		const allBtns = container.querySelectorAll('button');
		const retryAllBtns = [...allBtns].filter((b) => b.textContent?.trim() === 'Retry All');
		retryAllBtns.forEach((btn) => {
			expect(btn.className).toContain('h-[19px]');
		});

		// Negative: no raw Skeleton preset classes on any button
		const rawBtns = container.querySelectorAll('button.btn.btn-sm.preset-filled-primary-500');
		expect(rawBtns.length).toBe(0);
	});
});
