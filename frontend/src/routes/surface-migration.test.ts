import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { describe, expect, it } from 'vitest';
import layoutSource from './+layout.svelte?raw';
import surfacePageSource from './surfaces/[id]/+page.svelte?raw';
import settingsPageSource from './settings/+page.svelte?raw';
import globalSettingsTabSource from './settings/GlobalSettingsTab.svelte?raw';
import softwarePageSource from './software/+page.svelte?raw';
import softwareDetailPageSource from './software/[id]/+page.svelte?raw';

const legacyExtensionRoutePath = join(dirname(fileURLToPath(import.meta.url)), 'extensions/[id]/+page.ts');

const migratedRouteSources = [
	layoutSource,
	surfacePageSource,
	settingsPageSource,
	globalSettingsTabSource,
	softwarePageSource,
	softwareDetailPageSource
];

describe('shared-surface route migration', () => {
	it('removes the legacy /extensions/[id] compatibility route file', () => {
		expect(existsSync(legacyExtensionRoutePath)).toBe(false);
	});

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
