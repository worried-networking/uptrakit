import { describe, expect, it } from 'vitest';
import { Actions } from './local-types';

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

const spec = JSON.parse(readFileSync(resolveFromThisTest('../../../../crates/ui/web-api/openapi.json'), 'utf8')) as {
	components: {
		securitySchemes: {
			oauth2: { flows: { authorizationCode: { scopes: Record<string, string> } } };
		};
	};
};

describe('Actions constants', () => {
	it('every constant is an action the server catalog declares', () => {
		const scopes = Object.keys(spec.components.securitySchemes.oauth2.flows.authorizationCode.scopes);
		for (const action of Object.values(Actions)) {
			expect(scopes, `unknown action constant ${action}`).toContain(action);
		}
	});
});
