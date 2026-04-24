import { expect, test } from '@playwright/test';

const mockUser = {
	id: '00000000-0000-0000-0000-000000000001',
	email: 'dev@example.com',
	first_name: 'Dev',
	last_name: 'User',
	permissions: [
		'view_software',
		'create_software',
		'update_software',
		'delete_software',
		'trigger_checks',
		'trigger_updates',
		'manage_ignores'
	]
};

const softwareItems = {
	items: [
		{
			id: 'test-item-id',
			name: 'Firefox',
			plugins: ['apt'],
			featured: true,
			last_checked_at: '2026-04-21T10:00:00Z',
			host_count: 2,
			installed_version: null,
			installed_display_version: null,
			latest_version: '125.0',
			latest_release_metadata: null,
			update_available: true,
			created_at: '2026-01-01T00:00:00Z',
			updated_at: '2026-04-21T10:00:00Z',
			icon_url: null
		}
	],
	total: 1,
	page: 1,
	per_page: 50,
	total_pages: 1
};

const softwareDetail = {
	id: 'test-item-id',
	name: 'Firefox',
	plugins: ['apt'],
	featured: true,
	last_checked_at: '2026-04-21T10:00:00Z',
	host_count: 2,
	installed_version: null,
	installed_display_version: null,
	latest_version: '125.0',
	latest_release_metadata: null,
	update_available: true,
	created_at: '2026-01-01T00:00:00Z',
	updated_at: '2026-04-21T10:00:00Z',
	icon_url: null,
	hosts: [
		{
			id: 'row-1',
			host_id: 'host-1',
			hostname: 'server-01',
			friendly_name: 'Server 01',
			qualifier: null,
			installed_version: '124.0',
			installed_version_detected_at: '2026-04-20T00:00:00Z',
			installed_display_version: null,
			latest_version: '125.0',
			latest_release_metadata: null,
			update_available: true,
			active_update_history_id: null,
			last_updated_at: null,
			linked_at: '2026-01-01T00:00:00Z',
			plugins: [{ plugin_type: 'apt', plugin_config_name: 'apt', role: 'detect_version' }]
		},
		{
			id: 'row-2',
			host_id: 'host-2',
			hostname: 'server-02',
			friendly_name: 'Server 02',
			qualifier: null,
			installed_version: '125.0',
			installed_version_detected_at: '2026-04-21T00:00:00Z',
			installed_display_version: null,
			latest_version: '125.0',
			latest_release_metadata: null,
			update_available: false,
			active_update_history_id: null,
			last_updated_at: null,
			linked_at: '2026-01-01T00:00:00Z',
			plugins: [{ plugin_type: 'apt', plugin_config_name: 'apt', role: 'detect_version' }]
		}
	]
};

const ignoresPage = {
	items: [],
	total: 0,
	page: 1,
	per_page: 25,
	total_pages: 1
};

