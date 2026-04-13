import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { goto } from '$app/navigation';
import type {
	MergeSoftwareItemSummary,
	MergeSoftwareItemsExecuteResponse,
	MergeSoftwareItemsPreviewResponse
} from '$lib/types';
import SoftwareMergeWizard from './SoftwareMergeWizard.svelte';

vi.mock('$app/navigation', () => ({
	goto: vi.fn()
}));

vi.mock('$lib/notifications.svelte', () => ({
	showError: vi.fn()
}));

const candidates: MergeSoftwareItemSummary[] = [
	{ id: 'item-1', name: 'Alpha', host_count: 3, plugins: ['apt'] },
	{ id: 'item-2', name: 'Beta', host_count: 2, plugins: ['apt', 'docker'] },
	{ id: 'item-3', name: 'Gamma', host_count: 1, plugins: ['docker'] }
];

function makePreview(survivorId = 'item-1'): MergeSoftwareItemsPreviewResponse {
	const survivor = candidates.find((candidate) => candidate.id === survivorId)!;
	const losers = candidates.filter((candidate) => candidate.id !== survivorId);

	return {
		candidates,
		survivor,
		losers,
		moved_links: [
			{
				id: 'link-1',
				host_id: 'host-1',
				hostname: 'srv-1',
				friendly_name: 'Server One',
				qualifier: null
			}
		],
		skipped_duplicate_links: [
			{
				id: 'link-2',
				host_id: 'host-2',
				hostname: 'srv-2',
				friendly_name: 'Server Two',
				qualifier: 'stable'
			}
		],
		candidate_count: candidates.length,
		loser_count: losers.length,
		moved_link_count: 1,
		skipped_duplicate_link_count: 1
	};
}

function renderWizard({
	previewResult = makePreview(),
	executeResult = {
		survivor_id: 'item-1',
		deleted_ids: ['item-2', 'item-3'],
		moved_link_ids: ['link-1'],
		skipped_duplicate_link_ids: ['link-2']
	} satisfies MergeSoftwareItemsExecuteResponse
}: {
	previewResult?: MergeSoftwareItemsPreviewResponse;
	executeResult?: MergeSoftwareItemsExecuteResponse;
} = {}) {
	const previewMerge = vi.fn().mockResolvedValue(previewResult);
	const executeMerge = vi.fn().mockResolvedValue(executeResult);
	const onclose = vi.fn();
	const onsuccess = vi.fn();

	render(SoftwareMergeWizard, {
		candidates,
		previewMerge,
		executeMerge,
		onclose,
		onsuccess
	});

	return { previewMerge, executeMerge, onclose, onsuccess, executeResult };
}

afterEach(() => {
	cleanup();
	vi.clearAllMocks();
});

describe('SoftwareMergeWizard', () => {
	it('renders preview sections after clicking Next', async () => {
		const user = userEvent.setup();
		const { previewMerge } = renderWizard();

		expect(screen.getByPlaceholderText('Search software items')).toBeInTheDocument();

		await user.click(screen.getByRole('button', { name: 'Next' }));

		await waitFor(() =>
			expect(previewMerge).toHaveBeenCalledWith({
				candidate_ids: ['item-1', 'item-2', 'item-3'],
				survivor_id: 'item-1'
			})
		);

		expect(await screen.findByRole('heading', { name: 'Keep' })).toBeInTheDocument();
		expect(screen.getByRole('heading', { name: 'Delete' })).toBeInTheDocument();
		expect(screen.getByRole('heading', { name: 'Moved links' })).toBeInTheDocument();
		expect(screen.getByRole('heading', { name: 'Already present' })).toBeInTheDocument();
		expect(screen.getByText('Alpha')).toBeInTheDocument();
		expect(screen.getByText('Beta')).toBeInTheDocument();
		expect(screen.getByText('Server One')).toBeInTheDocument();
		expect(screen.getByText('Server Two')).toBeInTheDocument();
	});

	it('handles choosing a different survivor before preview', async () => {
		const user = userEvent.setup();
		const { previewMerge } = renderWizard({ previewResult: makePreview('item-2') });

		await user.click(screen.getByLabelText('Keep Beta'));
		await user.click(screen.getByRole('button', { name: 'Next' }));

		await waitFor(() =>
			expect(previewMerge).toHaveBeenCalledWith({
				candidate_ids: ['item-1', 'item-2', 'item-3'],
				survivor_id: 'item-2'
			})
		);

		expect(await screen.findByText('Beta')).toBeInTheDocument();
		expect(screen.getByText('Alpha')).toBeInTheDocument();
		expect(screen.getByText('Gamma')).toBeInTheDocument();
	});

	it('calls onsuccess and does not navigate on execute success', async () => {
		const user = userEvent.setup();
		const { executeMerge, onsuccess, executeResult } = renderWizard();

		await user.click(screen.getByRole('button', { name: 'Next' }));
		await screen.findByRole('heading', { name: 'Keep' });
		await user.click(screen.getByRole('button', { name: 'Merge' }));

		await waitFor(() =>
			expect(executeMerge).toHaveBeenCalledWith({
				candidate_ids: ['item-1', 'item-2', 'item-3'],
				survivor_id: 'item-1'
			})
		);

		await waitFor(() => expect(onsuccess).toHaveBeenCalledWith(executeResult));
		expect(vi.mocked(goto)).not.toHaveBeenCalled();
	});

	it('calls onclose from the footer cancel action', async () => {
		const user = userEvent.setup();
		const { onclose } = renderWizard();

		await user.click(screen.getByRole('button', { name: 'Cancel' }));

		expect(onclose).toHaveBeenCalledOnce();
	});
});
