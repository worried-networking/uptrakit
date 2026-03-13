import { describe, expect, it } from 'vitest';
import { resolveExtensionFormAction } from './extensions.svelte';
import type { ExtensionResponse } from './types';

function makeFormExtension(actionIds: string[], preLoadAction?: string): ExtensionResponse {
	return {
		id: 'ext.form',
		label: 'Form Extension',
		priority: 10,
		placement: { type: 'page', nav_section: 'test' },
		required_permission: undefined,
		targeting: 'universal',
		ui: { type: 'form', fields: [], pre_load_action: preLoadAction },
		actions: actionIds.map((actionId) => ({
			action_id: actionId,
			label: actionId,
			destructive: false
		})),
		provider_count: 1
	};
}

describe('resolveExtensionFormAction', () => {
	it('prefers save_<suffix> paired with get_<suffix>', () => {
		const extension = makeFormExtension(['get_global_smtp', 'save_global_smtp'], 'get_global_smtp');
		expect(resolveExtensionFormAction(extension)?.action_id).toBe('save_global_smtp');
	});

	it('falls back to save', () => {
		const extension = makeFormExtension(['save']);
		expect(resolveExtensionFormAction(extension)?.action_id).toBe('save');
	});

	it('falls back to a single save_* action', () => {
		const extension = makeFormExtension(['save_settings']);
		expect(resolveExtensionFormAction(extension)?.action_id).toBe('save_settings');
	});

	it('returns undefined when no save action can be inferred', () => {
		const extension = makeFormExtension(['list', 'get_settings'], 'get_settings');
		expect(resolveExtensionFormAction(extension)).toBeUndefined();
	});
});
