import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import type { ConsentDetails } from '../../src/lib/api/oauth';

// ---------------------------------------------------------------------------
// Session helpers (mirrors public-entry.spec.ts)
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
// Consent mock factory
// ---------------------------------------------------------------------------

const BASE_CONSENT: ConsentDetails = {
	client_id: 'client-abc',
	client_name: 'Test MCP Client',
	client_uri: null,
	redirect_uri: 'https://example.com/callback',
	redirect_uri_host: 'example.com',
	scopes: ['mcp:read'],
	created_via: 'manual',
	trusted_at: '2026-01-01T00:00:00Z',
	requires_typed_confirmation: false,
	typed_confirmation_value: '',
	metadata_change_diff: null
};

async function mockConsentGet(page: Page, requestId: string, overrides: Partial<ConsentDetails>) {
	const payload: ConsentDetails = { ...BASE_CONSENT, ...overrides };
	await page.route(`**/oauth/consent/${requestId}`, (route) => {
		if (route.request().method() === 'GET') {
			route.fulfill({ status: 200, json: payload });
		} else {
			route.fallback();
		}
	});
}

async function mockConsentApprove(page: Page, requestId: string, redirectTo: string) {
	await page.route(`**/oauth/consent/${requestId}/approve`, (route) =>
		route.fulfill({ status: 200, json: { redirect_to: redirectTo } })
	);
}

async function mockConsentDeny(page: Page, requestId: string, redirectTo: string) {
	await page.route(`**/oauth/consent/${requestId}/deny`, (route) =>
		route.fulfill({ status: 200, json: { redirect_to: redirectTo } })
	);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const ALLOW_BUTTON = 'button:has-text("Allow access")';
const DENY_BUTTON = 'button:has-text("Deny")';
const TYPED_INPUT = '[data-ui="typed-confirmation-input"]';
const WARNING_CALLOUT = '[data-ui="callout"][data-tone="warning"]';

async function navigateToConsent(page: Page, requestId: string) {
	await page.goto(`/oauth/consent/${requestId}`);
	// Wait until the details have loaded (the Allow button is only rendered after `details !== null`)
	await page.waitForSelector(ALLOW_BUTTON);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test.describe('oauth consent screen', () => {
	test.beforeEach(async ({ page }) => {
		await mockAuthenticatedSession(page);
	});

	test('unverified client — Allow button gated by typed confirmation', async ({ page }) => {
		const requestId = 'req-123';

		await mockConsentGet(page, requestId, {
			requires_typed_confirmation: true,
			typed_confirmation_value: 'example.com',
			trusted_at: null
		});

		await navigateToConsent(page, requestId);

		// Initially disabled — nothing typed yet
		await expect(page.locator(ALLOW_BUTTON)).toBeDisabled();

		// Type a wrong value — still disabled
		await page.locator(TYPED_INPUT).fill('wrong.com');
		await expect(page.locator(ALLOW_BUTTON)).toBeDisabled();

		// Clear and type the correct value — now enabled
		await page.locator(TYPED_INPUT).fill('example.com');
		await expect(page.locator(ALLOW_BUTTON)).toBeEnabled();
	});

	test('Allow with correct typed value calls approve and redirects', async ({ page }) => {
		const requestId = 'req-123';
		const redirectTo = 'http://localhost:5173/oauth-redirect-target';

		await mockConsentGet(page, requestId, {
			requires_typed_confirmation: true,
			typed_confirmation_value: 'example.com',
			trusted_at: null
		});
		await mockConsentApprove(page, requestId, redirectTo);

		await navigateToConsent(page, requestId);

		await page.locator(TYPED_INPUT).fill('example.com');
		await expect(page.locator(ALLOW_BUTTON)).toBeEnabled();

		const approveResponse = page.waitForResponse(`**/oauth/consent/${requestId}/approve`);
		await page.locator(ALLOW_BUTTON).click();
		await approveResponse;

		// The SPA sets window.location.href to the redirect URL; wait for URL change.
		await page.waitForURL('**/oauth-redirect-target', { timeout: 5_000 });
	});

	test('Deny calls deny endpoint and redirects', async ({ page }) => {
		const requestId = 'req-456';
		const redirectTo = 'http://localhost:5173/oauth-denied';

		await mockConsentGet(page, requestId, {
			requires_typed_confirmation: false,
			trusted_at: '2026-01-01T00:00:00Z'
		});
		await mockConsentDeny(page, requestId, redirectTo);

		await navigateToConsent(page, requestId);

		const denyResponse = page.waitForResponse(`**/oauth/consent/${requestId}/deny`);
		await page.locator(DENY_BUTTON).click();
		await denyResponse;

		await page.waitForURL('**/oauth-denied', { timeout: 5_000 });
	});

	test('localhost redirect_uri_host shows warning callout', async ({ page }) => {
		const requestId = 'req-789';

		await mockConsentGet(page, requestId, {
			redirect_uri_host: 'localhost',
			redirect_uri: 'http://localhost:8080/callback',
			trusted_at: '2026-01-01T00:00:00Z',
			requires_typed_confirmation: false
		});

		await navigateToConsent(page, requestId);

		await expect(page.locator(WARNING_CALLOUT)).toBeVisible();
		await expect(page.locator(WARNING_CALLOUT)).toContainText('local');
	});
});
