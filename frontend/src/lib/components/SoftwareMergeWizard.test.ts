import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { showError } from '$lib/notifications.svelte';
import type {
	MergeSoftwareItemSummary,
	MergeSoftwareItemsExecuteRequest,
	MergeSoftwareItemsExecuteResponse,
	MergeSoftwareItemsPreviewRequest,
	MergeSoftwareItemsPreviewResponse
} from '$lib/types';
import SoftwareMergeWizard from './SoftwareMergeWizard.svelte';

vi.mock('$lib/notifications.svelte', () => ({
	showError: vi.fn()
}));

const candidates: MergeSoftwareItemSummary[] = [
	{ id: 'item-1', name: 'Alpha', host_count: 3, plugins: ['apt'] },
	{ id: 'item-2', name: 'Beta', host_count: 2, plugins: ['apt', 'docker'] },
	{ id: 'item-3', name: 'Gamma', host_count: 1, plugins: ['docker'] }
];

type PreviewMergeMock = (request: MergeSoftwareItemsPreviewRequest) => Promise<MergeSoftwareItemsPreviewResponse>;
type ExecuteMergeMock = (request: MergeSoftwareItemsExecuteRequest) => Promise<MergeSoftwareItemsExecuteResponse>;
type SearchCandidatesMock = (query: string) => Promise<MergeSoftwareItemSummary[]>;

function makePreview(
	candidateSet: MergeSoftwareItemSummary[] = candidates,
	survivorId = candidateSet[0]!.id
): MergeSoftwareItemsPreviewResponse {
	const survivor = candidateSet.find((candidate) => candidate.id === survivorId)!;
	const losers = candidateSet.filter((candidate) => candidate.id !== survivorId);

	return {
		candidates: candidateSet,
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
		candidate_count: candidateSet.length,
		loser_count: losers.length,
		moved_link_count: 1,
		skipped_duplicate_link_count: 1
	};
}

function createDeferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (reason?: unknown) => void;

	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});

	return { promise, resolve, reject };
}

function renderWizard({
	candidateSet = candidates,
	seedItemId,
	searchCandidates,
	initialSearchQuery,
	previewResult = makePreview(candidateSet),
	previewMerge: customPreviewMerge,
	executeResult = {
		survivor_id: candidateSet[0]?.id ?? 'item-1',
		deleted_ids: candidateSet.slice(1).map((candidate) => candidate.id),
		moved_link_ids: ['link-1'],
		skipped_duplicate_link_ids: ['link-2']
	} satisfies MergeSoftwareItemsExecuteResponse,
	executeMerge: customExecuteMerge
}: {
	candidateSet?: MergeSoftwareItemSummary[];
	seedItemId?: string | null;
	searchCandidates?: SearchCandidatesMock;
	initialSearchQuery?: string;
	previewResult?: MergeSoftwareItemsPreviewResponse;
	previewMerge?: PreviewMergeMock;
	executeResult?: MergeSoftwareItemsExecuteResponse;
	executeMerge?: ExecuteMergeMock;
} = {}) {
	const previewMerge = customPreviewMerge ?? vi.fn<PreviewMergeMock>().mockResolvedValue(previewResult);
	const executeMerge = customExecuteMerge ?? vi.fn<ExecuteMergeMock>().mockResolvedValue(executeResult);
	const onclose = vi.fn();
	const onsuccess = vi.fn();

	const view = render(SoftwareMergeWizard, {
		candidates: candidateSet,
		seedItemId,
		searchCandidates,
		initialSearchQuery,
		previewMerge,
		executeMerge,
		onclose,
		onsuccess
	});

	return { ...view, previewMerge, executeMerge, onclose, onsuccess, executeResult };
}

afterEach(() => {
	cleanup();
	vi.clearAllMocks();
});

