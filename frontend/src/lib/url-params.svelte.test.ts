import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import { page } from '$app/state';
import { goto } from '$app/navigation';

vi.mock('$app/navigation', () => ({ goto: vi.fn() }));

function setUrl(url: string) {
	const parsed = new URL(url);
	Object.defineProperty(page, 'url', { value: parsed, configurable: true });
}

import UrlParamHarness from './url-params.test-harness.svelte';

describe('createUrlParam', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		setUrl('http://localhost/');
	});

	it('returns empty string when param absent', () => {
		setUrl('http://localhost/software');
		render(UrlParamHarness, { paramKey: 'query' });
		expect(screen.getByTestId('current-value').textContent).toBe('""');
	});

	it('returns param value when present', () => {
		setUrl('http://localhost/software?query=nginx');
		render(UrlParamHarness, { paramKey: 'query' });
		expect(screen.getByTestId('current-value').textContent).toBe('"nginx"');
	});

	it('set() calls goto() with updated URL', async () => {
		setUrl('http://localhost/software');
		const { rerender } = render(UrlParamHarness, { paramKey: 'query', testSetValue: '' });
		await rerender({ paramKey: 'query', testSetValue: 'nginx' });
		await fireEvent.click(screen.getByTestId('do-set'));
		expect(vi.mocked(goto)).toHaveBeenCalledWith(
			expect.objectContaining({ searchParams: expect.any(URLSearchParams) }),
			{ replaceState: true, keepFocus: true, noScroll: true }
		);
		const calledUrl: URL = vi.mocked(goto).mock.calls[0][0] as URL;
		expect(calledUrl.searchParams.get('query')).toBe('nginx');
	});

	it('set() removes page= from URL', async () => {
		setUrl('http://localhost/software?page=3');
		const { rerender } = render(UrlParamHarness, { paramKey: 'query', testSetValue: '' });
		await rerender({ paramKey: 'query', testSetValue: 'nginx' });
		await fireEvent.click(screen.getByTestId('do-set'));
		const calledUrl: URL = vi.mocked(goto).mock.calls[0][0] as URL;
		expect(calledUrl.searchParams.has('page')).toBe(false);
	});

	it('enum param falls back to default for unknown value', () => {
		setUrl('http://localhost/?status=unknown');
		render(UrlParamHarness, {
			paramKey: 'status',
			parse: (r: string | null) => (r === 'pending' || r === 'completed' ? r : 'all')
		});
		expect(screen.getByTestId('current-value').textContent).toBe('"all"');
	});
});
