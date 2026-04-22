import { afterEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import type { MergeSoftwareItemSummary } from '$lib/types';

vi.mock('$lib/notifications.svelte', () => ({
	showError: vi.fn(),
	showSuccess: vi.fn()
}));

import SoftwareMergeWizard from './SoftwareMergeWizard.svelte';
import * as notifications from '$lib/notifications.svelte';

const candidateA: MergeSoftwareItemSummary = { id: 'a', name: 'Firefox', host_count: 3, plugins: ['apt'] };
const candidateB: MergeSoftwareItemSummary = { id: 'b', name: 'Firefox ESR', host_count: 1, plugins: ['apt'] };

function makeProps(overrides: Record<string, unknown> = {}) {
	return {
		candidates: [candidateA, candidateB],
		seedItemId: 'a',
		searchCandidates: null,
		initialSearchQuery: '',
		onclose: vi.fn(),
		onsuccess: vi.fn(),
		previewMerge: vi.fn().mockResolvedValue({
			candidate_count: 2,
			moved_link_count: 1,
			skipped_duplicate_link_count: 0,
			survivor: { id: 'a', name: 'Firefox', host_count: 3 },
			losers: [{ id: 'b', name: 'Firefox ESR', host_count: 1 }],
			moved_links: [],
			skipped_duplicate_links: []
		}),
		executeMerge: vi.fn().mockResolvedValue({ merged_item_id: 'a' }),
		...overrides
	};
}

describe('SoftwareMergeWizard Button primitive contracts', () => {
	afterEach(() => {
		vi.clearAllMocks();
	});

	it('step-1 Next button renders primary variant', async () => {
		render(SoftwareMergeWizard, makeProps());
		const nextBtn = screen.getByRole('button', { name: 'Next' });
		expect(nextBtn.className).toContain('bg-[linear-gradient');
		expect(nextBtn).not.toHaveAttribute('aria-busy');
	});

	it('step-2 Back button renders secondary variant', async () => {
		render(SoftwareMergeWizard, makeProps());
		await fireEvent.click(screen.getByRole('button', { name: 'Next' }));
		const backBtn = await screen.findByRole('button', { name: 'Back' });
		expect(backBtn.className).toContain('var(--bg-raised)'); // secondary
	});

	it('step-2 Merge button renders primary variant', async () => {
		render(SoftwareMergeWizard, makeProps());
		await fireEvent.click(screen.getByRole('button', { name: 'Next' }));
		const mergeBtn = await screen.findByRole('button', { name: 'Merge' });
		expect(mergeBtn.className).toContain('bg-[linear-gradient');
	});

	it('Cancel button remains enabled during merge submit', async () => {
		const executeMerge = vi.fn().mockReturnValue(new Promise(() => {}));
		render(SoftwareMergeWizard, makeProps({ executeMerge }));

		await fireEvent.click(screen.getByRole('button', { name: 'Next' }));
		const mergeBtn = await screen.findByRole('button', { name: 'Merge' });
		await fireEvent.click(mergeBtn);

		// Confirm in the ConfirmDialog that surfaces after clicking Merge
		const confirmMergeBtn = await screen.findByRole('button', { name: 'Merge Items' });
		await fireEvent.click(confirmMergeBtn);

		await waitFor(() => expect(mergeBtn).toHaveAttribute('aria-busy', 'true'));

		const cancelBtn = screen.getByRole('button', { name: 'Cancel' });
		expect(cancelBtn).not.toBeDisabled();
		expect(cancelBtn).not.toHaveAttribute('aria-busy');
	});

	it('Next loading resets to false on preview error, showError called', async () => {
		const previewMerge = vi.fn().mockRejectedValue(new Error('Preview failed'));
		render(SoftwareMergeWizard, makeProps({ previewMerge }));

		await fireEvent.click(screen.getByRole('button', { name: 'Next' }));
		await waitFor(() => expect(vi.mocked(notifications.showError)).toHaveBeenCalledWith('Preview failed'));

		const nextBtn = screen.getByRole('button', { name: 'Next' });
		expect(nextBtn).not.toHaveAttribute('aria-busy');
	});
});
