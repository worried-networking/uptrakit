import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import ContextMenu from './ContextMenu.svelte';

// Build a reusable children snippet with three labelled menu items.
function makeItemsSnippet(labels: string[] = ['Item 1', 'Item 2', 'Item 3']) {
	return createRawSnippet(() => ({
		render() {
			return `<ul>${labels.map((l) => `<li><button role="menuitem">${l}</button></li>`).join('')}</ul>`;
		}
	}));
}

afterEach(() => {
	cleanup();
	vi.clearAllMocks();
});

describe('ContextMenu', () => {
	it('renders the children inside a menu element', () => {
		render(ContextMenu, {
			top: 100,
			left: 100,
			onclose: vi.fn(),
			children: makeItemsSnippet()
		});
		expect(screen.getByRole('menu')).toBeInTheDocument();
		expect(screen.getAllByRole('menuitem')).toHaveLength(3);
	});

	it('calls onclose when Escape is pressed', () => {
		const onclose = vi.fn();
		render(ContextMenu, {
			top: 100,
			left: 100,
			onclose,
			children: makeItemsSnippet()
		});
		fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' });
		expect(onclose).toHaveBeenCalledOnce();
	});

	it('moves focus to the next item on ArrowDown', async () => {
		render(ContextMenu, {
			top: 100,
			left: 100,
			onclose: vi.fn(),
			children: makeItemsSnippet()
		});
		const menu = screen.getByRole('menu');
		const items = screen.getAllByRole('menuitem');
		// Focus the first item manually (simulating the onMount auto-focus)
		items[0].focus();
		fireEvent.keyDown(menu, { key: 'ArrowDown' });
		expect(items[1]).toHaveFocus();
	});

	it('wraps focus back to the first item when ArrowDown is pressed on the last item', async () => {
		render(ContextMenu, {
			top: 100,
			left: 100,
			onclose: vi.fn(),
			children: makeItemsSnippet()
		});
		const menu = screen.getByRole('menu');
		const items = screen.getAllByRole('menuitem');
		// Navigate via keyboard to the last item so the component's internal
		// focusedIndex stays in sync (direct focus() would leave it at 0).
		for (let i = 0; i < items.length - 1; i++) {
			fireEvent.keyDown(menu, { key: 'ArrowDown' });
		}
		expect(items[items.length - 1]).toHaveFocus();
		fireEvent.keyDown(menu, { key: 'ArrowDown' });
		expect(items[0]).toHaveFocus();
	});

	it('moves focus to the previous item on ArrowUp', async () => {
		render(ContextMenu, {
			top: 100,
			left: 100,
			onclose: vi.fn(),
			children: makeItemsSnippet()
		});
		const menu = screen.getByRole('menu');
		const items = screen.getAllByRole('menuitem');
		// Navigate to item 2 (index 1) via keyboard to keep focusedIndex in sync.
		fireEvent.keyDown(menu, { key: 'ArrowDown' });
		expect(items[1]).toHaveFocus();
		fireEvent.keyDown(menu, { key: 'ArrowUp' });
		expect(items[0]).toHaveFocus();
	});

	it('focuses the first item on Home', async () => {
		render(ContextMenu, {
			top: 100,
			left: 100,
			onclose: vi.fn(),
			children: makeItemsSnippet()
		});
		const menu = screen.getByRole('menu');
		const items = screen.getAllByRole('menuitem');
		items[2].focus();
		fireEvent.keyDown(menu, { key: 'Home' });
		expect(items[0]).toHaveFocus();
	});

	it('focuses the last item on End', async () => {
		render(ContextMenu, {
			top: 100,
			left: 100,
			onclose: vi.fn(),
			children: makeItemsSnippet()
		});
		const menu = screen.getByRole('menu');
		const items = screen.getAllByRole('menuitem');
		items[0].focus();
		fireEvent.keyDown(menu, { key: 'End' });
		expect(items[items.length - 1]).toHaveFocus();
	});

	it('triggers the focused item click handler when Enter is pressed', async () => {
		const onAction = vi.fn();
		const children = createRawSnippet(() => ({
			render() {
				return `<ul><li><button role="menuitem" id="action-btn">Action</button></li></ul>`;
			}
		}));
		render(ContextMenu, { top: 100, left: 100, onclose: vi.fn(), children });
		const menu = screen.getByRole('menu');
		const btn = screen.getByRole('menuitem');
		// Attach the listener after render so we can assert it fires
		btn.addEventListener('click', onAction);
		btn.focus();
		fireEvent.keyDown(menu, { key: 'Enter' });
		expect(onAction).toHaveBeenCalledOnce();
	});
});
