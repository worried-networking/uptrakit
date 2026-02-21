import js from '@eslint/js';
import ts from 'typescript-eslint';
import svelte from 'eslint-plugin-svelte';
import prettier from 'eslint-config-prettier';
import globals from 'globals';

export default ts.config(
	js.configs.recommended,
	...ts.configs.recommended,
	...svelte.configs['flat/recommended'],
	prettier,
	...svelte.configs['flat/prettier'],
	{
		languageOptions: {
			globals: {
				...globals.browser,
				...globals.node
			}
		}
	},
	{
		files: ['**/*.svelte', '**/*.svelte.ts', '**/*.svelte.js'],
		languageOptions: {
			parserOptions: {
				parser: ts.parser
			}
		}
	},
	{
		rules: {
			'@typescript-eslint/no-unused-vars': [
				'error',
				{
					argsIgnorePattern: '^_',
					varsIgnorePattern: '^_'
				}
			],
			// This SPA uses SvelteKit's static adapter (client-side only).
			// resolve() is for SSR route resolution and is not applicable here.
			'svelte/no-navigation-without-resolve': 'off',
			// Error cause preservation is desirable but the existing error handling
			// pattern in api.ts intentionally wraps errors for user-facing messages.
			'preserve-caught-error': 'off'
		}
	},
	{
		ignores: ['build/', '.svelte-kit/', 'dist/']
	}
);
