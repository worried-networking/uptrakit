import { expect, test } from '@playwright/test';

const ROUTE = '/dev/surface-preview';

const mockUser = {
	id: '00000000-0000-0000-0000-000000000001',
	email: 'dev@example.com',
	first_name: 'Dev',
	last_name: 'User',
	permissions: []
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

test.describe('surface-preview visual baseline', () => {
	for (const theme of ['dark', 'light'] as const) {
		test.describe(theme, () => {
			test.beforeEach(async ({ page }) => {
				await mockAuthApi(page);
				await setTheme(page, theme);
				await page.goto(ROUTE);
				await page.waitForSelector('[data-testid="surface-preview-root"]');
			});

			test('full page snapshot', async ({ page }) => {
				await expect(page.locator('[data-testid="surface-preview-root"]')).toHaveScreenshot(`${theme}-full.png`, {
					threshold: 0.005
				});
			});
		});
	}
});
