import manifest from '../../theme/adapter-manifest.json';
import { describe, expect, it } from 'vitest';

// @ts-expect-error node:fs is not part of the browser-focused frontend type environment
const { readFileSync } = await import('node:fs');
// @ts-expect-error node:url is not part of the browser-focused frontend type environment
const { fileURLToPath } = await import('node:url');

function resolveFromThisTest(relativePath: string): string {
	const resolved = new URL(relativePath, import.meta.url);
	if (resolved.protocol === 'file:') {
		return fileURLToPath(resolved);
	}

	// Vitest can expose non-file module URLs; keep resolution anchored to this test URL.
	return decodeURIComponent(resolved.pathname).replace(/^\/@fs/, '');
}

const appCss = readFileSync(resolveFromThisTest('../../app.css'), 'utf8');

const canonicalTokens = [
	'--bg-base',
	'--bg-surface',
	'--bg-raised',
	'--border-subtle',
	'--border-default',
	'--text-inverted',
	'--text-primary',
	'--text-secondary',
	'--text-muted',
	'--accent',
	'--accent-rgb',
	'--accent-bright',
	'--accent-dark',
	'--accent-deep',
	'--color-success',
	'--color-success-bg',
	'--color-success-border',
	'--color-warning',
	'--color-warning-bg',
	'--color-warning-border',
	'--color-error',
	'--color-error-bg',
	'--color-error-border',
	'--color-info',
	'--color-info-bg',
	'--color-info-border'
] as const;

const expectedMappings = [
	{ token: '--bg-base', theme: 'dark', maps_to: '--color-surface-950' },
	{ token: '--bg-base', theme: 'light', maps_to: '--color-surface-50' },
	{ token: '--bg-surface', theme: 'dark', maps_to: '--color-surface-900' },
	{ token: '--bg-surface', theme: 'light', maps_to: '--color-surface-100' },
	{ token: '--bg-raised', theme: 'dark', maps_to: '--color-surface-800' },
	{ token: '--bg-raised', theme: 'light', maps_to: '--color-surface-200' },
	{ token: '--border-subtle', theme: 'dark', maps_to: '--color-surface-800' },
	{ token: '--border-subtle', theme: 'light', maps_to: '--color-surface-200' },
	{ token: '--border-default', theme: 'dark', maps_to: '--color-surface-700' },
	{ token: '--border-default', theme: 'light', maps_to: '--color-surface-300' },
	{ token: '--text-inverted', theme: 'dark', maps_to: '--color-surface-50' },
	{ token: '--text-inverted', theme: 'light', maps_to: '--color-surface-50' },
	{ token: '--text-primary', theme: 'dark', maps_to: '--color-surface-50' },
	{ token: '--text-primary', theme: 'light', maps_to: '--color-surface-950' },
	{ token: '--text-secondary', theme: 'dark', maps_to: '--color-surface-300' },
	{ token: '--text-secondary', theme: 'light', maps_to: '--color-surface-700' },
	{ token: '--text-muted', theme: 'dark', maps_to: '--color-surface-400' },
	{ token: '--text-muted', theme: 'light', maps_to: '--color-surface-500' },
	{ token: '--accent', theme: 'dark', maps_to: '--theme-accent' },
	{ token: '--accent', theme: 'light', maps_to: '--theme-accent' },
	{ token: '--accent-rgb', theme: 'dark', maps_to: '--theme-accent-rgb' },
	{ token: '--accent-rgb', theme: 'light', maps_to: '--theme-accent-rgb' },
	{ token: '--accent-bright', theme: 'dark', maps_to: '--theme-accent-bright' },
	{ token: '--accent-bright', theme: 'light', maps_to: '--theme-accent-bright' },
	{ token: '--accent-dark', theme: 'dark', maps_to: '--theme-accent-dark' },
	{ token: '--accent-dark', theme: 'light', maps_to: '--theme-accent-dark' },
	{ token: '--accent-deep', theme: 'dark', maps_to: '--theme-accent-deep' },
	{ token: '--accent-deep', theme: 'light', maps_to: '--theme-accent-deep' },
	{ token: '--color-success', theme: 'dark', maps_to: '--color-success-400' },
	{ token: '--color-success', theme: 'light', maps_to: '--color-success-600' },
	{ token: '--color-success-bg', theme: 'dark', maps_to: '--color-success-900' },
	{ token: '--color-success-bg', theme: 'light', maps_to: '--color-success-50' },
	{ token: '--color-success-border', theme: 'dark', maps_to: '--color-success-700' },
	{ token: '--color-success-border', theme: 'light', maps_to: '--color-success-200' },
	{ token: '--color-warning', theme: 'dark', maps_to: '--color-warning-400' },
	{ token: '--color-warning', theme: 'light', maps_to: '--color-warning-600' },
	{ token: '--color-warning-bg', theme: 'dark', maps_to: '--color-warning-900' },
	{ token: '--color-warning-bg', theme: 'light', maps_to: '--color-warning-50' },
	{ token: '--color-warning-border', theme: 'dark', maps_to: '--color-warning-700' },
	{ token: '--color-warning-border', theme: 'light', maps_to: '--color-warning-200' },
	{ token: '--color-error', theme: 'dark', maps_to: '--color-error-400' },
	{ token: '--color-error', theme: 'light', maps_to: '--color-error-600' },
	{ token: '--color-error-bg', theme: 'dark', maps_to: '--color-error-900' },
	{ token: '--color-error-bg', theme: 'light', maps_to: '--color-error-50' },
	{ token: '--color-error-border', theme: 'dark', maps_to: '--color-error-700' },
	{ token: '--color-error-border', theme: 'light', maps_to: '--color-error-200' },
	{ token: '--color-info', theme: 'dark', maps_to: '--theme-info' },
	{ token: '--color-info', theme: 'light', maps_to: '--theme-info' },
	{ token: '--color-info-bg', theme: 'dark', maps_to: '--theme-info-bg' },
	{ token: '--color-info-bg', theme: 'light', maps_to: '--theme-info-bg' },
	{ token: '--color-info-border', theme: 'dark', maps_to: '--theme-info-border' },
	{ token: '--color-info-border', theme: 'light', maps_to: '--theme-info-border' }
] as const;

