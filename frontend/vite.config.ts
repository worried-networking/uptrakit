import tailwindcss from '@tailwindcss/vite';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';
import { themeTokensPlugin } from './vite-plugins/theme-tokens';

export default defineConfig({
	plugins: [themeTokensPlugin(), tailwindcss(), sveltekit()],
	build: {
		modulePreload: {
			// The polyfill injects scripts via blob: URLs, which would require
			// 'blob:' in script-src and weaken the CSP. All browsers that can run
			// this app support <link rel="modulepreload"> natively.
			polyfill: false
		}
	},
	server: {
		proxy: {
			'/api': {
				target: 'https://localhost:8443',
				secure: false
			}
		}
	}
});
