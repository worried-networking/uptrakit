import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
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
	previewResult = makePreview(),
	previewMerge: customPreviewMerge,
	executeResult = {
		survivor_id: 'item-1',
		deleted_ids: ['item-2', 'item-3'],
		moved_link_ids: ['link-1'],
		skipped_duplicate_link_ids: ['link-2']
	} satisfies MergeSoftwareItemsExecuteResponse,
	executeMerge: customExecuteMerge
}: {
	candidateSet?: MergeSoftwareItemSummary[];
	seedItemId?: string | null;
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
	it('does not expose search UI and renders preview sections after clicking Next', async () => {
		const user = userEvent.setup();
		const { previewMerge } = renderWizard();

		expect(screen.queryByPlaceholderText('Search software items')).not.toBeInTheDocument();
		expect(screen.queryByText('Search software items')).not.toBeInTheDocument();

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

		previewDeferred.resolve(makePreview('item-2'));

		await screen.findByRole('heading', { name: 'Keep' });
		await user.click(screen.getByRole('button', { name: 'Merge' }));

		await waitFor(() =>
			expect(executeMerge).toHaveBeenCalledWith({
				candidate_ids: ['item-1', 'item-2', 'item-3'],
				survivor_id: 'item-2'
			})
		);
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
});
