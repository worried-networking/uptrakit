export type Theme = 'dark' | 'light';

export type TokenName =
	| '--bg-base'
	| '--bg-surface'
	| '--bg-raised'
	| '--border-subtle'
	| '--border-default'
	| '--text-muted'
	| '--text-secondary'
	| '--text-primary'
	| '--text-inverted'
	| '--accent'
	| '--accent-rgb'
	| '--accent-bright'
	| '--accent-dark'
	| '--accent-deep'
	| '--color-success'
	| '--color-success-bg'
	| '--color-success-border'
	| '--color-warning'
	| '--color-warning-bg'
	| '--color-warning-border'
	| '--color-error'
	| '--color-error-bg'
	| '--color-error-border'
	| '--color-error-bg-hover'
	| '--color-error-border-hover'
	| '--color-info'
	| '--color-info-bg'
	| '--color-info-border';

export type TokenValue = string;

/** Emit `rgba(R, G, B, A)` from a space-separated RGB base and an alpha. */
function rgba(base: string, alpha: number): TokenValue {
	const [r, g, b] = base.split(' ');
	// Strip trailing zeros so `0.10` → `0.1` and `0.30` → `0.3`.
	const a = String(Number(alpha.toFixed(3)));
	return `rgba(${r}, ${g}, ${b}, ${a})`;
}

const successBase = { dark: '74 222 128', light: '22 163 74' };
const warningBase = { dark: '251 191 36', light: '217 119 6' };
const errorBase = { dark: '234 88 12', light: '220 38 38' };
const infoBase = { dark: '6 182 212', light: '8 145 178' };

export const tokens: Record<TokenName, Record<Theme, TokenValue>> = {
	'--bg-base': { dark: '#09090b', light: '#f8fafc' },
	'--bg-surface': { dark: '#111113', light: '#ffffff' },
	'--bg-raised': { dark: '#18181b', light: '#f1f5f9' },
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
		dark: rgba(successBase.dark, 0.1),
		light: rgba(successBase.light, 0.08)
	},
	'--color-success-border': {
		dark: rgba(successBase.dark, 0.25),
		light: rgba(successBase.light, 0.3)
	},
	'--color-warning': { dark: '#fbbf24', light: '#d97706' },
	'--color-warning-bg': {
		dark: rgba(warningBase.dark, 0.12),
		light: rgba(warningBase.light, 0.08)
	},
	'--color-warning-border': {
		dark: rgba(warningBase.dark, 0.3),
		light: rgba(warningBase.light, 0.28)
	},
	'--color-error': { dark: '#fdba74', light: '#dc2626' },
	'--color-error-bg': {
		dark: rgba(errorBase.dark, 0.15),
		light: rgba(errorBase.light, 0.07)
	},
	'--color-error-border': {
		dark: rgba(errorBase.dark, 0.35),
		light: rgba(errorBase.light, 0.3)
	},
	'--color-error-bg-hover': {
		dark: rgba(errorBase.dark, 0.22),
		light: rgba(errorBase.light, 0.14)
	},
	'--color-error-border-hover': {
		dark: rgba(errorBase.dark, 0.5),
		light: rgba(errorBase.light, 0.45)
	},
	'--color-info': { dark: '#67e8f9', light: '#0891b2' },
	'--color-info-bg': {
		dark: rgba(infoBase.dark, 0.1),
		light: rgba(infoBase.light, 0.08)
	},
	'--color-info-border': {
		dark: rgba(infoBase.dark, 0.22),
		light: rgba(infoBase.light, 0.22)
	}
};

const TOKEN_NAMES = Object.keys(tokens) as TokenName[];

/** Emit `  --name: value;` lines for one theme block. */
export function cssForTheme(theme: Theme): string {
	return TOKEN_NAMES.map((name) => `  ${name}: ${tokens[name][theme]};`).join('\n');
}

/** Lookup helper for programmatic consumers (terminal shell, xterm theme). */
export function getToken(name: TokenName, theme: Theme): TokenValue {
	return tokens[name][theme];
}
