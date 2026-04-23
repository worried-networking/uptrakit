import { describe, expect, it } from 'vitest';
import { tokens, cssForTheme, getToken, type TokenName, type Theme } from './tokens';

const EXPECTED: Record<TokenName, Record<Theme, string>> = {
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
	'--color-danger': { dark: '#fdba74', light: '#dc2626' },
	'--color-danger-bg': {
		dark: 'rgba(234, 88, 12, 0.15)',
		light: 'rgba(220, 38, 38, 0.07)'
	},
	'--color-danger-border': {
		dark: 'rgba(234, 88, 12, 0.35)',
		light: 'rgba(220, 38, 38, 0.3)'
	},
	'--color-danger-bg-hover': {
		dark: 'rgba(234, 88, 12, 0.22)',
		light: 'rgba(220, 38, 38, 0.14)'
	},
	'--color-danger-border-hover': {
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

const EXPECTED_TOKEN_NAMES = Object.keys(EXPECTED) as TokenName[];

describe('tokens', () => {
	it('defines every TokenName for both dark and light themes', () => {
		for (const name of EXPECTED_TOKEN_NAMES) {
			expect(tokens[name], `missing entry for ${name}`).toBeDefined();
			expect(tokens[name].dark, `missing dark value for ${name}`).toBeTruthy();
			expect(tokens[name].light, `missing light value for ${name}`).toBeTruthy();
		}
	});

	it('pins every (name, theme) pair to the spec-approved value', () => {
		for (const name of EXPECTED_TOKEN_NAMES) {
			expect(tokens[name].dark, `dark ${name}`).toBe(EXPECTED[name].dark);
			expect(tokens[name].light, `light ${name}`).toBe(EXPECTED[name].light);
		}
	});

	it('exposes getToken as a lookup helper equivalent to the table', () => {
		for (const name of EXPECTED_TOKEN_NAMES) {
			expect(getToken(name, 'dark')).toBe(EXPECTED[name].dark);
			expect(getToken(name, 'light')).toBe(EXPECTED[name].light);
		}
	});

	it('accent-rgb values parse as three integers in 0..255 separated by single spaces', () => {
		for (const theme of ['dark', 'light'] as const) {
			const parts = tokens['--accent-rgb'][theme].split(' ');
			expect(parts).toHaveLength(3);
			for (const part of parts) {
				const n = Number(part);
				expect(Number.isInteger(n)).toBe(true);
				expect(n).toBeGreaterThanOrEqual(0);
				expect(n).toBeLessThanOrEqual(255);
			}
		}
	});

	it('cssForTheme emits every TokenName exactly once for the given theme', () => {
		for (const theme of ['dark', 'light'] as const) {
			const css = cssForTheme(theme);
			for (const name of EXPECTED_TOKEN_NAMES) {
				const occurrences = css.split(`${name}:`).length - 1;
				expect(occurrences, `${name} in ${theme} css`).toBe(1);
			}
		}
	});

	it('cssForTheme emits each pair as `  --name: value;` on its own line', () => {
		const css = cssForTheme('dark');
		for (const name of EXPECTED_TOKEN_NAMES) {
			expect(css).toContain(`  ${name}: ${EXPECTED[name].dark};`);
		}
	});
});
