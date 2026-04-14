import { describe, expect, it } from 'vitest';
import layoutSource from './+layout.svelte?raw';
import extensionPageSource from './extensions/[id]/+page.svelte?raw';
import settingsPageSource from './settings/+page.svelte?raw';
import globalSettingsTabSource from './settings/GlobalSettingsTab.svelte?raw';
import softwarePageSource from './software/+page.svelte?raw';
import softwareDetailPageSource from './software/[id]/+page.svelte?raw';

const migratedRouteSources = [
	layoutSource,
	extensionPageSource,
	settingsPageSource,
	globalSettingsTabSource,
	softwarePageSource,
	softwareDetailPageSource
];

describe('shared-surface route migration', () => {
	it('does not import the legacy extension store in migrated routes', () => {
		for (const content of migratedRouteSources) {
			expect(content).not.toContain('$lib/extensions.svelte');
		}
	});

	it('does not import legacy extension-only renderer components in migrated routes', () => {
		for (const content of migratedRouteSources) {
			expect(content).not.toContain('$lib/components/extensions/ExtensionTabContent.svelte');
			expect(content).not.toContain('$lib/components/extensions/SchemaForm.svelte');
		}
	});
});