async function mockAuthApi(page: import('@playwright/test').Page) {
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
		if (method === 'GET' && path === '/api/v1/auth/me') {
			return json(mockUser);
		}
		if (method === 'GET' && path === '/api/v1/system/alerts') {
			return json({ alerts: [] });
		}
		if (method === 'GET' && path === '/api/v1/surfaces') {
			return json([]);
		}
		if (method === 'GET' && path === '/api/v1/plugin-types') {
			return json([]);
		}
		if (method === 'GET' && path === '/api/v1/software-items') {
			return json(softwareItems);
		}
		if (method === 'GET' && path === '/api/v1/software-items/test-item-id') {
			return json(softwareDetail);
		}
		if (method === 'GET' && path === '/api/v1/autodiscovery/ignores') {
			return json(ignoresPage);
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

const SNAPSHOTS = [
	{ name: 'software-list-dark', route: '/software?tab=all', theme: 'dark' as const },
	{ name: 'software-list-light', route: '/software?tab=all', theme: 'light' as const },
	{ name: 'software-ignores-dark', route: '/software?tab=ignores', theme: 'dark' as const },
	{ name: 'software-ignores-light', route: '/software?tab=ignores', theme: 'light' as const },
	{ name: 'software-detail-dark', route: '/software/test-item-id', theme: 'dark' as const },
	{ name: 'software-detail-light', route: '/software/test-item-id', theme: 'light' as const }
];

const MOBILE_SNAPSHOTS = [
	{ name: 'software-list-mobile-dark', route: '/software?tab=all', theme: 'dark' as const },
	{ name: 'software-list-mobile-light', route: '/software?tab=all', theme: 'light' as const }
];

test.describe('software area snapshots', () => {
	for (const snap of SNAPSHOTS) {
		test(snap.name, async ({ page }) => {
			await mockAuthApi(page);
			await setTheme(page, snap.theme);
			await page.goto(snap.route);

			// Wait for the main content to load
			await page.waitForSelector('[data-ui="page-shell"]', { timeout: 10000 });

			// Mask dynamic content
			await expect(page).toHaveScreenshot(`${snap.name}.png`, {
				threshold: 0.02,
				mask: [
					page.locator('[aria-busy="true"]'),
					page.locator('td.font-mono'),
					page.locator('[data-ui="toast"]'),
					page.locator('time')
				]
			});
		});
	}
});

test.describe('software area mobile snapshots', () => {
	test.beforeEach((_fixtures, testInfo) => {
		if (!testInfo.project.name.includes('mobile')) test.skip();
	});

	for (const snap of MOBILE_SNAPSHOTS) {
		test(snap.name, async ({ page }) => {
			await mockAuthApi(page);
			await setTheme(page, snap.theme);
			await page.goto(snap.route);
			await page.waitForSelector('[data-ui="page-shell"]', { timeout: 10000 });
			await expect(page).toHaveScreenshot(`${snap.name}.png`, {
				threshold: 0.02,
				mask: [
					page.locator('[aria-busy="true"]'),
					page.locator('td.font-mono'),
					page.locator('[data-ui="toast"]'),
					page.locator('time')
				]
			});
		});
	}
});

test.describe('software area mobile layout', () => {
	test('mobile: software group list renders card layout at 393px', async ({ page }) => {
		await mockAuthApi(page);
		await setTheme(page, 'light');
		await page.setViewportSize({ width: 393, height: 852 });
		await page.goto('/software?tab=all');
		await page.waitForSelector('[data-ui="software-group-list-mobile"]', { timeout: 10000 });

		const mobileList = page.locator('[data-ui="software-group-list-mobile"]');
		await expect(mobileList).toBeVisible();

		// Desktop list should be hidden on mobile
		const desktopList = page.locator('[data-ui="software-group-list"]');
		await expect(desktopList).toBeHidden();

		// Each item renders as a mobile card
		const firstCard = mobileList.locator('[role="listitem"]').first();
		await expect(firstCard).toBeVisible();
		// Software name link is in the card
		await expect(firstCard.getByRole('link', { name: 'Firefox' })).toBeVisible();
	});

	test('mobile: software group list desktop layout is hidden at 393px', async ({ page }) => {
		await mockAuthApi(page);
		await setTheme(page, 'light');
		await page.setViewportSize({ width: 393, height: 852 });
		await page.goto('/software?tab=all');
		await page.waitForSelector('[data-ui="page-shell"]', { timeout: 10000 });

		// Desktop list uses max-sm:hidden — hidden at 393px
		const desktopList = page.locator('[data-ui="software-group-list"]');
		await expect(desktopList).toBeHidden();
	});

	test('desktop: software group list desktop layout is visible at 1280px', async ({ page }) => {
		await mockAuthApi(page);
		await setTheme(page, 'light');
		// Default viewport is desktop width; ensure mobile list is hidden
		await page.goto('/software?tab=all');
		await page.waitForSelector('[data-ui="software-group-list"]', { timeout: 10000 });

		const desktopList = page.locator('[data-ui="software-group-list"]');
		await expect(desktopList).toBeVisible();

		// Mobile list uses sm:hidden — hidden at 1280px
		const mobileList = page.locator('[data-ui="software-group-list-mobile"]');
		await expect(mobileList).toBeHidden();
	});
});
