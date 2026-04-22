import { expect, test } from '@playwright/test';

async function mockAnonymousSession(page: import('@playwright/test').Page) {
	await page.route('**/api/v1/auth/refresh', (route) =>
		route.fulfill({ status: 401, json: { error: 'Unauthorized' } })
	);
	await page.route('**/api/v1/system/alerts', (route) => route.fulfill({ json: { alerts: [] } }));
}

async function mockAuthenticatedSession(page: import('@playwright/test').Page) {
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
}

async function mockAuthMethods(
	page: import('@playwright/test').Page,
	overrides: Partial<{
		password: boolean;
		oidc_providers: { id: string; name: string; logo_url: string | null }[];
		setup_required: boolean;
		registration_token_required: boolean;
	}> = {}
) {
	await page.route('**/api/v1/auth/methods', (route) =>
		route.fulfill({
			json: {
				password: true,
				oidc_providers: [],
				setup_required: false,
				registration_token_required: false,
				...overrides
			}
		})
	);
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

const SHELL_SELECTOR = '[data-ui="public-entry-shell"]';

function relativeLuminance(rgb: string): number {
	const [r, g, b] = rgb
		.match(/\d+/g)!
		.map(Number)
		.map((c) => {
			const s = c / 255;
			return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
		});
	return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrastRatio(l1: number, l2: number): number {
	const [lighter, darker] = l1 > l2 ? [l1, l2] : [l2, l1];
	return (lighter + 0.05) / (darker + 0.05);
}

test('Input error state meets WCAG AA contrast in light theme', async ({ page }) => {
	await mockAnonymousSession(page);
	await mockAuthMethods(page);
	await setTheme(page, 'light');
	await page.goto('/login');
	await page.waitForSelector('[data-ui="form-field-row"]');
	await page.locator('button[type="submit"]').click();
	const errorInput = page.locator('#login-email');
	await errorInput.waitFor();
	const bg = await errorInput.evaluate((el) => getComputedStyle(el).backgroundColor);
	const color = await errorInput.evaluate((el) => getComputedStyle(el).color);
	const ratio = contrastRatio(relativeLuminance(bg), relativeLuminance(color));
	expect(ratio).toBeGreaterThanOrEqual(4.5);
});

test('Input error state meets WCAG AA contrast in dark theme', async ({ page }) => {
	await mockAnonymousSession(page);
	await mockAuthMethods(page);
	await setTheme(page, 'dark');
	await page.goto('/login');
	await page.waitForSelector('[data-ui="form-field-row"]');
	await page.locator('button[type="submit"]').click();
	const errorInput = page.locator('#login-email');
	await errorInput.waitFor();
	const bg = await errorInput.evaluate((el) => getComputedStyle(el).backgroundColor);
	const color = await errorInput.evaluate((el) => getComputedStyle(el).color);
	const ratio = contrastRatio(relativeLuminance(bg), relativeLuminance(color));
	expect(ratio).toBeGreaterThanOrEqual(4.5);
});

test.describe('public-entry snapshots', () => {
	for (const theme of ['dark', 'light'] as const) {
		test.describe(theme, () => {
			test.beforeEach(async ({ page }) => {
				await setTheme(page, theme);
				await mockAnonymousSession(page);
			});

			test(`login default — ${theme}`, async ({ page }) => {
				await mockAuthMethods(page);
				await page.goto('/login');
				await page.waitForSelector(SHELL_SELECTOR);
				await page.waitForSelector('[data-ui="form-field-row"]');
				await expect(page.locator(SHELL_SELECTOR)).toHaveScreenshot(`login-default-${theme}.png`, {
					threshold: 0.005
				});
			});

			test(`login setup_required — ${theme}`, async ({ page }) => {
				await mockAuthMethods(page, { setup_required: true });
				await page.goto('/login');
				await page.waitForSelector(SHELL_SELECTOR);
				await page.waitForSelector('h1:has-text("Welcome to Uptrakit")');
				await page.waitForSelector('[data-ui="callout"]');
				await page.waitForSelector('[data-ui="form-field-row"]');
				await expect(page.locator(SHELL_SELECTOR)).toHaveScreenshot(`login-setup-required-${theme}.png`, {
					threshold: 0.005,
					maxDiffPixelRatio: 0.02
				});
			});

			test(`login registration-token required — ${theme}`, async ({ page }) => {
				await mockAuthMethods(page);
				await page.goto('/login#registration_token_required=true&registration_code=abc123');
				await page.waitForSelector(SHELL_SELECTOR);
				await page.waitForSelector('[data-ui="form-field-row"]');
				await expect(page.locator(SHELL_SELECTOR)).toHaveScreenshot(`login-registration-token-${theme}.png`, {
					threshold: 0.005
				});
			});

			test(`login link-required — ${theme}`, async ({ page }) => {
				await mockAuthMethods(page);
				await page.goto('/login?link_required=true&email=user%40example.com&link_provider_id=google&link_token=tok');
				await page.waitForSelector(SHELL_SELECTOR);
				await page.waitForSelector('[data-ui="form-field-row"]');
				await expect(page.locator(SHELL_SELECTOR)).toHaveScreenshot(`login-link-required-${theme}.png`, {
					threshold: 0.005
				});
			});

			test(`register — ${theme}`, async ({ page }) => {
				await mockAuthMethods(page);
				await page.goto('/register');
				await page.waitForSelector(SHELL_SELECTOR);
				await page.waitForSelector('[data-ui="form-field-row"]');
				await expect(page.locator(SHELL_SELECTOR)).toHaveScreenshot(`register-${theme}.png`, {
					threshold: 0.005
				});
			});

			test(`device unauthenticated — ${theme}`, async ({ page }) => {
				await mockAuthMethods(page);
				await page.goto('/device?code=BCDF-GHJK');
				await page.waitForSelector(SHELL_SELECTOR);
				await expect(page.locator(SHELL_SELECTOR)).toHaveScreenshot(`device-unauth-${theme}.png`, {
					threshold: 0.005
				});
			});

			test(`device authenticated — ${theme}`, async ({ page }) => {
				await mockAuthenticatedSession(page);
				await mockAuthMethods(page);
				await page.goto('/device?code=BCDF-GHJK');
				await page.waitForSelector(SHELL_SELECTOR);
				await page.waitForSelector('[role="button"]:has-text("Approve"), button:has-text("Approve")');
				await expect(page.locator(SHELL_SELECTOR)).toHaveScreenshot(`device-auth-${theme}.png`, {
					threshold: 0.005
				});
			});

			test(`error 404 — ${theme}`, async ({ page }) => {
				await page.goto('/definitely-missing');
				await page.waitForSelector(SHELL_SELECTOR);
				await expect(page.locator(SHELL_SELECTOR)).toHaveScreenshot(`error-404-${theme}.png`, {
					threshold: 0.005
				});
			});
		});
	}
});
