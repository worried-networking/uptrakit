import { expect, test } from '@playwright/test';
import { PARITY_DYNAMIC_MASK_SELECTOR, PARITY_VIEWPORT_PRESETS, expectParityScreenshot } from './parity-config';

const isCanonicalUiParityHost = process.platform === 'darwin';
const canonicalUiParityReason =
	'ui parity screenshot baselines are canonicalized on macOS Chromium to avoid cross-OS rasterization drift';

const mockUser = {
	id: '00000000-0000-0000-0000-000000000201',
	email: 'admin@example.com',
	first_name: 'Admin',
	last_name: 'User',
	permissions: [
		'view_software',
		'trigger_updates',
		'view_services',
		'approve_services',
		'reject_services',
		'remove_services',
		'update_services',
		'view_hosts',
		'manage_hosts',
		'update_hosts',
		'deactivate_hosts',
		'create_software',
		'update_software',
		'delete_software',
		'trigger_checks',
		'manage_scheduler',
		'view_settings',
		'manage_auth_settings',
		'manage_enrollment_tokens',
		'manage_agent_certs',
		'manage_global_settings',
		'view_notifications',
		'update_system_services',
		'view_system_services',
		'view_audit_logs'
	]
};

const baseHistoryItem = {
	host_id: 'host-001',
	software_item_id: 'sw-001',
	actor_type: 'user',
	actor_id: 'actor-1',
	output: '',
	output_truncated: false,
	interactive: false,
	pre_update_protection_status: null,
	pre_update_protection_summary: null,
	recovery_hint: null,
	created_at: '2026-01-15T08:00:00Z'
};

const historyItems = [
	{
		...baseHistoryItem,
		id: 'hist-001',
		host_name: 'prod-01',
		software_item_name: 'nginx',
		from_version: '1.24.0',
		to_version: '1.25.0',
		status: 'completed',
		started_at: '2026-01-15T08:00:00Z',
		completed_at: '2026-01-15T08:05:00Z',
		output: 'Update completed successfully.'
	},
	{
		...baseHistoryItem,
		id: 'hist-002',
		host_name: 'prod-02',
		software_item_name: 'redis',
		from_version: '7.0.0',
		to_version: '7.2.0',
		status: 'failed',
		started_at: '2026-01-15T07:00:00Z',
		completed_at: '2026-01-15T07:10:00Z'
	},
	{
		...baseHistoryItem,
		id: 'hist-003',
		host_name: 'prod-03',
		software_item_name: 'postgresql',
		from_version: '16.1',
		to_version: '16.2',
		status: 'in_progress',
		interactive: true,
		started_at: '2026-01-15T09:00:00Z',
		completed_at: null
	}
];

async function mockHistoryApi(
	page: import('@playwright/test').Page,
	scenario: 'default' | 'filter-completed' | 'in-progress-interactive' = 'default'
) {
	await page.route('**/api/v1/**', async (route) => {
		const url = new URL(route.request().url());
		const path = url.pathname;
		const method = route.request().method();
		const json = (body: unknown, status = 200) =>
			route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

		if (method === 'POST' && path === '/api/v1/auth/refresh') {
			return json({
				access_token: 'test-token',
				refresh_token: 'test-refresh',
				expires_in: 3600,
				token_type: 'Bearer',
				user: mockUser
			});
		}
		if (method === 'GET' && path === '/api/v1/auth/me') return json(mockUser);
		if (method === 'GET' && path === '/api/v1/system/alerts') return json({ alerts: [] });
		if (method === 'GET' && path === '/api/v1/surfaces') return json([]);

		if (method === 'GET' && path === '/api/v1/update-history') {
			const items =
				scenario === 'filter-completed'
					? historyItems.filter((i) => i.status === 'completed')
					: scenario === 'in-progress-interactive'
						? historyItems.filter((i) => i.status === 'in_progress')
						: historyItems;
			return json({ items, total: items.length, page: 1, per_page: 25, total_pages: 1 });
		}

		// Block SSE connection
		if (method === 'GET' && path === '/api/v1/admin/events') {
			return route.abort();
		}

		return route.abort();
	});
}

