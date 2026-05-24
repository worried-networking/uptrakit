import { describe, expect, it } from 'vitest';
import { createFormDraft } from './draft.svelte';

describe('createFormDraft', () => {
	it('isDirty is false on creation', () => {
		const form = createFormDraft({ name: 'alice', enabled: true });
		expect(form.isDirty).toBe(false);
	});

	it('isDirty becomes true after update', () => {
		const form = createFormDraft({ name: 'alice', enabled: true });
		form.update('name', 'bob');
		expect(form.isDirty).toBe(true);
	});

	it('isFieldDirty tracks individual fields', () => {
		const form = createFormDraft({ name: 'alice', enabled: true });
		form.update('name', 'bob');
		expect(form.isFieldDirty('name')).toBe(true);
		expect(form.isFieldDirty('enabled')).toBe(false);
	});

	it('discard restores draft to serverValues', () => {
		const form = createFormDraft({ name: 'alice', enabled: true });
		form.update('name', 'bob');
		form.discard();
		expect(form.draft.name).toBe('alice');
		expect(form.isDirty).toBe(false);
	});

	it('load sets both serverValues and draft', () => {
		const form = createFormDraft({ name: 'alice', enabled: true });
		form.load({ name: 'carol', enabled: false });
		expect(form.draft.name).toBe('carol');
		expect(form.serverValues.name).toBe('carol');
		expect(form.isDirty).toBe(false);
	});

	it('commit sets serverValues to new state', () => {
		const form = createFormDraft({ name: 'alice', enabled: true });
		form.update('name', 'bob');
		form.commit({ name: 'bob', enabled: true });
		expect(form.serverValues.name).toBe('bob');
		expect(form.isDirty).toBe(false);
	});
});
