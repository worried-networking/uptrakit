import { expect, test } from '@playwright/test';

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

const mockUser = {
	id: '00000000-0000-0000-0000-000000000001',
	email: 'admin@example.com',
	first_name: 'Admin',
	last_name: 'User',
	permissions: [
		'view_agents',
		'manage_agents',
		'view_hosts',
		'manage_hosts',
		'view_settings',
		'manage_settings',
		'view_software',
		'manage_software'
	]
};

const tokenResponse = {
	access_token: 'test-access-token',
	refresh_token: 'test-refresh-token',
	expires_in: 3600,
	token_type: 'Bearer',
	user: mockUser
};

/** Mock the minimal set of endpoints needed to show the authenticated home page. */
async function mockAuthenticatedSession(page: import('@playwright/test').Page) {
	await page.route('**/api/v1/auth/refresh', (route) => route.fulfill({ json: tokenResponse }));
	await page.route('**/api/v1/auth/me', (route) => route.fulfill({ json: mockUser }));
	await page.route('**/api/v1/system/alerts', (route) => route.fulfill({ json: { alerts: [] } }));
}

/** Return 401 for the refresh endpoint so the app stays anonymous. */
async function mockUnauthenticated(page: import('@playwright/test').Page) {
	await page.route('**/api/v1/auth/refresh', (route) =>
		route.fulfill({ status: 401, json: { error: 'Unauthorized' } })
	);
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test.describe('Authentication', () => {
	test('unauthenticated visit to / redirects to /login', async ({ page }) => {
		await mockUnauthenticated(page);
		await page.goto('/');
		await expect(page).toHaveURL(/\/login/);
	});

	test('login page renders with a password form', async ({ page }) => {
		await mockUnauthenticated(page);
		await page.goto('/login');
		await expect(page.getByRole('textbox', { name: /email/i })).toBeVisible();
		await expect(page.locator('input[type="password"]')).toBeVisible();
		await expect(page.getByRole('button', { name: /sign in/i })).toBeVisible();
	});

	test('successful login navigates away from /login', async ({ page }) => {
		await mockUnauthenticated(page);
		await page.route('**/api/v1/auth/login', (route) => route.fulfill({ json: tokenResponse }));
		// After login, /auth/me is called to load user and /system/alerts for layout
		await page.route('**/api/v1/auth/me', (route) => route.fulfill({ json: mockUser }));
		await page.route('**/api/v1/system/alerts', (route) => route.fulfill({ json: { alerts: [] } }));

		await page.goto('/login');
		await page.getByRole('textbox', { name: /email/i }).fill('admin@example.com');
		await page.locator('input[type="password"]').fill('correct-password');
		await page.getByRole('button', { name: /sign in/i }).click();

		await expect(page).not.toHaveURL(/\/login/);
	});

	test('wrong credentials show an error message', async ({ page }) => {
		await mockUnauthenticated(page);
		await page.route('**/api/v1/auth/login', (route) =>
			route.fulfill({ status: 401, json: { error: 'Invalid email or password' } })
		);

		await page.goto('/login');
		await page.getByRole('textbox', { name: /email/i }).fill('bad@example.com');
		await page.locator('input[type="password"]').fill('wrong');
		await page.getByRole('button', { name: /sign in/i }).click();

		await expect(page.getByText('Invalid email or password')).toBeVisible();
	});

	test('authenticated session loads the home page without redirecting', async ({ page }) => {
		await mockAuthenticatedSession(page);
		await page.goto('/');
		await expect(page).not.toHaveURL(/\/login/);
	});
});
