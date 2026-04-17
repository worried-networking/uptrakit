import manifest from '../../theme/adapter-manifest.json';
import { describe, expect, it } from 'vitest';

const expectedMappings = [
	{ token: '--bg-base', theme: 'dark', maps_to: '--color-surface-950' },
	{ token: '--bg-base', theme: 'light', maps_to: '--color-surface-50' },
	{ token: '--bg-surface', theme: 'dark', maps_to: '--color-surface-900' },
	{ token: '--bg-surface', theme: 'light', maps_to: '--color-surface-100' },
	{ token: '--bg-raised', theme: 'dark', maps_to: '--color-surface-800' },
	{ token: '--bg-raised', theme: 'light', maps_to: '--color-surface-200' },
	{ token: '--border-subtle', theme: 'dark', maps_to: '--color-surface-800' },
	{ token: '--border-subtle', theme: 'light', maps_to: '--color-surface-200' },
	{ token: '--border-default', theme: 'dark', maps_to: '--color-surface-700' },
	{ token: '--border-default', theme: 'light', maps_to: '--color-surface-300' },
	{ token: '--text-inverted', theme: 'dark', maps_to: '--color-surface-50' },
	{ token: '--text-inverted', theme: 'light', maps_to: '--color-surface-50' },
	{ token: '--text-primary', theme: 'dark', maps_to: '--color-surface-50' },
	{ token: '--text-primary', theme: 'light', maps_to: '--color-surface-950' },
	{ token: '--text-secondary', theme: 'dark', maps_to: '--color-surface-300' },
	{ token: '--text-secondary', theme: 'light', maps_to: '--color-surface-700' },
	{ token: '--text-muted', theme: 'dark', maps_to: '--color-surface-400' },
	{ token: '--text-muted', theme: 'light', maps_to: '--color-surface-500' },
	{ token: '--accent', theme: 'dark', maps_to: '--color-primary-500' },
	{ token: '--accent', theme: 'light', maps_to: '--color-primary-500' },
	{ token: '--accent-rgb', theme: 'dark', maps_to: '--color-primary-500' },
	{ token: '--accent-rgb', theme: 'light', maps_to: '--color-primary-500' },
	{ token: '--accent-bright', theme: 'dark', maps_to: '--color-primary-400' },
	{ token: '--accent-bright', theme: 'light', maps_to: '--color-primary-400' },
	{ token: '--accent-dark', theme: 'dark', maps_to: '--color-primary-600' },
	{ token: '--accent-dark', theme: 'light', maps_to: '--color-primary-600' },
	{ token: '--accent-deep', theme: 'dark', maps_to: '--color-primary-700' },
	{ token: '--accent-deep', theme: 'light', maps_to: '--color-primary-700' },
	{ token: '--color-success', theme: 'dark', maps_to: '--color-success-400' },
	{ token: '--color-success', theme: 'light', maps_to: '--color-success-600' },
	{ token: '--color-success-bg', theme: 'dark', maps_to: '--color-success-900' },
	{ token: '--color-success-bg', theme: 'light', maps_to: '--color-success-50' },
	{ token: '--color-success-border', theme: 'dark', maps_to: '--color-success-700' },
	{ token: '--color-success-border', theme: 'light', maps_to: '--color-success-200' },
	{ token: '--color-warning', theme: 'dark', maps_to: '--color-warning-400' },
	{ token: '--color-warning', theme: 'light', maps_to: '--color-warning-600' },
	{ token: '--color-warning-bg', theme: 'dark', maps_to: '--color-warning-900' },
	{ token: '--color-warning-bg', theme: 'light', maps_to: '--color-warning-50' },
	{ token: '--color-warning-border', theme: 'dark', maps_to: '--color-warning-700' },
	{ token: '--color-warning-border', theme: 'light', maps_to: '--color-warning-200' },
	{ token: '--color-error', theme: 'dark', maps_to: '--color-error-400' },
	{ token: '--color-error', theme: 'light', maps_to: '--color-error-600' },
	{ token: '--color-error-bg', theme: 'dark', maps_to: '--color-error-900' },
	{ token: '--color-error-bg', theme: 'light', maps_to: '--color-error-50' },
	{ token: '--color-error-border', theme: 'dark', maps_to: '--color-error-700' },
	{ token: '--color-error-border', theme: 'light', maps_to: '--color-error-200' },
	{ token: '--color-info', theme: 'dark', maps_to: '--color-info-400' },
	{ token: '--color-info', theme: 'light', maps_to: '--color-info-600' },
	{ token: '--color-info-bg', theme: 'dark', maps_to: '--color-info-900' },
	{ token: '--color-info-bg', theme: 'light', maps_to: '--color-info-50' },
	{ token: '--color-info-border', theme: 'dark', maps_to: '--color-info-700' },
	{ token: '--color-info-border', theme: 'light', maps_to: '--color-info-200' }
] as const;

describe('adapter manifest', () => {
	it('covers all required semantic token mappings in dark and light themes', () => {
		expect(manifest).toHaveLength(expectedMappings.length);
		for (const mapping of expectedMappings) {
			expect(manifest).toContainEqual(mapping);
		}
	});
});