describe('adapter manifest', () => {
	it('covers the full canonical semantic-token set for both themes with no gaps', () => {
		expect(manifest).toHaveLength(canonicalTokens.length * 2);
		expect(new Set(manifest.map(({ token, theme }) => `${token}|${theme}`))).toHaveLength(manifest.length);

		for (const token of canonicalTokens) {
			const mappingsForToken = manifest.filter((entry) => entry.token === token);
			expect(mappingsForToken).toHaveLength(2);
			expect(new Set(mappingsForToken.map((entry) => entry.theme))).toEqual(new Set(['dark', 'light']));
		}
	});

	it('pins each canonical mapping to the approved runtime token', () => {
		for (const mapping of expectedMappings) {
			expect(manifest).toContainEqual(mapping);
		}
	});

	it('pins the shared layering z-index contract in app.css', () => {
		expect(appCss).toMatch(/\[data-ui='app-shell-header'\][\s\S]*?z-index:\s*10;/);
		expect(appCss).toMatch(/\[data-ui='app-shell-sidebar'\][\s\S]*?z-index:\s*20;/);
		expect(appCss).toMatch(/\[data-ui='context-menu-shell'\][\s\S]*?z-index:\s*100;/);
		expect(appCss).toMatch(/\[data-ui='toast-notifications'\][\s\S]*?z-index:\s*500;/);
		expect(appCss).toMatch(/\[data-ui='modal-backdrop'\][\s\S]*?z-index:\s*900;/);
		expect(appCss).toMatch(/\[data-ui='modal-shell'\][\s\S]*?z-index:\s*910;/);
	});

	it('pins global transition and focus-visible interaction rules', () => {
		const transitionDeclarations = [...appCss.matchAll(/transition:\s*([^;]+);/g)].map((match) => match[1]);
		expect(transitionDeclarations.length).toBeGreaterThan(0);

		const allowedTransitionProperties = new Set(['background', 'border-color', 'color']);
		for (const declaration of transitionDeclarations) {
			const properties = declaration
				.split(',')
				.map((segment: string) => segment.trim().split(/\s+/)[0])
				.filter(Boolean);

			for (const property of properties) {
				expect(allowedTransitionProperties).toContain(property);
			}
		}

		expect(appCss).toMatch(
			/:is\(button, \[href\], input, select, textarea, summary, \[role='button'\], \[role='tab'\]\):focus-visible[\s\S]*?outline:\s*none;[\s\S]*?box-shadow:\s*0 0 0 3px rgba\(var\(--accent-rgb\), 0.25\);/
		);
		expect(appCss).toMatch(
			/:is\(input, select, textarea\)\[aria-invalid='true'\]:focus-visible[\s\S]*?border-color:\s*var\(--color-error-border\);/
		);
	});
});
