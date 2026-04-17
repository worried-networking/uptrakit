import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/svelte';
import Pagination from './Pagination.svelte';

describe('Pagination', () => {
	it('renders nothing when totalPages is 1', () => {
		render(Pagination, { currentPage: 1, totalPages: 1, onPageChange: vi.fn() });
		expect(screen.queryByRole('navigation')).not.toBeInTheDocument();
	});

	it('renders navigation when totalPages is greater than 1', () => {
		render(Pagination, { currentPage: 2, totalPages: 5, onPageChange: vi.fn() });
		expect(screen.getByRole('navigation', { name: /pagination/i })).toBeInTheDocument();
		expect(screen.getByRole('navigation', { name: /pagination/i }).className).not.toContain('mt-4');
	});

	it('disables the Previous button on the first page', () => {
		render(Pagination, { currentPage: 1, totalPages: 3, onPageChange: vi.fn() });
		expect(screen.getByRole('button', { name: /previous/i })).toBeDisabled();
		expect(screen.getByRole('button', { name: /next/i })).not.toBeDisabled();
	});

	it('disables the Next button on the last page', () => {
		render(Pagination, { currentPage: 3, totalPages: 3, onPageChange: vi.fn() });
		expect(screen.getByRole('button', { name: /next/i })).toBeDisabled();
		expect(screen.getByRole('button', { name: /previous/i })).not.toBeDisabled();
	});

	it('calls onPageChange with the previous page when Previous is clicked', () => {
		const onPageChange = vi.fn();
		render(Pagination, { currentPage: 3, totalPages: 5, onPageChange });
		fireEvent.click(screen.getByRole('button', { name: /previous/i }));
		expect(onPageChange).toHaveBeenCalledWith(2);
	});

	it('calls onPageChange with the next page when Next is clicked', () => {
		const onPageChange = vi.fn();
		render(Pagination, { currentPage: 3, totalPages: 5, onPageChange });
		fireEvent.click(screen.getByRole('button', { name: /next/i }));
		expect(onPageChange).toHaveBeenCalledWith(4);
	});

	it('highlights the current page button', () => {
		render(Pagination, { currentPage: 2, totalPages: 5, onPageChange: vi.fn() });
		const currentBtn = screen.getByRole('button', { name: '2' });
		expect(currentBtn).toHaveAttribute('aria-current', 'page');
		expect(currentBtn.className).toContain('preset-filled-primary-500');
	});

	it('renders all page buttons for small page counts', () => {
		render(Pagination, { currentPage: 3, totalPages: 5, onPageChange: vi.fn() });
		for (let i = 1; i <= 5; i++) {
			expect(screen.getByRole('button', { name: String(i) })).toBeInTheDocument();
		}
	});

	it('calls onPageChange when a page number button is clicked', () => {
		const onPageChange = vi.fn();
		render(Pagination, { currentPage: 1, totalPages: 5, onPageChange });
		fireEvent.click(screen.getByRole('button', { name: '4' }));
		expect(onPageChange).toHaveBeenCalledWith(4);
	});

	it('shows ellipsis for large page counts', () => {
		const { container } = render(Pagination, { currentPage: 10, totalPages: 20, onPageChange: vi.fn() });
		// Should have ellipsis elements (rendered as "…")
		const ellipses = Array.from(container.querySelectorAll('span')).filter((element) =>
			element.textContent?.includes('…')
		);
		expect(ellipses.length).toBeGreaterThanOrEqual(1);
	});

	it('does not display a total count label inline', () => {
		render(Pagination, { currentPage: 1, totalPages: 3, onPageChange: vi.fn() });
		expect(screen.queryByText(/total/i)).not.toBeInTheDocument();
	});
});
