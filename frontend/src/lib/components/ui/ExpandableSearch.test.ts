import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import ExpandableSearch from './ExpandableSearch.svelte';

afterEach(() => {
	cleanup();
	vi.useRealTimers();
});

describe('ExpandableSearch', () => {
	it('starts collapsed when value is empty', () => {
		render(ExpandableSearch, {
			id: 'test-search',
			value: '',
			onchange: vi.fn(),
			placeholder: 'Search...'
		});
		expect(screen.queryByRole('searchbox')).not.toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Search...' })).toBeInTheDocument();
	});

	it('starts expanded when value is non-empty', () => {
		render(ExpandableSearch, {
			id: 'test-search',
			value: 'nginx',
			onchange: vi.fn(),
			placeholder: 'Search...'
		});
		const input = screen.getByRole('searchbox') as HTMLInputElement;
		expect(input.value).toBe('nginx');
	});

	it('clicking the search icon button expands to show input', async () => {
		render(ExpandableSearch, {
			id: 'test-search',
			value: '',
			onchange: vi.fn(),
			placeholder: 'Search...'
		});
		await fireEvent.click(screen.getByRole('button', { name: 'Search...' }));
		await waitFor(() => expect(screen.getByRole('searchbox')).toBeInTheDocument());
	});

	it('calls onchange after debounce when typing', async () => {
		vi.useFakeTimers();
		const onchange = vi.fn();
		render(ExpandableSearch, { id: 'test-search', value: '', onchange, debounceMs: 100 });
		await fireEvent.click(screen.getByRole('button', { name: 'Search...' }));
		await waitFor(() => expect(screen.getByRole('searchbox')).toBeInTheDocument());
		await fireEvent.input(screen.getByRole('searchbox'), { target: { value: 'nginx' } });
		expect(onchange).not.toHaveBeenCalled();
		vi.advanceTimersByTime(100);
		expect(onchange).toHaveBeenCalledWith('nginx');
	});

	it('pressing Escape clears value and collapses', async () => {
		const onchange = vi.fn();
		render(ExpandableSearch, { id: 'test-search', value: 'nginx', onchange });
		const input = screen.getByRole('searchbox');
		await fireEvent.keyDown(input, { key: 'Escape' });
		expect(onchange).toHaveBeenCalledWith('');
		await waitFor(() => expect(screen.queryByRole('searchbox')).not.toBeInTheDocument());
	});

	it('clicking clear button calls onchange with empty string and collapses', async () => {
		const onchange = vi.fn();
		render(ExpandableSearch, { id: 'test-search', value: 'nginx', onchange });
		const clearBtn = screen.getByRole('button', { name: 'Clear search' });
		await fireEvent.click(clearBtn);
		expect(onchange).toHaveBeenCalledWith('');
		await waitFor(() => expect(screen.queryByRole('searchbox')).not.toBeInTheDocument());
	});

	it('external value prop change syncs local state', async () => {
		const { rerender } = render(ExpandableSearch, {
			id: 'test-search',
			value: '',
			onchange: vi.fn()
		});
		await rerender({ value: 'updated' });
		await waitFor(() => {
			const input = screen.getByRole('searchbox') as HTMLInputElement;
			expect(input.value).toBe('updated');
		});
	});
});