describe('SoftwareMergeWizard', () => {
	it('does not expose search UI when no search function is provided and renders preview sections', async () => {
		const user = userEvent.setup();
		const { previewMerge } = renderWizard();

		expect(screen.getByLabelText('Merge wizard steps')).toHaveAttribute('data-ui', 'software-merge-workflow');
		expect(screen.queryByPlaceholderText('Search software items')).not.toBeInTheDocument();
		const cancelButton = screen.getByRole('button', { name: 'Cancel' });
		const nextButton = screen.getByRole('button', { name: 'Next' });
		expect(cancelButton.compareDocumentPosition(nextButton) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

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
	});

	it('supports tenant-wide candidate search with add and remove before preview', async () => {
		const user = userEvent.setup();
		const delta: MergeSoftwareItemSummary = { id: 'item-4', name: 'Delta', host_count: 4, plugins: ['npm'] };
		const searchCandidates = vi.fn<SearchCandidatesMock>().mockResolvedValue([candidates[0], candidates[1], delta]);
		const previewMerge = vi
			.fn<PreviewMergeMock>()
			.mockResolvedValue(makePreview([candidates[0], candidates[1]], 'item-2'));

		renderWizard({
			candidateSet: [candidates[0]],
			seedItemId: 'item-1',
			searchCandidates,
			initialSearchQuery: 'Alpha',
			previewMerge
		});

		expect(screen.getByLabelText('Search software items')).toHaveValue('Alpha');
		expect(screen.queryByLabelText('Remove Alpha')).not.toBeInTheDocument();

		await user.click(screen.getByRole('button', { name: 'Search' }));

		await waitFor(() => expect(searchCandidates).toHaveBeenCalledWith('Alpha'));
		await user.click(await screen.findByLabelText('Add Beta'));
		await user.click(screen.getByLabelText('Add Delta'));
		await user.click(screen.getByLabelText('Remove Delta'));
		await user.click(screen.getByLabelText('Keep Beta'));
		await user.click(screen.getByRole('button', { name: 'Next' }));

		await waitFor(() =>
			expect(previewMerge).toHaveBeenCalledWith({
				candidate_ids: ['item-1', 'item-2'],
				survivor_id: 'item-2',
				seed_item_id: 'item-1'
			})
		);
	});

	it('prefills the initial tenant-wide search query for seeded single-item flows', async () => {
		const user = userEvent.setup();
		const searchCandidates = vi.fn<SearchCandidatesMock>().mockResolvedValue([candidates[1]]);

		renderWizard({
			candidateSet: [candidates[0]],
			seedItemId: 'item-1',
			searchCandidates,
			initialSearchQuery: 'Alpha'
		});

		expect(screen.getByLabelText('Search software items')).toHaveValue('Alpha');
		await user.click(screen.getByRole('button', { name: 'Search' }));
		await waitFor(() => expect(searchCandidates).toHaveBeenCalledWith('Alpha'));
		expect(await screen.findByText('Beta')).toBeInTheDocument();
	});

	it('handles choosing a different survivor before preview', async () => {
		const user = userEvent.setup();
		const { previewMerge } = renderWizard({ previewResult: makePreview(candidates, 'item-2') });

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

	it('locks survivor selection during preview and executes the previewed survivor', async () => {
		const user = userEvent.setup();
		const previewDeferred = createDeferred<MergeSoftwareItemsPreviewResponse>();
		const previewMerge = vi.fn<PreviewMergeMock>().mockReturnValue(previewDeferred.promise);
		const executeMerge = vi.fn<ExecuteMergeMock>().mockResolvedValue({
			survivor_id: 'item-2',
			deleted_ids: ['item-1', 'item-3'],
			moved_link_ids: ['link-1'],
			skipped_duplicate_link_ids: ['link-2']
		} satisfies MergeSoftwareItemsExecuteResponse);

		renderWizard({ previewMerge, executeMerge });

		await user.click(screen.getByLabelText('Keep Beta'));
		await user.click(screen.getByRole('button', { name: 'Next' }));

		await waitFor(() =>
			expect(previewMerge).toHaveBeenCalledWith({
				candidate_ids: ['item-1', 'item-2', 'item-3'],
				survivor_id: 'item-2'
			})
		);

		expect(screen.getByLabelText('Keep Alpha')).toBeDisabled();
		expect(screen.getByLabelText('Keep Beta')).toBeDisabled();
		expect(screen.getByLabelText('Keep Gamma')).toBeDisabled();

		previewDeferred.resolve(makePreview(candidates, 'item-2'));

		await screen.findByRole('heading', { name: 'Keep' });
		await user.click(screen.getByRole('button', { name: 'Merge' }));
		expect(screen.getByText('Confirm Merge')).toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Merge Items' }));

		await waitFor(() =>
			expect(executeMerge).toHaveBeenCalledWith({
				candidate_ids: ['item-1', 'item-2', 'item-3'],
				survivor_id: 'item-2'
			})
		);
	});

	it('keeps the wizard on preview when merge confirmation is cancelled and clears stacked state on back', async () => {
		const user = userEvent.setup();
		const executeMerge = vi.fn<ExecuteMergeMock>().mockResolvedValue({
			survivor_id: 'item-1',
			deleted_ids: ['item-2', 'item-3'],
			moved_link_ids: ['link-1'],
			skipped_duplicate_link_ids: ['link-2']
		} satisfies MergeSoftwareItemsExecuteResponse);

		renderWizard({ executeMerge });

		await user.click(screen.getByRole('button', { name: 'Next' }));
		await screen.findByRole('heading', { name: 'Keep' });

		await user.click(screen.getByRole('button', { name: 'Merge' }));
		const confirmDialog = screen.getByText('Confirm Merge').closest('[role="dialog"]');
		expect(confirmDialog).not.toBeNull();

		await user.click(within(confirmDialog as HTMLElement).getByRole('button', { name: 'Cancel' }));
		expect(screen.queryByText('Confirm Merge')).not.toBeInTheDocument();
		expect(screen.getByRole('heading', { name: 'Keep' })).toBeInTheDocument();
		expect(executeMerge).not.toHaveBeenCalled();

		await user.click(screen.getByRole('button', { name: 'Merge' }));
		expect(screen.getByText('Confirm Merge')).toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Back' }));

		expect(screen.queryByText('Confirm Merge')).not.toBeInTheDocument();
		expect(screen.getByText('Choose the software item to keep')).toBeInTheDocument();
		expect(screen.queryByRole('heading', { name: 'Keep' })).not.toBeInTheDocument();
		expect(executeMerge).not.toHaveBeenCalled();
	});

	it('calls onclose from the footer cancel action', async () => {
		const user = userEvent.setup();
		const { onclose } = renderWizard();

		await user.click(screen.getByRole('button', { name: 'Cancel' }));

		expect(onclose).toHaveBeenCalledOnce();
	});

	it('resets to the selection step when rerendered with a new candidate set and seed item', async () => {
		const user = userEvent.setup();
		const nextCandidates: MergeSoftwareItemSummary[] = [
			{ id: 'item-4', name: 'Delta', host_count: 4, plugins: ['apt'] },
			{ id: 'item-5', name: 'Epsilon', host_count: 2, plugins: ['docker'] }
		];
		const { rerender } = renderWizard();

		await user.click(screen.getByRole('button', { name: 'Next' }));
		await screen.findByRole('heading', { name: 'Keep' });

		await rerender({
			candidates: nextCandidates,
			seedItemId: 'item-5',
			previewMerge: vi.fn<PreviewMergeMock>().mockResolvedValue({
				candidates: nextCandidates,
				survivor: nextCandidates[1],
				losers: [nextCandidates[0]],
				moved_links: [],
				skipped_duplicate_links: [],
				candidate_count: 2,
				loser_count: 1,
				moved_link_count: 0,
				skipped_duplicate_link_count: 0
			}),
			executeMerge: vi.fn<ExecuteMergeMock>().mockResolvedValue({
				survivor_id: 'item-5',
				deleted_ids: ['item-4'],
				moved_link_ids: [],
				skipped_duplicate_link_ids: []
			}),
			onclose: vi.fn(),
			onsuccess: vi.fn()
		});

		expect(screen.getByText('Choose the software item to keep')).toBeInTheDocument();
		expect(screen.queryByRole('heading', { name: 'Keep' })).not.toBeInTheDocument();
		expect(screen.queryByText('Alpha')).not.toBeInTheDocument();
		expect(screen.getByText('Delta')).toBeInTheDocument();
		expect(screen.getByLabelText('Keep Epsilon')).toBeChecked();
		expect(screen.getByRole('button', { name: 'Next' })).toBeInTheDocument();
	});

	it('shows a validation error when fewer than two candidates are selected', async () => {
		const user = userEvent.setup();
		const { previewMerge } = renderWizard({
			candidateSet: [candidates[0]],
			seedItemId: 'item-1'
		});

		await user.click(screen.getByRole('button', { name: 'Next' }));

		expect(showError).toHaveBeenCalledWith('Choose at least two software items to merge before continuing.');
		expect(previewMerge).not.toHaveBeenCalled();
	});
});
