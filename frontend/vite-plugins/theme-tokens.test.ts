import { describe, expect, it, vi } from 'vitest';
import { themeTokensPlugin, VIRTUAL_ID } from './theme-tokens';
import { tokens } from '../src/theme/tokens';

type VitePluginHook<K extends keyof ReturnType<typeof themeTokensPlugin>> = ReturnType<typeof themeTokensPlugin>[K];

function callResolveId(id: string): string | undefined {
	const plugin = themeTokensPlugin();
	const resolveId = plugin.resolveId as VitePluginHook<'resolveId'>;
	if (typeof resolveId !== 'function') return undefined;
	const result = resolveId.call({} as never, id, undefined, {} as never);
	return typeof result === 'string' ? result : undefined;
}

function callLoad(id: string): string | undefined {
	const plugin = themeTokensPlugin();
	const load = plugin.load as VitePluginHook<'load'>;
	if (typeof load !== 'function') return undefined;
	const result = load.call({} as never, id, undefined);
	return typeof result === 'string' ? result : undefined;
}

describe('theme-tokens Vite plugin', () => {
	it('exposes the canonical virtual id constant', () => {
		expect(VIRTUAL_ID).toBe('virtual:theme/tokens.css');
	});

	it('resolveId returns the resolved id for the virtual module', () => {
		expect(callResolveId(VIRTUAL_ID)).toBe('\0' + VIRTUAL_ID);
	});

	it('resolveId returns undefined for unrelated ids', () => {
		expect(callResolveId('some-other-module')).toBeUndefined();
		expect(callResolveId('virtual:theme/other.css')).toBeUndefined();
	});

	it('load returns undefined for unrelated ids', () => {
		expect(callLoad('\0virtual:theme/other.css')).toBeUndefined();
	});

	it('load emits :root and .dark blocks for the resolved virtual id', () => {
		const css = callLoad('\0' + VIRTUAL_ID);
		expect(css).toBeDefined();
		expect(css).toContain(':root {');
		expect(css).toContain('color-scheme: light;');
		expect(css).toContain('.dark {');
		expect(css).toContain('color-scheme: dark;');
	});

	it('load declares every TokenName twice (once per theme)', () => {
		const css = callLoad('\0' + VIRTUAL_ID)!;
		for (const name of Object.keys(tokens)) {
			const occurrences = css.split(`${name}:`).length - 1;
			expect(occurrences, `${name} declaration count`).toBe(2);
		}
	});

	it('handleHotUpdate invalidates the virtual module when tokens.ts changes', () => {
		const plugin = themeTokensPlugin();
		const invalidateModule = vi.fn();
		const virtualModule = { id: '\0' + VIRTUAL_ID };
		const server = {
			moduleGraph: {
				getModuleById: vi.fn().mockReturnValue(virtualModule),
				invalidateModule
			}
		};

		const handleHotUpdate = plugin.handleHotUpdate as VitePluginHook<'handleHotUpdate'>;
		if (typeof handleHotUpdate !== 'function') {
			throw new Error('handleHotUpdate hook missing');
		}

		const ctx = {
			file: '/abs/path/to/frontend/src/theme/tokens.ts',
			server,
			modules: [],
			read: async () => '',
			timestamp: Date.now()
		} as never;
		const result = handleHotUpdate.call({} as never, ctx);

		expect(invalidateModule).toHaveBeenCalledWith(virtualModule);
		expect(Array.isArray(result) ? result : [result]).toContain(virtualModule);
	});

	it('handleHotUpdate ignores unrelated file changes', () => {
		const plugin = themeTokensPlugin();
		const invalidateModule = vi.fn();
		const server = {
			moduleGraph: {
				getModuleById: vi.fn(),
				invalidateModule
			}
		};

		const handleHotUpdate = plugin.handleHotUpdate as VitePluginHook<'handleHotUpdate'>;
		if (typeof handleHotUpdate !== 'function') {
			throw new Error('handleHotUpdate hook missing');
		}

		const ctx = {
			file: '/abs/path/to/frontend/src/routes/+page.svelte',
			server,
			modules: [],
			read: async () => '',
			timestamp: Date.now()
		} as never;
		handleHotUpdate.call({} as never, ctx);

		expect(invalidateModule).not.toHaveBeenCalled();
	});
});