async function setTheme(page: import('@playwright/test').Page, theme: 'dark' | 'light') {
	await page.addInitScript((t) => {
		if (t === 'dark') document.documentElement.classList.add('dark');
		else document.documentElement.classList.remove('dark');
		try {
			localStorage.setItem('theme', t);
		} catch {
			/* ignore */
		}
	}, theme);
}

test.use({
	viewport: PARITY_VIEWPORT_PRESETS.desktop,
	locale: 'en-US',
	timezoneId: 'UTC'
});

test.describe('history route visual parity', () => {
	test.skip(!isCanonicalUiParityHost, canonicalUiParityReason);

	test('default feed — light', async ({ page }) => {
		await setTheme(page, 'light');
		await mockHistoryApi(page, 'default');
		await page.goto('/history');
		await page.waitForSelector('[data-ui="history-feed-list"]');
		await page.waitForLoadState('networkidle');

		await expectParityScreenshot({
			page,
			target: page,
			name: 'history-default-light.png',
			viewport: 'desktop',
			maskSelectors: [PARITY_DYNAMIC_MASK_SELECTOR]
		});
	});

	test('default feed — dark', async ({ page }) => {
		await setTheme(page, 'dark');
		await mockHistoryApi(page, 'default');
		await page.goto('/history');
		await page.waitForSelector('[data-ui="history-feed-list"]');
		await page.waitForLoadState('networkidle');

		await expectParityScreenshot({
			page,
			target: page,
			name: 'history-default-dark.png',
			viewport: 'desktop',
			maskSelectors: [PARITY_DYNAMIC_MASK_SELECTOR]
		});
	});

	test('filter active — completed chip selected, light', async ({ page }) => {
		await setTheme(page, 'light');
		await mockHistoryApi(page, 'filter-completed');
		await page.goto('/history?status=completed');
		await page.waitForSelector('[data-ui="history-feed-list"]');
		await page.waitForLoadState('networkidle');

		await expectParityScreenshot({
			page,
			target: page,
			name: 'history-filter-completed-light.png',
			viewport: 'desktop',
			maskSelectors: [PARITY_DYNAMIC_MASK_SELECTOR]
		});
	});

	test('in-progress interactive row — light', async ({ page }) => {
		await setTheme(page, 'light');
		await mockHistoryApi(page, 'in-progress-interactive');
		await page.goto('/history?status=in_progress');
		await page.waitForSelector('[data-ui="history-feed-list"]');
		await page.waitForLoadState('networkidle');

		await expectParityScreenshot({
			page,
			target: page,
			name: 'history-in-progress-interactive-light.png',
			viewport: 'desktop',
			maskSelectors: [PARITY_DYNAMIC_MASK_SELECTOR]
		});
	});
});

test.describe('history route button contract smoke', () => {
	test('filter chips have correct active/inactive classes', async ({ page }) => {
		await mockHistoryApi(page, 'filter-completed');
		await page.goto('/history?status=completed');
		await page.waitForSelector('[data-ui="history-feed-list"]');

		// Active chip: Completed
		const completedChip = page.getByRole('button', { name: 'Completed' });
		const completedClass = await completedChip.getAttribute('class');
		expect(completedClass).toContain('text-[var(--accent)]');
		expect(completedClass).toContain('bg-[var(--bg-hover)]');

		// Inactive chip: Failed
		const failedChip = page.getByRole('button', { name: 'Failed' });
		const failedClass = await failedChip.getAttribute('class');
		expect(failedClass).not.toContain('text-[var(--accent)]');
	});

	test('expand toggle shows Attach terminal for interactive in-progress row', async ({ page }) => {
		await mockHistoryApi(page, 'in-progress-interactive');
		await page.goto('/history?status=in_progress');
		await page.waitForSelector('[data-ui="history-feed-list"]');

		const attachBtn = page.getByRole('button', { name: /attach terminal/i });
		await expect(attachBtn).toBeVisible();
		await expect(attachBtn).toHaveAttribute('aria-expanded', 'false');
	});

	test('no preset-filled-* or preset-tonal-* classes in history DOM', async ({ page }) => {
		await mockHistoryApi(page, 'default');
		await page.goto('/history');
		await page.waitForSelector('[data-ui="history-feed-list"]');

		const presetElements = page.locator('[class*="preset-filled-"],[class*="preset-tonal-"]');
		await expect(presetElements).toHaveCount(0);
	});
});
