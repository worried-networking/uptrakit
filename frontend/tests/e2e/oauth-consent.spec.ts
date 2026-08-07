import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import type { ConsentDetails } from '../../src/lib/api/oauth';

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
				actions: [],
				authority: 'ok'
			}
		})
	);
	await page.route('**/api/v1/system/alerts', (route) => route.fulfill({ json: { alerts: [] } }));
	// Layout loads surface registry on auth; return empty to avoid backend proxy.
	await page.route('**/api/v1/surfaces', (route) => route.fulfill({ status: 200, json: [] }));
	await page.route('**/api/v1/surfaces?**', (route) => route.fulfill({ status: 200, json: [] }));
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
		const req = route.request();
		// Only intercept fetch/XHR for the consent details. The document request
		// for the same URL must be handled by SvelteKit's client-side router (see
		// navigateToConsent), so fallback() here would hit Vite's `/oauth` proxy.
		if (req.method() !== 'GET' || req.resourceType() === 'document') {
			route.fallback();
			return;
		}
		route.fulfill({ status: 200, json: payload });
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
// Selectors
// ---------------------------------------------------------------------------

const APPROVE_BUTTON = 'button:has-text("Approve")';
const DENY_BUTTON = 'button:has-text("Deny")';
const CONSENT_PROMPT = '[data-ui="consent-prompt"]';
const DANGER_CALLOUT = '[data-ui="callout"][data-tone="danger"]';
const WARNING_CALLOUT = '[data-ui="callout"][data-tone="warning"]';

async function navigateToConsent(page: Page, requestId: string) {
	// Vite dev server proxies `/oauth/*` to the real backend, so a direct
	// document navigation to `/oauth/consent/{id}` never reaches SvelteKit.
	// Land on a non-proxied route first and trigger a SvelteKit client-side
	// navigation by clicking a synthesized anchor, so the consent page module
	// is fetched via the SvelteKit data loader (which is not proxied).
	await page.goto('/');
	await page.waitForLoadState('networkidle');
	await page.evaluate((id) => {
		const a = document.createElement('a');
		a.href = `/oauth/consent/${id}`;
		a.textContent = 'go';
		a.setAttribute('data-testid', '__nav_to_consent__');
		document.body.appendChild(a);
	}, requestId);
	await page.locator('[data-testid="__nav_to_consent__"]').click();
	await page.waitForSelector(APPROVE_BUTTON);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test.describe('oauth consent screen', () => {
	test.beforeEach(async ({ page }) => {
		await mockAuthenticatedSession(page);
	});

	test('trusted client — renders client name and Approve button', async ({ page }) => {
		const requestId = 'req-001';
		await mockConsentGet(page, requestId, {});

		await navigateToConsent(page, requestId);

		await expect(page.locator(CONSENT_PROMPT)).toContainText('Test MCP Client');
		await expect(page.locator(APPROVE_BUTTON)).toBeEnabled();
		await expect(page.locator(DENY_BUTTON)).toBeVisible();
	});

	test('unverified client — shows danger callout, Approve still enabled', async ({ page }) => {
		const requestId = 'req-002';
		await mockConsentGet(page, requestId, { trusted_at: null });

		await navigateToConsent(page, requestId);

		await expect(page.locator(DANGER_CALLOUT)).toBeVisible();
		await expect(page.locator(DANGER_CALLOUT)).toContainText('not been verified');
		// Approve is immediately available — no typed gate
		await expect(page.locator(APPROVE_BUTTON)).toBeEnabled();
	});

	test('Approve calls approve endpoint and redirects', async ({ page }) => {
		const requestId = 'req-003';
		const redirectTo = 'http://localhost:5173/oauth-redirect-target';

		await mockConsentGet(page, requestId, {});
		await mockConsentApprove(page, requestId, redirectTo);

		await navigateToConsent(page, requestId);

		const approveResponse = page.waitForResponse(`**/oauth/consent/${requestId}/approve`);
		await page.locator(APPROVE_BUTTON).click();
		await approveResponse;

		await page.waitForURL('**/oauth-redirect-target', { timeout: 5_000 });
	});

	test('Deny calls deny endpoint and redirects', async ({ page }) => {
		const requestId = 'req-004';
		const redirectTo = 'http://localhost:5173/oauth-denied';

		await mockConsentGet(page, requestId, {});
		await mockConsentDeny(page, requestId, redirectTo);

		await navigateToConsent(page, requestId);

		const denyResponse = page.waitForResponse(`**/oauth/consent/${requestId}/deny`);
		await page.locator(DENY_BUTTON).click();
		await denyResponse;

		await page.waitForURL('**/oauth-denied', { timeout: 5_000 });
	});

	test('localhost redirect_uri_host shows warning callout', async ({ page }) => {
		const requestId = 'req-005';

		await mockConsentGet(page, requestId, {
			redirect_uri_host: 'localhost',
			redirect_uri: 'http://localhost:8080/callback'
		});

		await navigateToConsent(page, requestId);

		await expect(page.locator(WARNING_CALLOUT)).toBeVisible();
		await expect(page.locator(WARNING_CALLOUT)).toContainText('local');
	});

	test('DCR client — shows warning callout', async ({ page }) => {
		const requestId = 'req-006';

		await mockConsentGet(page, requestId, {
			created_via: 'dcr',
			trusted_at: '2026-01-01T00:00:00Z'
		});

		await navigateToConsent(page, requestId);

		await expect(page.locator(WARNING_CALLOUT)).toBeVisible();
		await expect(page.locator(WARNING_CALLOUT)).toContainText('recently registered');
	});

	test('scope descriptions are shown as human-readable text', async ({ page }) => {
		const requestId = 'req-007';

		await mockConsentGet(page, requestId, { scopes: ['mcp:read', 'mcp:write'] });

		await navigateToConsent(page, requestId);

		await expect(page.locator(CONSENT_PROMPT)).toContainText('Read your uptrakit data');
		await expect(page.locator(CONSENT_PROMPT)).toContainText('Trigger software updates');
	});
});
