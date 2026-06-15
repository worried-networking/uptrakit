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
// Exported as a plain const so derived configs (`playwright.behavior.config.ts`,
// `playwright.parity.config.ts`) can spread a known object — not the
// `defineConfig` return value, which may be re-processed by Playwright.
export const baseConfig = {
	testDir: './tests/e2e',
	timeout: 30_000,
	fullyParallel: true,
	forbidOnly: !!process.env.CI,
	retries: process.env.CI ? 2 : 0,
	workers: process.env.CI ? 1 : undefined,
	reporter: [['html', { open: 'never' }]] as const,
	// Keep snapshot names OS-agnostic; the ui-parity screenshot suite enforces a
	// canonical macOS Chromium execution guard to avoid cross-OS render drift.
	snapshotPathTemplate: '{testDir}/{testFilePath}-snapshots/{arg}-{projectName}{ext}',
	use: {
		baseURL: 'http://localhost:5173',
		trace: 'on-first-retry' as const,
		screenshot: 'only-on-failure' as const
	},
	projects: [
		{
			name: 'chromium',
			use: { ...devices['Desktop Chrome'], colorScheme: 'light' as const }
		},
		{
			name: 'chromium-dark',
			use: { ...devices['Desktop Chrome'], colorScheme: 'dark' as const }
		},
		{
			name: 'chromium-mobile',
			use: {
				...devices['Desktop Chrome'],
				colorScheme: 'light' as const,
				viewport: { width: 393, height: 852 }
			}
		},
		{
			name: 'chromium-mobile-dark',
			use: {
				...devices['Desktop Chrome'],
				colorScheme: 'dark' as const,
				viewport: { width: 393, height: 852 }
			}
		}
	],
	webServer: {
		command: 'npm run dev',
		url: 'http://localhost:5173',
		reuseExistingServer: !process.env.CI
	}
};

export default defineConfig(baseConfig);
