import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	preprocess: vitePreprocess(),
	kit: {
		adapter: adapter({
			fallback: 'index.html'
		}),
		csp: {
			// Hash mode: SvelteKit computes sha256 hashes for every inline script it
			// generates (hydration data, module init, etc.) as well as any inline
			// scripts written directly in app.html (e.g. the theme-init snippet).
			// This replaces the hand-crafted hash in the app.html <meta> tag and
			// keeps the policy correct across builds without 'unsafe-inline'.
			mode: 'hash',
			directives: {
				'default-src': ['self'],
				'script-src': ['self'],
				// Tailwind/Skeleton inject critical styles inline; 'unsafe-inline'
				// for styles is acceptable — style injection cannot exfiltrate data
				// or execute code the way inline scripts can.
				'style-src': ['self', 'unsafe-inline'],
				// Allow any HTTPS image source so admin-configured OIDC provider
				// logos can be displayed. URLs are validated as HTTPS-only via
				// isValidLogoUrl() before use. See docs/security/auth-and-authorization.md.
				'img-src': ['self', 'https:'],
				'connect-src': ['self'],
				'font-src': ['self'],
				'object-src': ['none'],
				'base-uri': ['self'],
				'form-action': ['self']
			}
		}
	}
};

export default config;
