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
		await page.goto('/device?code=AB12-1BAD');

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
});
