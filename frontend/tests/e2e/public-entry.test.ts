import { expect, test } from '@playwright/test';

async function mockAnonymousSession(page: import('@playwright/test').Page) {
	await page.route('**/api/v1/auth/refresh', (route) =>
		route.fulfill({ status: 401, json: { error: 'Unauthorized' } })
	);
	await page.route('**/api/v1/system/alerts', (route) => route.fulfill({ json: { alerts: [] } }));
}

async function mockAuthMethods(page: import('@playwright/test').Page) {
	await page.route('**/api/v1/auth/methods', (route) =>
		route.fulfill({
			json: {
				password: true,
				oidc_providers: [],
				setup_required: false,
				registration_token_required: false
			}
		})
	);
}

test.describe('Public entry shell', () => {
	test.beforeEach(async ({ page }) => {
		await mockAnonymousSession(page);
		await mockAuthMethods(page);
	});

	test('login uses the shared shell and inline validation', async ({ page }) => {
		await page.goto('/login');

		await expect(page.locator('[data-ui="public-entry-shell"]')).toBeVisible();
		await expect(page.getByRole('heading', { name: 'Login' })).toBeVisible();
		await expect(page.getByText('Use your account credentials or an identity provider.')).toBeVisible();
		await expect(page.locator('[data-ui="form-field-row"]')).toHaveCount(2);
		await expect(page.locator('#main-content').getByRole('link', { name: 'Register' })).toHaveAttribute(
			'href',
			'/register'
		);
		await expect(page.getByRole('textbox', { name: 'Email' })).toHaveClass(/focus-visible:/);
		await expect(page.getByRole('button', { name: 'Login' })).toHaveClass(/focus-visible:/);

		await page.getByRole('button', { name: 'Login' }).click();

		await expect(page.getByText('Email is required.')).toBeVisible();
		await expect(page.getByText('Password is required.')).toBeVisible();
	});

	test('register uses the shared shell and inline validation', async ({ page }) => {
		await page.goto('/register');

		await expect(page.locator('[data-ui="public-entry-shell"]')).toBeVisible();
		await expect(page.getByRole('heading', { name: 'Register' })).toBeVisible();
		await expect(page.getByText('Create your local account to sign in later.')).toBeVisible();
		await expect(page.locator('[data-ui="form-field-row"]')).toHaveCount(4);
		await expect(page.locator('#main-content a[href="/login"]')).toBeVisible();
		await expect(page.getByRole('textbox', { name: 'Email' })).toHaveClass(/focus-visible:/);
		await expect(page.getByRole('button', { name: 'Register' })).toHaveClass(/focus-visible:/);

		await page.getByRole('button', { name: 'Register' }).click();

		await expect(page.getByText('Email is required.')).toBeVisible();
		await expect(page.getByText('First name is required.')).toBeVisible();
		await expect(page.getByText('Last name is required.')).toBeVisible();
		await expect(page.getByText('Password is required.')).toBeVisible();
	});

	test('device uses semantic callouts in the shared shell', async ({ page }) => {
		await page.goto('/device?user_code=AB12-1BAD');

		await expect(page.locator('[data-ui="public-entry-shell"]')).toBeVisible();
		await expect(page.getByRole('heading', { name: 'Authorize Device' })).toBeVisible();
		await expect(page.getByText('Confirm the code shown in your CLI to finish signing in.')).toBeVisible();
		await expect(page.locator('[data-ui="callout"][data-tone="danger"]')).toContainText('Invalid device code format');
	});

	test('public errors use the shared shell framing', async ({ page }) => {
		await page.goto('/definitely-missing');

		await expect(page.locator('[data-ui="public-entry-shell"]')).toBeVisible();
		await expect(page.getByRole('heading', { name: 'Something went wrong' })).toBeVisible();
		await expect(page.getByText('The requested page could not be loaded.')).toBeVisible();
		await expect(page.locator('[data-ui="callout"][data-tone="danger"]')).toContainText('Error 404');
		await expect(page.getByRole('button', { name: 'Go to Home' })).toBeVisible();
	});

	test('device shows client name when lookup succeeds', async ({ page }) => {
		await page.route('**/api/v1/auth/refresh', (route) =>
			route.fulfill({
				status: 200,
				json: { access_token: 'test-access-token', refresh_token: 'test-refresh-token' }
			})
		);
		await page.route('**/api/v1/auth/me', (route) =>
			route.fulfill({
				status: 200,
				json: {
					id: '00000000-0000-0000-0000-000000000001',
					email: 'user@example.com',
					first_name: 'Test',
					last_name: 'User',
					permissions: []
				}
			})
		);
		await page.route('**/api/v1/system/alerts', (route) => route.fulfill({ json: { alerts: [] } }));
		await page.route('**/api/v1/auth/device/lookup*', (route) =>
			route.fulfill({
				json: { client_name: 'cli-laptop-2026-05-12', expires_at: '2026-05-12T12:00:00Z' }
			})
		);

		await page.goto('/device?user_code=BCDF-GHJK');
		await expect(page.locator('[data-ui="callout"]')).toContainText('cli-laptop-2026-05-12');
	});

	test('device approve succeeds', async ({ page }) => {
		await page.route('**/api/v1/auth/refresh', (route) =>
			route.fulfill({
				status: 200,
				json: { access_token: 'test-access-token', refresh_token: 'test-refresh-token' }
			})
		);
		await page.route('**/api/v1/auth/me', (route) =>
			route.fulfill({
				status: 200,
				json: {
					id: '00000000-0000-0000-0000-000000000001',
					email: 'user@example.com',
					first_name: 'Test',
					last_name: 'User',
					permissions: []
				}
			})
		);
		await page.route('**/api/v1/system/alerts', (route) => route.fulfill({ json: { alerts: [] } }));
		await page.route('**/api/v1/auth/device/lookup*', (route) =>
			route.fulfill({
				json: { client_name: null, expires_at: '2026-05-12T12:00:00Z' }
			})
		);
		await page.route('**/api/v1/auth/device/approve', (route) => route.fulfill({ json: { message: 'approved' } }));

		await page.goto('/device?user_code=BCDF-GHJK');
		await page.getByRole('button', { name: 'Approve' }).click();
		await expect(page.locator('[data-ui="callout"][data-tone="success"]')).toContainText('CLI session approved');
	});

	test('device deny succeeds', async ({ page }) => {
		await page.route('**/api/v1/auth/refresh', (route) =>
			route.fulfill({
				status: 200,
				json: { access_token: 'test-access-token', refresh_token: 'test-refresh-token' }
			})
		);
		await page.route('**/api/v1/auth/me', (route) =>
			route.fulfill({
				status: 200,
				json: {
					id: '00000000-0000-0000-0000-000000000001',
					email: 'user@example.com',
					first_name: 'Test',
					last_name: 'User',
					permissions: []
				}
			})
		);
		await page.route('**/api/v1/system/alerts', (route) => route.fulfill({ json: { alerts: [] } }));
		await page.route('**/api/v1/auth/device/lookup*', (route) =>
			route.fulfill({
				json: { client_name: null, expires_at: '2026-05-12T12:00:00Z' }
			})
		);
		await page.route('**/api/v1/auth/device/deny', (route) => route.fulfill({ json: { message: 'denied' } }));

		await page.goto('/device?user_code=BCDF-GHJK');
		await page.getByRole('button', { name: 'Deny' }).click();
		await expect(page.locator('[data-ui="callout"][data-tone="warning"]')).toContainText('CLI authorization denied');
	});
});
