import { defineConfig } from '@hey-api/openapi-ts';

export default defineConfig({
	input: '../crates/ui/web-api/openapi.json', // co-located with producer (spec D5)
	output: { path: './src/lib/api/generated', postProcess: ['prettier'] }, // `format` is deprecated in 0.99.x
	plugins: [
		{ name: '@hey-api/typescript', enums: 'javascript' }, // runtime const enums — current `Permission` is a runtime enum; call sites do `Permission.X` + `Object.values(Permission)` (R5)
		'@hey-api/sdk',
		{ name: '@hey-api/client-fetch', throwOnError: true } // throwOnError lives on the client plugin
	]
});
