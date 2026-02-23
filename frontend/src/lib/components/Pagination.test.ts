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

	it('displays the current page and total pages', () => {
		render(Pagination, { currentPage: 2, totalPages: 7, onPageChange: vi.fn() });
		expect(screen.getByText(/page 2 of 7/i)).toBeInTheDocument();
	});
});
