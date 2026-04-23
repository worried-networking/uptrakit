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

	it('emits the spec-pinned golden CSS for both themes', () => {
		const css = callLoad('\0' + VIRTUAL_ID)!;
		const expected = [
			':root {',
			'  color-scheme: light;',
			'  --bg-base: #f8fafc;',
			'  --bg-surface: #ffffff;',
			'  --bg-raised: #f1f5f9;',
			'  --bg-hover: #eef1f5;',
			'  --border-subtle: #e2e8f0;',
			'  --border-default: #cbd5e1;',
			'  --text-muted: #94a3b8;',
			'  --text-secondary: #64748b;',
			'  --text-primary: #0f172a;',
			'  --text-inverted: #ffffff;',
			'  --accent: #2563eb;',
			'  --accent-rgb: 37 99 235;',
			'  --accent-bright: #3b82f6;',
			'  --accent-dark: #1d4ed8;',
			'  --accent-deep: #1e40af;',
			'  --color-success: #16a34a;',
			'  --color-success-bg: rgba(22, 163, 74, 0.08);',
			'  --color-success-border: rgba(22, 163, 74, 0.3);',
			'  --color-warning: #d97706;',
			'  --color-warning-bg: rgba(217, 119, 6, 0.08);',
			'  --color-warning-border: rgba(217, 119, 6, 0.28);',
			'  --color-danger: #dc2626;',
			'  --color-danger-bg: rgba(220, 38, 38, 0.07);',
			'  --color-danger-border: rgba(220, 38, 38, 0.3);',
			'  --color-danger-bg-hover: rgba(220, 38, 38, 0.14);',
			'  --color-danger-border-hover: rgba(220, 38, 38, 0.45);',
			'  --color-info: #0891b2;',
			'  --color-info-bg: rgba(8, 145, 178, 0.08);',
			'  --color-info-border: rgba(8, 145, 178, 0.22);',
			'}',
			'.dark {',
			'  color-scheme: dark;',
			'  --bg-base: #09090b;',
			'  --bg-surface: #111113;',
			'  --bg-raised: #18181b;',
			'  --bg-hover: #1e1e22;',
			'  --border-subtle: #1c1c1f;',
			'  --border-default: #27272a;',
			'  --text-muted: #52525b;',
			'  --text-secondary: #a1a1aa;',
			'  --text-primary: #e4e4e7;',
			'  --text-inverted: #fafafa;',
			'  --accent: #06b6d4;',
			'  --accent-rgb: 6 182 212;',
			'  --accent-bright: #22d3ee;',
			'  --accent-dark: #0891b2;',
			'  --accent-deep: #0e7490;',
			'  --color-success: #4ade80;',
			'  --color-success-bg: rgba(74, 222, 128, 0.1);',
			'  --color-success-border: rgba(74, 222, 128, 0.25);',
			'  --color-warning: #fbbf24;',
			'  --color-warning-bg: rgba(251, 191, 36, 0.12);',
			'  --color-warning-border: rgba(251, 191, 36, 0.3);',
			'  --color-danger: #fdba74;',
			'  --color-danger-bg: rgba(234, 88, 12, 0.15);',
			'  --color-danger-border: rgba(234, 88, 12, 0.35);',
			'  --color-danger-bg-hover: rgba(234, 88, 12, 0.22);',
			'  --color-danger-border-hover: rgba(234, 88, 12, 0.5);',
			'  --color-info: #67e8f9;',
			'  --color-info-bg: rgba(6, 182, 212, 0.1);',
			'  --color-info-border: rgba(6, 182, 212, 0.22);',
			'}',
			''
		].join('\n');

		expect(css).toBe(expected);
	});
});
