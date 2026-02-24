import { svelte } from '@sveltejs/vite-plugin-svelte';
import { fileURLToPath, URL } from 'url';
import { defineConfig } from 'vitest/config';

export default defineConfig({
	plugins: [svelte({ hot: false })],
	resolve: {
		// Force browser entry points so Svelte's client runtime is used in jsdom
		// (prevents "mount() is not available on the server" errors).
		conditions: ['browser'],
		alias: {
			// Mirror SvelteKit's $lib alias so component tests can import from $lib/...
			$lib: fileURLToPath(new URL('./src/lib', import.meta.url)),
			// Stub SvelteKit virtual modules that don't exist in jsdom
			'$app/state': fileURLToPath(new URL('./src/lib/test-mocks/app-state.ts', import.meta.url)),
			'$app/navigation': fileURLToPath(new URL('./src/lib/test-mocks/app-navigation.ts', import.meta.url))
		}
	},
	test: {
		environment: 'jsdom',
		// Expose vitest globals (describe, it, expect, vi, etc.) so that
		// @testing-library/jest-dom can call expect.extend() on import.
		globals: true,
		setupFiles: ['./src/test-setup.ts'],
		// Exclude Playwright E2E tests — they are run separately via `npm run test:e2e`.
		exclude: ['tests/e2e/**', 'node_modules/**'],
		coverage: {
			provider: 'v8',
			include: ['src/lib/**'],
			exclude: ['src/lib/**/*.test.ts', 'src/lib/**/*.test.svelte'],
			thresholds: {
				lines: 70,
				branches: 65,
				functions: 70
			}
		}
	}
});
