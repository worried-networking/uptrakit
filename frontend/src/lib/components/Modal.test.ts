import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import Modal from './Modal.svelte';
import ModalWithFormFieldFixture from '$lib/test-mocks/modal-with-form-field-fixture.svelte';

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
		const { container } = render(Modal, { onclose: vi.fn(), children: makeChildren() });
		const dialog = screen.getByRole('dialog');
		expect(dialog).toBeInTheDocument();
		expect(dialog.className).toContain('bg-[var(--bg-surface)]');
		expect(dialog.className).toContain('max-w-2xl');
		expect(dialog.className).toContain('z-[910]');
		expect(dialog).toHaveAttribute('data-ui', 'modal-shell');
		expect(document.body.querySelector('[data-ui="modal-shell"]')).toBe(dialog);
		// Portal contract: backdrop and shell live directly under <body>, not inside the test wrapper.
		expect(container.querySelector('[data-ui="modal-shell"]')).toBeNull();
		expect(container.querySelector('[data-ui="modal-backdrop"]')).toBeNull();
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
		render(Modal, { onclose: vi.fn(), children: makeChildren() });
		const dialog = document.body.querySelector('[role="dialog"]')!;
		expect(dialog.querySelector('.justify-end')).not.toBeInTheDocument();
	});

	it('calls onclose when Escape key is pressed', () => {
		const onclose = vi.fn();
		render(Modal, { onclose, children: makeChildren() });
		const backdrop = document.body.querySelector('[data-ui="modal-backdrop"]') as HTMLElement;
		fireEvent.keyDown(backdrop, { key: 'Escape' });
		expect(onclose).toHaveBeenCalledOnce();
	});

	it('portals the backdrop to document.body so it covers the viewport from any ancestor containing block', () => {
		render(Modal, { onclose: vi.fn(), children: makeChildren() });
		const backdrop = document.body.querySelector('[data-ui="modal-backdrop"]');
		expect(backdrop).not.toBeNull();
		expect(backdrop?.parentElement).toBe(document.body);
	});

	it('stacks a second modal above the first by DOM order under <body>', () => {
		// Real call sites (e.g. ConfirmDialog opened from inside Modal in
		// PluginConfigsTab, EditHostAssignmentModal, SoftwareMergeWizard) rely on
		// "render order = paint order" once both backdrops live under <body>.
		render(Modal, { onclose: vi.fn(), title: 'First', children: makeChildren('<p>first</p>') });
		render(Modal, { onclose: vi.fn(), title: 'Second', children: makeChildren('<p>second</p>') });

		const backdrops = Array.from(document.body.querySelectorAll('[data-ui="modal-backdrop"]'));
		expect(backdrops).toHaveLength(2);

		const firstIndex = Array.from(document.body.children).indexOf(backdrops[0] as Element);
		const secondIndex = Array.from(document.body.children).indexOf(backdrops[1] as Element);
		expect(secondIndex).toBeGreaterThan(firstIndex);
	});

	it('preserves Svelte form-layout context across portaling so FormFieldRow uses modal grid', () => {
		render(ModalWithFormFieldFixture, { onclose: vi.fn() });
		const fieldRow = document.body.querySelector('[data-ui="form-field-row"]');
		expect(fieldRow).not.toBeNull();
		// FormFieldRow reads getFormLayout() from Svelte context. Modal sets
		// FormLayout.Modal which maps to the narrow-label grid. If portaling
		// disrupted context propagation (it shouldn't — context is component-tree,
		// not DOM-tree) the wide page-layout grid would appear instead.
		expect(fieldRow?.className).toContain('md:grid-cols-[minmax(0,11rem)_minmax(0,1fr)]');
	});
});
