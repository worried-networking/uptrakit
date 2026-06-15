import { createRequire } from 'node:module';
import { defineConfig } from '@playwright/test';
import { baseConfig } from './playwright.config';

const require = createRequire(import.meta.url);
const buckets = require('./tests/e2e/buckets.json') as { parity: string[] };

export default defineConfig({
	...baseConfig,
	testMatch: buckets.parity
	// Inherit all 4 chromium projects + `workers: 1` from baseConfig.
	// Baselines are per-project; serial execution prevents pixel-diff flake.
});
