import { describe, expect, it } from 'vitest';

// @ts-expect-error node:fs is not part of the browser-focused frontend type environment
const { readFileSync } = await import('node:fs');
const appCss = readFileSync('src/app.css', 'utf8');

describe('design token CSS values', () => {
	it('pins the approved light-theme semantic color values in the raw stylesheet', () => {
		expect(appCss).toContain(':root {');
		expect(appCss).toContain('--bg-base: #f8fafc;');
		expect(appCss).toContain('--bg-surface: #ffffff;');
		expect(appCss).toContain('--bg-raised: #f1f5f9;');
		expect(appCss).toContain('--border-subtle: #e2e8f0;');
		expect(appCss).toContain('--border-default: #cbd5e1;');
		expect(appCss).toContain('--text-primary: #0f172a;');
		expect(appCss).toContain('--text-secondary: #64748b;');
		expect(appCss).toContain('--text-muted: #94a3b8;');
		expect(appCss).toContain('--text-inverted: #f8fafc;');
		expect(appCss).toContain('--theme-accent: #2563eb;');
		expect(appCss).toContain('--theme-accent-rgb: 37 99 235;');
		expect(appCss).toContain('--theme-accent-bright: #3b82f6;');
		expect(appCss).toContain('--theme-accent-dark: #1d4ed8;');
		expect(appCss).toContain('--theme-accent-deep: #1e40af;');
		expect(appCss).toContain('--color-success: #16a34a;');
		expect(appCss).toContain('--color-success-bg: rgba(22, 163, 74, 0.08);');
		expect(appCss).toContain('--color-success-border: rgba(22, 163, 74, 0.2);');
		expect(appCss).toContain('--color-warning: #d97706;');
		expect(appCss).toContain('--color-warning-bg: rgba(217, 119, 6, 0.1);');
		expect(appCss).toContain('--color-warning-border: rgba(217, 119, 6, 0.22);');
		expect(appCss).toContain('--color-error: #dc2626;');
		expect(appCss).toContain('--color-error-bg: rgba(220, 38, 38, 0.08);');
		expect(appCss).toContain('--color-error-border: rgba(220, 38, 38, 0.2);');
		expect(appCss).toContain('--theme-info: #0891b2;');
		expect(appCss).toContain('--theme-info-bg: rgba(8, 145, 178, 0.08);');
		expect(appCss).toContain('--theme-info-border: rgba(8, 145, 178, 0.22);');
	});

	it('pins the approved dark-theme semantic color values in the raw stylesheet', () => {
		expect(appCss).toContain('.dark {');
		expect(appCss).toContain('--bg-base: #09090b;');
		expect(appCss).toContain('--bg-surface: #111113;');
		expect(appCss).toContain('--bg-raised: #18181b;');
		expect(appCss).toContain('--border-subtle: #1c1c1f;');
		expect(appCss).toContain('--border-default: #27272a;');
		expect(appCss).toContain('--text-primary: #e4e4e7;');
		expect(appCss).toContain('--text-secondary: #a1a1aa;');
		expect(appCss).toContain('--text-muted: #52525b;');
		expect(appCss).toContain('--text-inverted: #09090b;');
		expect(appCss).toContain('--theme-accent: #06b6d4;');
		expect(appCss).toContain('--theme-accent-rgb: 6 182 212;');
		expect(appCss).toContain('--theme-accent-bright: #22d3ee;');
		expect(appCss).toContain('--theme-accent-dark: #0891b2;');
		expect(appCss).toContain('--theme-accent-deep: #0e7490;');
		expect(appCss).toContain('--color-success: #4ade80;');
		expect(appCss).toContain('--color-success-bg: rgba(74, 222, 128, 0.14);');
		expect(appCss).toContain('--color-success-border: rgba(74, 222, 128, 0.22);');
		expect(appCss).toContain('--color-warning: #fbbf24;');
		expect(appCss).toContain('--color-warning-bg: rgba(251, 191, 36, 0.14);');
		expect(appCss).toContain('--color-warning-border: rgba(251, 191, 36, 0.24);');
		expect(appCss).toContain('--color-error: #fdba74;');
		expect(appCss).toContain('--color-error-bg: rgba(253, 186, 116, 0.14);');
		expect(appCss).toContain('--color-error-border: rgba(253, 186, 116, 0.22);');
		expect(appCss).toContain('--theme-info: #67e8f9;');
		expect(appCss).toContain('--theme-info-bg: rgba(6, 182, 212, 0.1);');
		expect(appCss).toContain('--theme-info-border: rgba(6, 182, 212, 0.22);');
	});

	it('keeps info tokens distinct from accent tokens in both theme blocks', () => {
		expect(appCss).toContain('--theme-info: #0891b2;');
		expect(appCss).not.toContain('--theme-info: #2563eb;');
		expect(appCss).toContain('--theme-info: #67e8f9;');
		expect(appCss).not.toContain('--theme-info: #06b6d4;');
	});
});
