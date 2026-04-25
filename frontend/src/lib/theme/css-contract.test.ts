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

describe('app.css structural contract', () => {
	it('pins the shared layering z-index contract in app.css', () => {
		expect(appCss).toMatch(/\[data-ui='app-shell-header'\][\s\S]*?z-index:\s*10;/);
		expect(appCss).toMatch(/\[data-ui='app-shell-sidebar'\][\s\S]*?z-index:\s*20;/);
		expect(appCss).toMatch(/\[data-ui='context-menu-shell'\][\s\S]*?z-index:\s*100;/);
		expect(appCss).toMatch(/\[data-ui='toast-notifications'\][\s\S]*?z-index:\s*920;/);
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
			/:is\(input, select, textarea\)\[aria-invalid='true'\]:focus-visible[\s\S]*?border-color:\s*var\(--color-danger-border\);/
		);
	});
});
