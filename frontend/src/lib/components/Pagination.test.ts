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
		expect(currentBtn.className).toContain('bg-[var(--accent)]');
		expect(currentBtn.className).toContain('text-[var(--text-inverted)]');
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

	it('Previous and Next buttons carry h-8 height override class', () => {
		render(Pagination, { currentPage: 2, totalPages: 5, onPageChange: vi.fn() });
		expect(screen.getByRole('button', { name: /previous/i }).className).toContain('h-8');
		expect(screen.getByRole('button', { name: /next/i }).className).toContain('h-8');
	});

	it('page-number buttons carry h-8 height override class', () => {
		render(Pagination, { currentPage: 2, totalPages: 5, onPageChange: vi.fn() });
		expect(screen.getByRole('button', { name: '3' }).className).toContain('h-8');
	});

	it('inactive page-number buttons do not carry active accent/bg-hover classes', () => {
		render(Pagination, { currentPage: 2, totalPages: 5, onPageChange: vi.fn() });
		const inactiveBtn = screen.getByRole('button', { name: '3' });
		expect(inactiveBtn.className).not.toContain('text-[var(--accent)]');
		expect(inactiveBtn.className).not.toContain('bg-[var(--bg-hover)]');
	});

	it('Previous button has a leadingIcon SVG in the DOM', () => {
		render(Pagination, { currentPage: 2, totalPages: 5, onPageChange: vi.fn() });
		const prevBtn = screen.getByRole('button', { name: /previous/i });
		expect(prevBtn.querySelector('svg')).not.toBeNull();
	});

	it('Next button has a trailingIcon SVG in the DOM', () => {
		render(Pagination, { currentPage: 2, totalPages: 5, onPageChange: vi.fn() });
		const nextBtn = screen.getByRole('button', { name: /next/i });
		expect(nextBtn.querySelector('svg')).not.toBeNull();
	});
});
