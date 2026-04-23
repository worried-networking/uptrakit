import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import ContextMenuItem from './ContextMenuItem.svelte';

afterEach(() => {
	cleanup();
});

describe('ContextMenuItem', () => {
	it('renders the canonical menu row metrics', () => {
		render(ContextMenuItem, { label: 'Delete', destructive: true });

		const item = screen.getByRole('menuitem', { name: 'Delete' });
		expect(item.className).toContain('min-h-8');
		expect(item.className).toContain('px-3');
		expect(item.className).toContain('text-table-body');
		expect(item.className).toContain('text-[var(--color-danger)]');
	});
});
