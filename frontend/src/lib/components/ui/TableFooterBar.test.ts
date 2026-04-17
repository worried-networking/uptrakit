import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import TableFooterBar from './TableFooterBar.svelte';

afterEach(() => {
	cleanup();
});

describe('TableFooterBar', () => {
	it('renders totals and pagination inside the shared footer row', () => {
		const { container } = render(TableFooterBar, {
			total: 42,
			currentPage: 2,
			totalPages: 4,
			onPageChange: vi.fn()
		});

		expect(screen.getByText('42 total')).toBeInTheDocument();
		expect(screen.getByRole('navigation', { name: /pagination/i })).toBeInTheDocument();
		expect(container.querySelector('[data-ui="table-footer-bar"]')).toHaveClass('items-center', 'justify-between');
	});
});
