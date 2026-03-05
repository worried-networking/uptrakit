import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import Modal from './Modal.svelte';

function makeChildren(html = '<p>Modal content</p>') {
	return createRawSnippet(() => ({
		render() {
			return html;
		}
	}));
}

function makeFooter(html: string) {
	return createRawSnippet(() => ({
		render() {
			return html;
		}
	}));
}

afterEach(() => {
	cleanup();
	vi.clearAllMocks();
});

describe('Modal', () => {
	it('renders children inside a role="dialog" element with background classes', () => {
		render(Modal, { onclose: vi.fn(), children: makeChildren() });
		const dialog = screen.getByRole('dialog');
		expect(dialog).toBeInTheDocument();
		expect(dialog.className).toContain('bg-surface-50');
		expect(dialog.className).toContain('dark:bg-surface-900');
	});

	it('renders the title as <h3> when provided', () => {
		render(Modal, { onclose: vi.fn(), title: 'Test Title', children: makeChildren() });
		const heading = screen.getByRole('heading', { level: 3 });
		expect(heading).toBeInTheDocument();
		expect(heading.textContent).toBe('Test Title');
	});

	it('omits the <h3> when title is not provided', () => {
		render(Modal, { onclose: vi.fn(), children: makeChildren() });
		expect(screen.queryByRole('heading', { level: 3 })).not.toBeInTheDocument();
	});

	it('renders footer content when footer snippet is provided', () => {
		render(Modal, {
			onclose: vi.fn(),
			children: makeChildren(),
			footer: makeFooter('<button>OK</button>')
		});
		expect(screen.getByRole('button', { name: 'OK' })).toBeInTheDocument();
	});

	it('omits footer wrapper when footer snippet is not provided', () => {
		const { container } = render(Modal, { onclose: vi.fn(), children: makeChildren() });
		const dialog = container.querySelector('[role="dialog"]')!;
		expect(dialog.querySelector('.justify-end')).not.toBeInTheDocument();
	});

	it('calls onclose when Escape key is pressed', () => {
		const onclose = vi.fn();
		const { container } = render(Modal, { onclose, children: makeChildren() });
		const backdrop = container.firstElementChild as HTMLElement;
		fireEvent.keyDown(backdrop, { key: 'Escape' });
		expect(onclose).toHaveBeenCalledOnce();
	});
});
