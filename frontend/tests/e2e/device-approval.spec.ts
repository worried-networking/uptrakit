import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';

// ---------------------------------------------------------------------------
// Session helpers
// ---------------------------------------------------------------------------

async function mockAuthenticatedSession(page: Page) {
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

// ---------------------------------------------------------------------------
// Device mock helpers
// ---------------------------------------------------------------------------

async function mockDeviceLookupSuccess(page: Page) {
	await page.route('**/api/v1/auth/device/lookup*', (route) => {
		if (route.request().method() === 'GET') {
			route.fulfill({
				status: 200,
				json: { client_name: 'uptrakit CLI', expires_at: '2099-01-01T00:00:00Z' }
			});
		} else {
			route.fallback();
		}
	});
}

async function mockDeviceLookup404(page: Page) {
	await page.route('**/api/v1/auth/device/lookup*', (route) => {
		if (route.request().method() === 'GET') {
			route.fulfill({ status: 404, json: { error: 'not found' } });
		} else {
			route.fallback();
		}
	});
}

async function mockDeviceApprove(page: Page) {
	await page.route('**/api/v1/auth/device/approve', (route) => {
		if (route.request().method() === 'POST') {
			route.fulfill({ status: 200, json: { message: 'Device approved' } });
		} else {
			route.fallback();
		}
	});
}

async function mockDeviceDeny(page: Page) {
	await page.route('**/api/v1/auth/device/deny', (route) => {
		if (route.request().method() === 'POST') {
			route.fulfill({ status: 200, json: { message: 'Device denied' } });
		} else {
			route.fallback();
		}
	});
}

// ---------------------------------------------------------------------------
// Selectors
// ---------------------------------------------------------------------------

const CONSENT_PROMPT = '[data-ui="consent-prompt"]';
const APPROVE_BUTTON = 'button:has-text("Approve")';
const DENY_BUTTON = 'button:has-text("Deny")';
// Scoped to the page content card to avoid matching the layout's session-expired banner.
const ENTRY_CONTENT = '[data-ui="public-entry-content"]';
const SUCCESS_CALLOUT = `${ENTRY_CONTENT} [data-ui="callout"][data-tone="success"]`;
const WARNING_CALLOUT = `${ENTRY_CONTENT} [data-ui="callout"][data-tone="warning"]`;
const DANGER_CALLOUT = `${ENTRY_CONTENT} [data-ui="callout"][data-tone="danger"]`;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test.describe('device approval page', () => {
	test('pre-filled code triggers lookup and shows consent prompt', async ({ page }) => {
		await mockAuthenticatedSession(page);
		await mockDeviceLookupSuccess(page);

		const lookupDone = page.waitForResponse('**/api/v1/auth/device/lookup*');
		await page.goto('/device?user_code=BCDF-GHJK');
		await lookupDone;

		await expect(page.locator(CONSENT_PROMPT)).toBeVisible();
		await expect(page.locator(APPROVE_BUTTON)).toBeEnabled();
		await expect(page.locator(DENY_BUTTON)).toBeEnabled();
	});

	test('approve calls approve endpoint and shows success callout', async ({ page }) => {
		await mockAuthenticatedSession(page);
		await mockDeviceLookupSuccess(page);
		await mockDeviceApprove(page);

		const lookupDone = page.waitForResponse('**/api/v1/auth/device/lookup*');
		await page.goto('/device?user_code=BCDF-GHJK');
		await lookupDone;
		await expect(page.locator(CONSENT_PROMPT)).toBeVisible();

		const approveResponse = page.waitForResponse('**/api/v1/auth/device/approve');
		await page.locator(APPROVE_BUTTON).click();
		await approveResponse;

		await expect(page.locator(SUCCESS_CALLOUT)).toBeVisible();
	});

	test('deny calls deny endpoint and shows denied callout', async ({ page }) => {
		await mockAuthenticatedSession(page);
		await mockDeviceLookupSuccess(page);
		await mockDeviceDeny(page);

		const lookupDone = page.waitForResponse('**/api/v1/auth/device/lookup*');
		await page.goto('/device?user_code=BCDF-GHJK');
		await lookupDone;
		await expect(page.locator(CONSENT_PROMPT)).toBeVisible();

		const denyResponse = page.waitForResponse('**/api/v1/auth/device/deny');
		await page.locator(DENY_BUTTON).click();
		await denyResponse;

		await expect(page.locator(WARNING_CALLOUT)).toBeVisible();
	});

	test('invalid user_code shows error callout', async ({ page }) => {
		await mockAuthenticatedSession(page);
		await mockDeviceLookup404(page);

		const lookupDone = page.waitForResponse('**/api/v1/auth/device/lookup*');
		await page.goto('/device?user_code=BCDF-GHJK');
		await lookupDone;

		await expect(page.locator(DANGER_CALLOUT)).toBeVisible();
		await expect(page.locator(CONSENT_PROMPT)).not.toBeVisible();
	});

	test('unauthenticated user sees login prompt', async ({ page }) => {
		await page.route('**/api/v1/auth/refresh', (route) => route.fulfill({ status: 401, json: {} }));
		await page.route('**/api/v1/auth/me', (route) => route.fulfill({ status: 401, json: {} }));
		await page.route('**/api/v1/system/alerts', (route) => route.fulfill({ json: { alerts: [] } }));

		await page.goto('/device?user_code=BCDF-GHJK');
		await expect(page.locator('[data-ui="public-entry-content"]')).toBeVisible();

		await expect(page.locator(`${ENTRY_CONTENT} a[href*="/login?redirect"]`)).toBeVisible();
		await expect(page.locator(CONSENT_PROMPT)).not.toBeVisible();
	});
});
