import { describe, expect, it } from 'vitest';
import { SURFACE_NAV_ICONS, resolveNavIcon } from './nav-icons';

describe('resolveNavIcon', () => {
	it('returns Box for an unknown icon name', () => {
		const result = resolveNavIcon('SomeUnknownIcon');
		expect(result).toBe(SURFACE_NAV_ICONS['Box']);
	});

	it('returns the correct component for a known icon name', () => {
		const result = resolveNavIcon('Package');
		expect(result).toBe(SURFACE_NAV_ICONS['Package']);
		expect(result).not.toBe(SURFACE_NAV_ICONS['Box']);
	});

	it('returns Box for empty string', () => {
		const result = resolveNavIcon('');
		expect(result).toBe(SURFACE_NAV_ICONS['Box']);
	});

	it('SURFACE_NAV_ICONS contains expected keys', () => {
		const expectedKeys = [
			'Box',
			'Cpu',
			'Database',
			'FileText',
			'Globe',
			'HardDrive',
			'History',
			'Layers',
			'Package',
			'Puzzle',
			'ScrollText',
			'Server',
			'ServerCog',
			'Settings',
			'Shield',
			'Tag',
			'Tags',
			'Wrench'
		];
		for (const key of expectedKeys) {
			expect(SURFACE_NAV_ICONS[key], `expected key "${key}" in SURFACE_NAV_ICONS`).toBeDefined();
		}
	});
});
