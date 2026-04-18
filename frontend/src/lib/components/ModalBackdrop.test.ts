import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import ModalBackdrop from './ModalBackdrop.svelte';

function makeContent(innerHtml = '<button>Action</button>') {
	return createRawSnippet(() => ({
		render() {
			return `<div role="dialog" aria-modal="true">${innerHtml}</div>`;
		}
	}));
}

afterEach(() => {
	cleanup();
	vi.clearAllMocks();
});

describe('ModalBackdrop', () => {
	it('renders the child content inside the backdrop', () => {
		const { container } = render(ModalBackdrop, { onclose: vi.fn(), children: makeContent() });
		expect(screen.getByRole('dialog')).toBeInTheDocument();
		const backdrop = container.firstElementChild as HTMLElement;
		expect(backdrop).toHaveAttribute('data-ui', 'modal-backdrop');
		expect(backdrop.className).toContain('z-[900]');
	});

	it('calls onclose when the Escape key is pressed', () => {
		const onclose = vi.fn();
		const { container } = render(ModalBackdrop, { onclose, children: makeContent() });
		const backdrop = container.firstElementChild as HTMLElement;
		fireEvent.keyDown(backdrop, { key: 'Escape' });
		expect(onclose).toHaveBeenCalledOnce();
	});

	it('calls onclose when the backdrop overlay is clicked directly', () => {
		const onclose = vi.fn();
		const { container } = render(ModalBackdrop, { onclose, children: makeContent() });
		// The inner flex-wrapper fills the viewport and holds the onclick handler.
		// Clicking it directly (not a child element) simulates a click on the dark backdrop area.
		const innerWrapper = container.firstElementChild!.firstElementChild as HTMLElement;
		fireEvent.click(innerWrapper);
		expect(onclose).toHaveBeenCalledOnce();
	});

	it('does not call onclose when a child element inside the backdrop is clicked', () => {
		const onclose = vi.fn();
		render(ModalBackdrop, { onclose, children: makeContent() });
		const btn = screen.getByRole('button', { name: 'Action' });
		fireEvent.click(btn);
		expect(onclose).not.toHaveBeenCalled();
	});

	it('traps Tab focus within the modal content', () => {
		// Render with two focusable elements so we can verify the wrap-around.
		const children = createRawSnippet(() => ({
			render() {
				return `<div role="dialog">
					<button id="first">First</button>
					<button id="last">Last</button>
				</div>`;
			}
		}));
		const { container } = render(ModalBackdrop, { onclose: vi.fn(), children });
		const backdrop = container.firstElementChild as HTMLElement;
		const last = screen.getByRole('button', { name: 'Last' });
		// Simulate Tab pressed while the last element is focused → wraps to first
		last.focus();
		fireEvent.keyDown(backdrop, { key: 'Tab', shiftKey: false });
		expect(screen.getByRole('button', { name: 'First' })).toHaveFocus();
	});

	it('traps Shift+Tab focus within the modal content', () => {
		const children = createRawSnippet(() => ({
			render() {
				return `<div role="dialog">
					<button id="first">First</button>
					<button id="last">Last</button>
				</div>`;
			}
		}));
		const { container } = render(ModalBackdrop, { onclose: vi.fn(), children });
		const backdrop = container.firstElementChild as HTMLElement;
		const first = screen.getByRole('button', { name: 'First' });
		// Simulate Shift+Tab pressed while the first element is focused → wraps to last
		first.focus();
		fireEvent.keyDown(backdrop, { key: 'Tab', shiftKey: true });
		expect(screen.getByRole('button', { name: 'Last' })).toHaveFocus();
	});
});
