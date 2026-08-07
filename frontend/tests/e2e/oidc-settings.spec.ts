import { expect, test } from '@playwright/test';

const mockUser = {
	id: '00000000-0000-0000-0000-000000000001',
	email: 'admin@example.com',
	first_name: 'Admin',
	last_name: 'User',
	actions: ['settings.auth:manage'],
	authority: 'ok'
};

const mockSettings = {
	agent_certificates: {
		lifetime_days: 365,
		renewal_window_hours_override: null,
		effective_renewal_window_hours: 336
	},
	enrollment_tokens: { active_count: 0 },
	multi_tenancy_enabled: false
};

async function mockApi(page: import('@playwright/test').Page) {
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
		if (method === 'GET' && path === '/api/v1/settings') return json(mockSettings);
		if (method === 'GET' && path === '/api/v1/settings/oidc-providers') return json([]);
		if (method === 'GET' && path === '/api/v1/admin/events') return route.abort();
		return route.abort();
	});
}

test.describe('OIDC settings — Add Provider modal layout', () => {
	test.beforeEach(async ({ page }) => {
		await mockApi(page);
		await page.goto('/settings');
		await page.waitForSelector('[data-ui="modal-shell"]', { state: 'hidden', timeout: 2000 }).catch(() => {});
		await page.getByRole('button', { name: 'Add Provider' }).click();
		await page.waitForSelector('[data-ui="modal-shell"]');
	});

	test('Name and Slug fields appear on the same row', async ({ page }) => {
		const nameLabel = page.locator('label[for="oidc-name"]');
		const slugLabel = page.locator('label[for="oidc-slug"]');

		const nameTop = await nameLabel.evaluate((el) => el.getBoundingClientRect().top);
		const slugTop = await slugLabel.evaluate((el) => el.getBoundingClientRect().top);

		expect(Math.abs(nameTop - slugTop)).toBeLessThan(4);
	});

	test('Client ID and Client Secret fields appear on the same row', async ({ page }) => {
		const clientIdLabel = page.locator('label[for="oidc-client-id"]');
		const clientSecretLabel = page.locator('label[for="oidc-client-secret"]');

		const clientIdTop = await clientIdLabel.evaluate((el) => el.getBoundingClientRect().top);
		const clientSecretTop = await clientSecretLabel.evaluate((el) => el.getBoundingClientRect().top);

		expect(Math.abs(clientIdTop - clientSecretTop)).toBeLessThan(4);
	});

	test('Name and Slug fields are not stacked (different left positions)', async ({ page }) => {
		const nameInput = page.locator('#oidc-name');
		const slugInput = page.locator('#oidc-slug');

		const nameLeft = await nameInput.evaluate((el) => el.getBoundingClientRect().left);
		const slugLeft = await slugInput.evaluate((el) => el.getBoundingClientRect().left);

		expect(slugLeft).toBeGreaterThan(nameLeft + 50);
	});
});
