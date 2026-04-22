import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import BatchActionBar from './BatchActionBar.svelte';

afterEach(() => {
	cleanup();
	vi.clearAllMocks();
});

type Action = {
	id: string;
	label: string;
	destructive?: boolean;
	variant?: 'primary' | 'secondary' | 'danger';
	loading?: boolean;
};

describe('BatchActionBar', () => {
	it('non-destructive action renders variant="primary"', () => {
		const actions: Action[] = [{ id: 'do', label: 'Do It', destructive: false }];
		render(BatchActionBar, { selectedCount: 2, actions, onaction: vi.fn(), oncancel: vi.fn() });
		const btn = screen.getByRole('button', { name: 'Do It' });
		expect(btn.className).toMatch(/bg-\[linear-gradient/);
	});

	it('destructive action renders variant="danger"', () => {
		const actions: Action[] = [{ id: 'del', label: 'Delete', destructive: true }];
		render(BatchActionBar, { selectedCount: 1, actions, onaction: vi.fn(), oncancel: vi.fn() });
		expect(screen.getByRole('button', { name: 'Delete' }).className).toMatch(/danger|error/);
	});

	it('explicit variant override wins over destructive flag', () => {
		const actions: Action[] = [{ id: 'act', label: 'Mark', destructive: true, variant: 'secondary' }];
		render(BatchActionBar, { selectedCount: 1, actions, onaction: vi.fn(), oncancel: vi.fn() });
		const btn = screen.getByRole('button', { name: 'Mark' });
		expect(btn.className).toContain('border');
		expect(btn.className).not.toMatch(/danger|error/);
	});

	it('action with loading=true has aria-busy="true"', () => {
		const actions: Action[] = [
			{ id: 'a', label: 'Action A', loading: true },
			{ id: 'b', label: 'Action B', loading: false }
		];
		render(BatchActionBar, { selectedCount: 2, actions, onaction: vi.fn(), oncancel: vi.fn() });
		expect(screen.getByRole('button', { name: 'Action A' })).toHaveAttribute('aria-busy', 'true');
		expect(screen.getByRole('button', { name: 'Action B' })).not.toHaveAttribute('aria-busy');
	});

	it('onaction fires with the correct id on click', () => {
		const onaction = vi.fn();
		const actions: Action[] = [{ id: 'my-action', label: 'Run' }];
		render(BatchActionBar, { selectedCount: 1, actions, onaction, oncancel: vi.fn() });
		fireEvent.click(screen.getByRole('button', { name: 'Run' }));
		expect(onaction).toHaveBeenCalledWith('my-action');
	});

	it('Deselect all button renders variant="secondary" size="sm"', () => {
		render(BatchActionBar, {
			selectedCount: 2,
			actions: [{ id: 'x', label: 'X' }],
			onaction: vi.fn(),
			oncancel: vi.fn()
		});
		const btn = screen.getByRole('button', { name: 'Deselect all' });
		expect(btn.className).toContain('border');
	});
});
