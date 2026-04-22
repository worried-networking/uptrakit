import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import BatchResultDialog from './BatchResultDialog.svelte';
import type { BatchActionResponse } from '$lib/types';

afterEach(() => {
	cleanup();
	vi.clearAllMocks();
});

const makeResponse = (succeededIds: string[], failed: { id: string; error: string }[]): BatchActionResponse => ({
	succeeded: succeededIds.map((id) => ({ id })),
	failed
});

describe('BatchResultDialog', () => {
	it('Close button renders variant="primary"', () => {
		render(BatchResultDialog, { title: 'Results', response: makeResponse(['id-1'], []), onclose: vi.fn() });
		const closeBtn = screen.getByRole('button', { name: 'Close' });
		expect(closeBtn.className).toMatch(/bg-\[linear-gradient/);
	});

	it('calls onclose when Close is clicked', () => {
		const onclose = vi.fn();
		render(BatchResultDialog, { title: 'Results', response: makeResponse(['id-1'], []), onclose });
		fireEvent.click(screen.getByRole('button', { name: 'Close' }));
		expect(onclose).toHaveBeenCalledOnce();
	});
});
