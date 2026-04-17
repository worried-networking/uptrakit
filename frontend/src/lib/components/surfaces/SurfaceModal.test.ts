import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import SurfaceModal from './SurfaceModal.svelte';

function makeChildren(html = '<p>Modal content</p>') {
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

describe('SurfaceModal', () => {
	it('reuses the shared modal shell when open', () => {
		const { container } = render(SurfaceModal, {
			open: true,
			title: 'Surface details',
			onclose: vi.fn(),
			children: makeChildren()
		});

		expect(screen.getByRole('dialog')).toBeInTheDocument();
		expect(screen.getByRole('heading', { name: 'Surface details' })).toBeInTheDocument();
		expect(screen.getByText('Modal content')).toBeInTheDocument();
		expect(container.querySelector('[data-ui="modal-shell"]')).toBeInTheDocument();
	});

	it('does not render modal chrome when closed', () => {
		render(SurfaceModal, {
			open: false,
			title: 'Surface details',
			onclose: vi.fn(),
			children: makeChildren()
		});

		expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
	});
});
