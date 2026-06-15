import { createRequire } from 'node:module';
import { defineConfig } from '@playwright/test';
import { baseConfig } from './playwright.config';

// JSON loaded via createRequire (not `with { type: 'json' }`) because
// frontend `package.json` is `"type": "module"` with `moduleResolution: "bundler"`
// — import attributes are not reliably handled by Playwright's loader chain.
const require = createRequire(import.meta.url);
const buckets = require('./tests/e2e/buckets.json') as {
	behavior: string[];
	parity: string[];
	skipped: string[];
};

const chromiumLight = baseConfig.projects.find((p) => p.name === 'chromium');
if (!chromiumLight) {
	throw new Error('baseConfig is missing the "chromium" project');
}

export default defineConfig({
	...baseConfig,
	testMatch: buckets.behavior,
	// Behavior specs do not assert on theme/viewport — single project saves ~75% CI time.
	projects: [chromiumLight],
	// Override base `workers: 1` (set for parity screenshot stability).
	// Explicit `'50%'` over `undefined` so CI behavior is predictable.
	workers: '50%',
	fullyParallel: true
});
