import { describe, expect, it } from 'vitest';
import { cssForTheme, tokens, type TokenName, type Theme } from '../../theme/tokens';

const SPEC: Record<TokenName, Record<Theme, string>> = {
	'--bg-base': { dark: '#09090b', light: '#f8fafc' },
	'--bg-surface': { dark: '#111113', light: '#ffffff' },
	'--bg-raised': { dark: '#18181b', light: '#f1f5f9' },
	'--bg-hover': { dark: '#1e1e22', light: '#eef1f5' },
	'--border-subtle': { dark: '#1c1c1f', light: '#e2e8f0' },
	'--border-default': { dark: '#27272a', light: '#cbd5e1' },
	'--text-muted': { dark: '#52525b', light: '#94a3b8' },
	'--text-secondary': { dark: '#a1a1aa', light: '#64748b' },
	'--text-primary': { dark: '#e4e4e7', light: '#0f172a' },
	'--text-inverted': { dark: '#fafafa', light: '#ffffff' },
	'--accent': { dark: '#06b6d4', light: '#2563eb' },
	'--accent-rgb': { dark: '6 182 212', light: '37 99 235' },
	'--accent-bright': { dark: '#22d3ee', light: '#3b82f6' },
	'--accent-dark': { dark: '#0891b2', light: '#1d4ed8' },
	'--accent-deep': { dark: '#0e7490', light: '#1e40af' },
	'--color-success': { dark: '#4ade80', light: '#16a34a' },
	'--color-success-bg': {
		dark: 'rgba(74, 222, 128, 0.1)',
		light: 'rgba(22, 163, 74, 0.08)'
	},
	'--color-success-border': {
		dark: 'rgba(74, 222, 128, 0.25)',
		light: 'rgba(22, 163, 74, 0.3)'
	},
	'--color-warning': { dark: '#fbbf24', light: '#d97706' },
	'--color-warning-bg': {
		dark: 'rgba(251, 191, 36, 0.12)',
		light: 'rgba(217, 119, 6, 0.08)'
	},
	'--color-warning-border': {
		dark: 'rgba(251, 191, 36, 0.3)',
		light: 'rgba(217, 119, 6, 0.28)'
	},
	'--color-error': { dark: '#fdba74', light: '#dc2626' },
	'--color-error-bg': {
		dark: 'rgba(234, 88, 12, 0.15)',
		light: 'rgba(220, 38, 38, 0.07)'
	},
	'--color-error-border': {
		dark: 'rgba(234, 88, 12, 0.35)',
		light: 'rgba(220, 38, 38, 0.3)'
	},
	'--color-error-bg-hover': {
		dark: 'rgba(234, 88, 12, 0.22)',
		light: 'rgba(220, 38, 38, 0.14)'
	},
	'--color-error-border-hover': {
		dark: 'rgba(234, 88, 12, 0.5)',
		light: 'rgba(220, 38, 38, 0.45)'
	},
	'--color-info': { dark: '#67e8f9', light: '#0891b2' },
	'--color-info-bg': {
		dark: 'rgba(6, 182, 212, 0.1)',
		light: 'rgba(8, 145, 178, 0.08)'
	},
	'--color-info-border': {
		dark: 'rgba(6, 182, 212, 0.22)',
		light: 'rgba(8, 145, 178, 0.22)'
	}
};

const SPEC_NAMES = Object.keys(SPEC) as TokenName[];

describe('design token values', () => {
	it('pins every dark-theme token to the approved spec value', () => {
		for (const name of SPEC_NAMES) {
			expect(tokens[name].dark, `dark ${name}`).toBe(SPEC[name].dark);
		}
	});

	it('pins every light-theme token to the approved spec value', () => {
		for (const name of SPEC_NAMES) {
			expect(tokens[name].light, `light ${name}`).toBe(SPEC[name].light);
		}
	});

	it('keeps info tokens distinct from accent tokens in both themes', () => {
		expect(tokens['--color-info'].dark).not.toBe(tokens['--accent'].dark);
		expect(tokens['--color-info'].light).not.toBe(tokens['--accent'].light);
	});

	it('snapshot: cssForTheme(light) output matches the canonical form', () => {
		expect(cssForTheme('light')).toMatchInlineSnapshot(`
"  --bg-base: #f8fafc;
  --bg-surface: #ffffff;
  --bg-raised: #f1f5f9;
  --bg-hover: #eef1f5;
  --border-subtle: #e2e8f0;
  --border-default: #cbd5e1;
  --text-muted: #94a3b8;
  --text-secondary: #64748b;
  --text-primary: #0f172a;
  --text-inverted: #ffffff;
  --accent: #2563eb;
  --accent-rgb: 37 99 235;
  --accent-bright: #3b82f6;
  --accent-dark: #1d4ed8;
  --accent-deep: #1e40af;
  --color-success: #16a34a;
  --color-success-bg: rgba(22, 163, 74, 0.08);
  --color-success-border: rgba(22, 163, 74, 0.3);
  --color-warning: #d97706;
  --color-warning-bg: rgba(217, 119, 6, 0.08);
  --color-warning-border: rgba(217, 119, 6, 0.28);
  --color-error: #dc2626;
  --color-error-bg: rgba(220, 38, 38, 0.07);
  --color-error-border: rgba(220, 38, 38, 0.3);
  --color-error-bg-hover: rgba(220, 38, 38, 0.14);
  --color-error-border-hover: rgba(220, 38, 38, 0.45);
  --color-info: #0891b2;
  --color-info-bg: rgba(8, 145, 178, 0.08);
  --color-info-border: rgba(8, 145, 178, 0.22);"
`);
	});

	it('snapshot: cssForTheme(dark) output matches the canonical form', () => {
		expect(cssForTheme('dark')).toMatchInlineSnapshot(`
"  --bg-base: #09090b;
  --bg-surface: #111113;
  --bg-raised: #18181b;
  --bg-hover: #1e1e22;
  --border-subtle: #1c1c1f;
  --border-default: #27272a;
  --text-muted: #52525b;
  --text-secondary: #a1a1aa;
  --text-primary: #e4e4e7;
  --text-inverted: #fafafa;
  --accent: #06b6d4;
  --accent-rgb: 6 182 212;
  --accent-bright: #22d3ee;
  --accent-dark: #0891b2;
  --accent-deep: #0e7490;
  --color-success: #4ade80;
  --color-success-bg: rgba(74, 222, 128, 0.1);
  --color-success-border: rgba(74, 222, 128, 0.25);
  --color-warning: #fbbf24;
  --color-warning-bg: rgba(251, 191, 36, 0.12);
  --color-warning-border: rgba(251, 191, 36, 0.3);
  --color-error: #fdba74;
  --color-error-bg: rgba(234, 88, 12, 0.15);
  --color-error-border: rgba(234, 88, 12, 0.35);
  --color-error-bg-hover: rgba(234, 88, 12, 0.22);
  --color-error-border-hover: rgba(234, 88, 12, 0.5);
  --color-info: #67e8f9;
  --color-info-bg: rgba(6, 182, 212, 0.1);
  --color-info-border: rgba(6, 182, 212, 0.22);"
`);
	});
});
