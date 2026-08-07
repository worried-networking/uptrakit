/**
 * Tests for the login page's behaviour when an OAuth AS redirects an
 * unauthenticated browser request to /login?redirect=/oauth/authorize&_auth_context=oauth.
 *
 * The root problem: SvelteKit's goto() for a non-SvelteKit route (/oauth/authorize)
 * falls back to a hard browser reload. That reload has no Authorization header, so
 * the server redirects to /login again, creating an infinite loop until 429.
 *
 * The fix: when _auth_context=oauth, the login page must call the non-SvelteKit
 * /oauth/authorize endpoint via fetch() with an Authorization: Bearer header, then
 * navigate to the SvelteKit consent page returned in the redirect Location.
 */

import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';

// ---------------------------------------------------------------------------
// Shared helpers (same pattern as oauth-consent.spec.ts)
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
	// Layout calls loadSurfaceRegistry() once user is authenticated; mock to
	// keep the dev server proxy out of the test path.
	await page.route('**/api/v1/surfaces', (route) => route.fulfill({ status: 200, json: [] }));
}

// ---------------------------------------------------------------------------
// Minimal consent payload (mirrors BASE_CONSENT in oauth-consent.spec.ts)
// ---------------------------------------------------------------------------

const CONSENT_PAYLOAD = {
	client_id: 'https://claude.ai/oauth/claude-code-client-metadata',
	client_name: 'Claude Code',
	client_uri: 'https://claude.ai',
	redirect_uri: 'http://localhost:9999/callback',
	redirect_uri_host: 'localhost',
	scopes: ['mcp:read', 'mcp:write'],
	created_via: 'manual',
	trusted_at: '2026-01-01T00:00:00Z',
	requires_typed_confirmation: false,
	typed_confirmation_value: '',
	metadata_change_diff: null
};

// ---------------------------------------------------------------------------
// Shared OAuth params (same as what Claude Code sends)
// ---------------------------------------------------------------------------

const OAUTH_PARAMS =
	'response_type=code' +
	'&client_id=https%3A%2F%2Fclaude.ai%2Foauth%2Fclaude-code-client-metadata' +
	'&redirect_uri=http%3A%2F%2Flocalhost%3A9999%2Fcallback' +
	'&scope=mcp%3Aread+mcp%3Awrite' +
	'&state=test-state-123' +
	'&code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM' +
	'&code_challenge_method=S256';

const OAUTH_PATH = `/oauth/authorize?${OAUTH_PARAMS}`;
const ENCODED_OAUTH_PATH = encodeURIComponent(OAUTH_PATH);

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test.describe('login page OAuth redirect', () => {
	const REQUEST_ID = 'consent-req-abc123';

	/**
	 * Mock /oauth/authorize to behave like the real backend:
	 * - Request with Authorization: Bearer → 302 to consent page (authenticated)
	 * - Request without Authorization → 302 back to /login (unauthenticated)
	 *
	 * Returns a function that reports whether a Bearer-authenticated call was made.
	 */
	async function mockOauthAuthorize(page: Page, requestId: string) {
		let calledWithBearer = false;
		await page.route('**/oauth/authorize**', async (route) => {
			const auth = route.request().headers()['authorization'];
			if (auth?.startsWith('Bearer ')) {
				calledWithBearer = true;
				await route.fulfill({
					status: 302,
					headers: { location: `/oauth/consent/${requestId}` }
				});
			} else {
				// Simulate server redirecting unauthenticated browser navigation to login
				await route.fulfill({
					status: 302,
					headers: {
						location: `/login?redirect=${ENCODED_OAUTH_PATH}&_auth_context=oauth`
					}
				});
			}
		});
		return () => calledWithBearer;
	}

	/** Mock the backend consent data endpoint (called by the SvelteKit consent page). */
	async function mockConsentEndpoint(page: Page, requestId: string) {
		await page.route(`**/oauth/consent/${requestId}`, async (route) => {
			if (route.request().method() === 'GET') {
				await route.fulfill({ status: 200, json: CONSENT_PAYLOAD });
			} else {
				await route.fallback();
			}
		});
	}

	test('navigates to consent page without looping when user is authenticated', async ({ page }) => {
		// Track distinct visits to /login — more than 1 distinct landing means
		// the loop occurred. `framenavigated` can fire multiple times for the
		// same URL when SvelteKit replays its router effects on the same path
		// (e.g. effect re-runs, OIDC code clearing), so consecutive duplicates
		// of the same URL are coalesced.
		const loginVisits: string[] = [];
		page.on('framenavigated', (frame) => {
			if (frame !== page.mainFrame()) return;
			if (new URL(frame.url()).pathname !== '/login') return;
			if (loginVisits[loginVisits.length - 1] === frame.url()) return;
			loginVisits.push(frame.url());
		});

		await mockAuthenticatedSession(page);
		const wasCalledWithBearer = await mockOauthAuthorize(page, REQUEST_ID);
		await mockConsentEndpoint(page, REQUEST_ID);

		// Start where the server lands after redirecting the unauthenticated browser
		// navigation from /oauth/authorize (this is what actually happens in production).
		await page.goto(`/login?redirect=${ENCODED_OAUTH_PATH}&_auth_context=oauth`);

		// Must reach the consent page within 5 s — a timeout here means the loop fired.
		await page.waitForURL(`**/oauth/consent/${REQUEST_ID}`, { timeout: 5_000 });
		// ConsentPrompt renames the primary action from "Allow access" → "Approve"
		// after commit 9df993227 (redesign with ConsentPrompt component).
		await page.waitForSelector('button:has-text("Approve")');

		// /login should have been visited exactly once — the initial goto above.
		// More visits = the loop reproduced = the fix is broken.
		expect(loginVisits).toHaveLength(1);

		// The authorize endpoint must have been called with a Bearer token, not via a
		// bare browser navigation (which would have no Authorization header).
		expect(wasCalledWithBearer()).toBe(true);
	});

	test('does not loop when the server returns 302 to /login for bare navigation', async ({ page }) => {
		// Counts how many times /oauth/authorize is hit without Bearer — indicates the
		// old goto() hard-reload pattern: should be 0 in the fixed code.
		let unauthenticatedAuthorizeHits = 0;
		await mockAuthenticatedSession(page);
		await page.route('**/oauth/authorize**', async (route) => {
			const auth = route.request().headers()['authorization'];
			if (auth?.startsWith('Bearer ')) {
				await route.fulfill({
					status: 302,
					headers: { location: `/oauth/consent/${REQUEST_ID}` }
				});
			} else {
				unauthenticatedAuthorizeHits++;
				// After 3 unauthenticated hits, bail to prevent the test hanging forever
				// (3 is enough to prove the loop; waitForURL will timeout anyway)
				await route.fulfill({
					status: 302,
					headers: {
						location: `/login?redirect=${ENCODED_OAUTH_PATH}&_auth_context=oauth`
					}
				});
			}
		});
		await mockConsentEndpoint(page, REQUEST_ID);

		await page.goto(`/login?redirect=${ENCODED_OAUTH_PATH}&_auth_context=oauth`);
		await page.waitForURL(`**/oauth/consent/${REQUEST_ID}`, { timeout: 5_000 });

		// Fixed code: /oauth/authorize is never called without Bearer.
		expect(unauthenticatedAuthorizeHits).toBe(0);
	});
});
