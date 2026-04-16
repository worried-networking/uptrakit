import { describe, expect, it } from 'vitest';
import layoutSource from './+layout.svelte?raw';
import surfacePageSource from './surfaces/[id]/+page.svelte?raw';
import settingsPageSource from './settings/+page.svelte?raw';
import globalSettingsTabSource from './settings/GlobalSettingsTab.svelte?raw';
import softwarePageSource from './software/+page.svelte?raw';
import softwareDetailPageSource from './software/[id]/+page.svelte?raw';

const migratedRouteSources = [
	layoutSource,
	surfacePageSource,
	settingsPageSource,
	globalSettingsTabSource,
	softwarePageSource,
	softwareDetailPageSource
];

describe('shared-surface route migration', () => {
	it('uses shared-surface runtime modules in migrated routes', () => {
		for (const content of migratedRouteSources) {
			expect(content).toContain('$lib/surfaces');
		}
	});

	it('keeps migrated routes on the canonical shared-surface page path', () => {
		for (const content of migratedRouteSources) {
			expect(content).not.toContain('/extensions/');
			expect(content).toContain('/surfaces/');
		}
	});
});
