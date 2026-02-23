import { defineConfig, devices } from '@playwright/test';

/**
 * Playwright end-to-end test configuration.
 *
 * All API calls are intercepted via `page.route()` inside each test, so a real
 * Uptrakit backend is NOT required. Tests run against the SvelteKit dev server
 * started automatically via the `webServer` option.
 *
 * Running locally:
 *   npx playwright install --with-deps chromium   # one-time browser install
 *   npm run test:e2e
 *
 * In CI add a dedicated job with `npm run test:e2e` after `npm ci`.
 */
export default defineConfig({
	testDir: './tests/e2e',
	timeout: 30_000,
	fullyParallel: true,
	forbidOnly: !!process.env.CI,
	retries: process.env.CI ? 2 : 0,
	workers: process.env.CI ? 1 : undefined,
	reporter: [['html', { open: 'never' }]],
	use: {
		baseURL: 'http://localhost:5173',
		trace: 'on-first-retry',
		screenshot: 'only-on-failure'
	},
	projects: [
		{
			name: 'chromium',
			use: { ...devices['Desktop Chrome'] }
		}
	],
	webServer: {
		command: 'npm run dev',
		url: 'http://localhost:5173',
		reuseExistingServer: !process.env.CI
	}
});
