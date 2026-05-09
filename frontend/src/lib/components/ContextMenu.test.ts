import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import ContextMenu from './ContextMenu.svelte';
import { computeMenuPosition } from './context-menu-position';

// Build a reusable children snippet with three labelled menu items.
function makeItemsSnippet(labels: string[] = ['Item 1', 'Item 2', 'Item 3']) {
	return createRawSnippet(() => ({
		render() {
			return `<ul>${labels.map((l) => `<li><button role="menuitem">${l}</button></li>`).join('')}</ul>`;
		}
	}));
}

const ANCHOR = new DOMRect(100, 100, 32, 32);

afterEach(() => {
	cleanup();
	vi.clearAllMocks();
});

describe('ContextMenu', () => {
	it('renders the children inside a menu element', () => {
		const { container } = render(ContextMenu, {
			anchorRect: ANCHOR,
			onclose: vi.fn(),
			children: makeItemsSnippet()
		});
		expect(screen.getByRole('menu')).toBeInTheDocument();
		expect(screen.getAllByRole('menuitem')).toHaveLength(3);
		expect(document.body.querySelector('[data-ui="context-menu-shell"]')).toBe(screen.getByRole('menu'));
		// Portal contract: the menu lives directly under <body>, not inside the test wrapper.
		expect(container.querySelector('[data-ui="context-menu-shell"]')).toBeNull();
		expect(screen.getByRole('menu')).toHaveAttribute('data-ui', 'context-menu-shell');
		expect(screen.getByRole('menu').className).toContain('border-[var(--border-default)]');
		expect(screen.getByRole('menu').className).toContain('rounded-panel');
		expect(screen.getByRole('menu').className).toContain('z-[100]');
	});

	it('calls onclose when Escape is pressed', () => {
		const onclose = vi.fn();
		render(ContextMenu, {
			anchorRect: ANCHOR,
			onclose,
			children: makeItemsSnippet()
		});
		fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' });
		expect(onclose).toHaveBeenCalledOnce();
	});

	it('moves focus to the next item on ArrowDown', async () => {
		render(ContextMenu, {
			anchorRect: ANCHOR,
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
			anchorRect: ANCHOR,
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
			anchorRect: ANCHOR,
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
			anchorRect: ANCHOR,
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
			anchorRect: ANCHOR,
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
		render(ContextMenu, { anchorRect: ANCHOR, onclose: vi.fn(), children });
		const menu = screen.getByRole('menu');
		const btn = screen.getByRole('menuitem');
		// Attach the listener after render so we can assert it fires
		btn.addEventListener('click', onAction);
		btn.focus();
		fireEvent.keyDown(menu, { key: 'Enter' });
		expect(onAction).toHaveBeenCalledOnce();
	});

	it('portals the menu shell to document.body so it escapes ancestor containing blocks', () => {
		render(ContextMenu, {
			anchorRect: ANCHOR,
			onclose: vi.fn(),
			children: makeItemsSnippet()
		});
		expect(screen.getByRole('menu').parentElement).toBe(document.body);
	});

	it('warns in dev when opened over a modal backdrop', () => {
		const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
		const backdrop = document.createElement('div');
		backdrop.setAttribute('data-ui', 'modal-backdrop');
		document.body.appendChild(backdrop);

		try {
			render(ContextMenu, {
				anchorRect: ANCHOR,
				onclose: vi.fn(),
				children: makeItemsSnippet()
			});
			expect(warn).toHaveBeenCalledTimes(1);
			expect(warn.mock.calls[0][0]).toMatch(/ContextMenu opened over a modal/);
		} finally {
			backdrop.remove();
			warn.mockRestore();
		}
	});

	it('activates the currently focused menu item when focus changes directly', async () => {
		const firstAction = vi.fn();
		const secondAction = vi.fn();
		const children = createRawSnippet(() => ({
			render() {
				return `<ul><li><button role="menuitem" id="first-action">First</button></li><li><button role="menuitem" id="second-action">Second</button></li></ul>`;
			}
		}));
		render(ContextMenu, { anchorRect: ANCHOR, onclose: vi.fn(), children });
		const menu = screen.getByRole('menu');
		const [firstButton, secondButton] = screen.getAllByRole('menuitem');
		firstButton.addEventListener('click', firstAction);
		secondButton.addEventListener('click', secondAction);

		secondButton.focus();
		fireEvent.keyDown(menu, { key: 'Enter' });

		expect(secondAction).toHaveBeenCalledOnce();
		expect(firstAction).not.toHaveBeenCalled();
	});

	describe('positioning integration', () => {
		const VIEWPORT = { vw: 1280, vh: 800 };
		const MENU_SIZE = { width: 176, height: 240 };

		const originalGBCR = HTMLDivElement.prototype.getBoundingClientRect;
		const originalInnerWidth = window.innerWidth;
		const originalInnerHeight = window.innerHeight;

		// Mock getBoundingClientRect on the menu's HTMLDivElement so the positioning
		// effect sees realistic dimensions in jsdom (which otherwise reports 0×0).
		function mockMenuSize(size: { width: number; height: number }) {
			HTMLDivElement.prototype.getBoundingClientRect = function (this: HTMLDivElement) {
				if (this.dataset.ui === 'context-menu-shell') {
					return new DOMRect(0, 0, size.width, size.height);
				}
				return originalGBCR.call(this);
			} as never;
		}

		function setViewport(vp: { vw: number; vh: number }) {
			Object.defineProperty(window, 'innerWidth', { value: vp.vw, configurable: true, writable: true });
			Object.defineProperty(window, 'innerHeight', { value: vp.vh, configurable: true, writable: true });
		}

		afterEach(() => {
			HTMLDivElement.prototype.getBoundingClientRect = originalGBCR;
			Object.defineProperty(window, 'innerWidth', { value: originalInnerWidth, configurable: true, writable: true });
			Object.defineProperty(window, 'innerHeight', {
				value: originalInnerHeight,
				configurable: true,
				writable: true
			});
		});

		it('applies computeMenuPosition output as inline top/left styles', () => {
			mockMenuSize(MENU_SIZE);
			setViewport(VIEWPORT);
			const anchor = new DOMRect(200, 200, 32, 32);
			render(ContextMenu, { anchorRect: anchor, onclose: vi.fn(), children: makeItemsSnippet() });

			const expected = computeMenuPosition(anchor, MENU_SIZE, VIEWPORT);
			const menu = screen.getByRole('menu');
			expect(menu.style.top).toBe(`${expected.top}px`);
			expect(menu.style.left).toBe(`${expected.left}px`);
		});

		it('flips horizontally and clamps vertically for a bottom-right corner anchor', () => {
			mockMenuSize({ width: 200, height: 300 });
			setViewport(VIEWPORT);
			const anchor = new DOMRect(VIEWPORT.vw - 50, VIEWPORT.vh - 50, 32, 32);
			render(ContextMenu, { anchorRect: anchor, onclose: vi.fn(), children: makeItemsSnippet() });

			const expected = computeMenuPosition(anchor, { width: 200, height: 300 }, VIEWPORT);
			const menu = screen.getByRole('menu');
			expect(menu.style.top).toBe(`${expected.top}px`);
			expect(menu.style.left).toBe(`${expected.left}px`);
			// Sanity: flipped horizontally (with 2px gap) and clamped to vh - pad - height.
			expect(expected.left).toBe(anchor.left - 2 - 200);
			expect(expected.top).toBe(VIEWPORT.vh - 8 - 300);
		});

		it('closes on window resize so stale anchor rects do not mis-position', () => {
			const onclose = vi.fn();
			render(ContextMenu, { anchorRect: ANCHOR, onclose, children: makeItemsSnippet() });
			window.dispatchEvent(new Event('resize'));
			expect(onclose).toHaveBeenCalledOnce();
		});
	});
});
