import { describe, expect, it, vi } from 'vitest';
import { render, waitFor } from '@testing-library/svelte';

// Heavy mocks — +page.svelte pulls in many modules
vi.mock('$app/state', () => ({ page: { url: { searchParams: { get: () => null } } } }));
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$lib/auth.svelte', () => ({ getUser: vi.fn(() => null) }));
vi.mock('$lib/api', async (importOriginal) => ({
	...(await importOriginal<typeof import('$lib/api')>()),
	getCombinedSettings: vi.fn().mockRejectedValue(new Error('network error')),
	listProviders: vi.fn().mockRejectedValue(new Error('network error'))
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
	filterSurfacesByAction: vi.fn(() => []),
	isSurfaceTabPending: vi.fn(() => false)
}));

import * as auth from '$lib/auth.svelte';
import { goto } from '$app/navigation';
import { Actions } from '$lib/api';
import SettingsPage from './+page.svelte';

function makeUser() {
	return {
		id: 'u1',
		email: 'a@b.com',
		first_name: 'A',
		last_name: 'B',
		has_pending_email_change: false,
		actions: [Actions.SETTINGS_AUTH_MANAGE],
		authority: 'ok' as const
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

describe('+page.svelte authority-unavailable redirect guard', () => {
	it('does not bounce the user to "/" when authority is unavailable, even though actions is empty', async () => {
		// Degraded-authority `me` response: empty `actions` + `authority: 'unavailable'` is a
		// deliberate fail-open placeholder, not a genuine denial (M1.7). Without the guard in
		// the redirect `$effect`, `hasAnyTabAction` is false here and the page would call the
		// bare `goto('/')` redirect — distinct from the always-running URL-sync effect, which
		// calls `goto(pathname, { replaceState: true, ... })` with a second argument.
		vi.mocked(auth.getUser).mockReturnValue({
			id: 'u-degraded',
			email: 'degraded@b.com',
			first_name: 'D',
			last_name: 'E',
			has_pending_email_change: false,
			actions: [],
			authority: 'unavailable' as const
		});

		render(SettingsPage);

		await waitFor(() => {
			expect(vi.mocked(goto)).toHaveBeenCalled();
		});

		expect(vi.mocked(goto)).not.toHaveBeenCalledWith('/');
	});
});
