import { defineConfig } from '@hey-api/openapi-ts';

export default defineConfig({
	input: '../crates/ui/web-api/openapi.json', // co-located with producer (spec D5)
	output: { path: './src/lib/api/generated', postProcess: ['prettier'] }, // `format` is deprecated in 0.99.x
	plugins: [
		{ name: '@hey-api/typescript', enums: 'javascript' }, // runtime const enums for the surviving generated enums (MfaMethod, ServiceStatus, …); the old Permission-union rationale is gone (M1.7)
		'@hey-api/sdk',
		{ name: '@hey-api/client-fetch', throwOnError: true } // throwOnError lives on the client plugin
	]
});
