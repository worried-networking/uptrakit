<script lang="ts">
	import Modal from '$lib/components/Modal.svelte';
	import { showError } from '$lib/notifications.svelte';
	import type {
		MergeSoftwareItemSummary,
		MergeSoftwareItemsExecuteRequest,
		MergeSoftwareItemsExecuteResponse,
		MergeSoftwareItemsPreviewRequest,
		MergeSoftwareItemsPreviewResponse
	} from '$lib/types';

	type PreviewMergeFn = (request: MergeSoftwareItemsPreviewRequest) => Promise<MergeSoftwareItemsPreviewResponse>;
	type ExecuteMergeFn = (request: MergeSoftwareItemsExecuteRequest) => Promise<MergeSoftwareItemsExecuteResponse>;
	type SearchCandidatesFn = (query: string) => Promise<MergeSoftwareItemSummary[]>;

	let {
		candidates,
		seedItemId = null,
		searchCandidates = null,
		initialSearchQuery = '',
		onclose,
		onsuccess,
		previewMerge,
		executeMerge
	}: {
		candidates: MergeSoftwareItemSummary[];
		seedItemId?: string | null;
		searchCandidates?: SearchCandidatesFn | null;
		initialSearchQuery?: string;
		onclose: () => void;
		onsuccess: (result: MergeSoftwareItemsExecuteResponse) => void | Promise<void>;
		previewMerge: PreviewMergeFn;
		executeMerge: ExecuteMergeFn;
	} = $props();

	let step: 1 | 2 = $state(1);
	let loading = $state(false);
	let preview = $state<MergeSoftwareItemsPreviewResponse | null>(null);
	let previewSurvivorId = $state<string | null>(null);
	let survivorId = $state('');
	let selectedCandidates = $state<MergeSoftwareItemSummary[]>([]);
	let searchQuery = $state('');
	let searchResults = $state<MergeSoftwareItemSummary[]>([]);
	let searchLoading = $state(false);
	let candidateResetVersion = $state(0);
	let lastCandidateResetKey = '';
	let searchRequestVersion = 0;

	const candidateIds = $derived(selectedCandidates.map((candidate) => candidate.id));
	const selectedCandidateIdSet = $derived(new Set(candidateIds));
	const availableSearchResults = $derived(
		searchResults.filter((candidate) => !selectedCandidateIdSet.has(candidate.id))
	);
	const candidateResetKey = $derived(
		`${seedItemId ?? ''}:${initialSearchQuery}:${candidates.map((candidate) => candidate.id).join(',')}`
	);

	$effect(() => {
		const nextCandidateResetKey = candidateResetKey;
		if (nextCandidateResetKey === lastCandidateResetKey) return;

		lastCandidateResetKey = nextCandidateResetKey;
		const nextCandidates = [...candidates];
		const defaultSurvivorId =
			(seedItemId && nextCandidates.some((candidate) => candidate.id === seedItemId)
				? seedItemId
				: nextCandidates[0]?.id) ?? '';

		candidateResetVersion += 1;
		step = 1;
		loading = false;
		preview = null;
		previewSurvivorId = null;
		selectedCandidates = nextCandidates;
		survivorId = defaultSurvivorId;
		searchQuery = initialSearchQuery.trim();
		searchResults = [];
		searchLoading = false;

		if (searchCandidates && initialSearchQuery.trim()) {
			void runSearch();
		}
	});

	function pluginSummary(candidate: MergeSoftwareItemSummary): string {
		return candidate.plugins.join(', ') || '\u2014';
	}

	async function runSearch() {
		if (!searchCandidates || loading) return;

		const query = searchQuery.trim();
		const resetVersion = candidateResetVersion;
		const requestVersion = ++searchRequestVersion;

		searchLoading = true;
		try {
			const nextResults = await searchCandidates(query);
			if (resetVersion !== candidateResetVersion || requestVersion !== searchRequestVersion) return;
			searchResults = nextResults;
		} catch (error) {
			if (resetVersion === candidateResetVersion && requestVersion === searchRequestVersion) {
				showError(error instanceof Error ? error.message : 'Failed to search software items');
				searchResults = [];
			}
		} finally {
			if (resetVersion === candidateResetVersion && requestVersion === searchRequestVersion) {
				searchLoading = false;
			}
		}
	}

	function addCandidate(candidate: MergeSoftwareItemSummary) {
		if (loading || selectedCandidateIdSet.has(candidate.id)) return;
		selectedCandidates = [...selectedCandidates, candidate];
		if (!survivorId) {
			survivorId = candidate.id;
		}
	}

	function removeCandidate(candidateId: string) {
		if (loading || candidateId === seedItemId) return;
		const nextCandidates = selectedCandidates.filter((candidate) => candidate.id !== candidateId);
		selectedCandidates = nextCandidates;
		if (survivorId === candidateId) {
			survivorId = nextCandidates[0]?.id ?? '';
		}
	}

	async function goToPreview() {
		if (loading) return;
		if (selectedCandidates.length < 2) {
			showError('Choose at least two software items to merge before continuing.');
			return;
		}
		if (!survivorId) {
			showError('Choose which software item to keep before continuing.');
			return;
		}
		if (!selectedCandidateIdSet.has(survivorId)) {
			showError('The survivor must stay within the selected candidate set.');
			return;
		}

		const requestedSurvivorId = survivorId;
		const resetVersion = candidateResetVersion;
		loading = true;
		try {
			const nextPreview = await previewMerge({
				candidate_ids: candidateIds,
				survivor_id: requestedSurvivorId,
				...(seedItemId ? { seed_item_id: seedItemId } : {})
			});
			if (resetVersion !== candidateResetVersion) return;
			preview = nextPreview;
			previewSurvivorId = requestedSurvivorId;
			step = 2;
		} catch (error) {
			if (resetVersion === candidateResetVersion) {
				showError(error instanceof Error ? error.message : 'Failed to preview software item merge');
			}
		} finally {
			if (resetVersion === candidateResetVersion) {
				loading = false;
			}
		}
	}

	async function merge() {
		if (loading) return;

		const executeSurvivorId = previewSurvivorId ?? survivorId;
		const resetVersion = candidateResetVersion;
		loading = true;
		try {
			const result = await executeMerge({
				candidate_ids: candidateIds,
				survivor_id: executeSurvivorId
			});
			if (resetVersion !== candidateResetVersion) return;
			await onsuccess(result);
		} catch (error) {
			if (resetVersion === candidateResetVersion) {
				showError(error instanceof Error ? error.message : 'Failed to merge software items');
			}
		} finally {
			if (resetVersion === candidateResetVersion) {
				loading = false;
			}
		}
	}
</script>

<Modal title="Merge Software Items" maxWidth="max-w-3xl" {onclose}>
	<div class="mb-6 flex items-center gap-2">
		<div class="flex items-center gap-2">
			<div
				class="flex h-7 w-7 items-center justify-center rounded-full text-xs font-bold {step === 1
					? 'bg-primary-500 text-white'
					: 'bg-primary-200 dark:bg-primary-800 text-primary-700 dark:text-primary-200'}"
			>
				1
			</div>
			<span class="text-sm {step === 1 ? 'font-semibold' : 'text-surface-500'}">Choose survivor</span>
		</div>
		<div class="h-px w-8 bg-surface-300 dark:bg-surface-600"></div>
		<div class="flex items-center gap-2">
			<div
				class="flex h-7 w-7 items-center justify-center rounded-full text-xs font-bold {step === 2
					? 'bg-primary-500 text-white'
					: 'bg-surface-200 dark:bg-surface-700 text-surface-500'}"
			>
				2
			</div>
			<span class="text-sm {step === 2 ? 'font-semibold' : 'text-surface-500'}">Review preview</span>
		</div>
	</div>

	{#if step === 1}
		<div class="space-y-4">
			<div class="space-y-1">
				<h4 class="h4">Choose the software item to keep</h4>
				<p class="text-sm text-surface-500">
					Adjust the candidate set if needed, then choose the survivor and preview which links will move and which
					duplicates will be skipped.
				</p>
			</div>

			{#if searchCandidates}
				<section class="space-y-3">
					<div class="space-y-1">
						<h5 class="font-semibold">Add candidates from this tenant</h5>
						<p class="text-sm text-surface-500">
							Search across all software items in the tenant and add the ones that belong in this merge. Edit the query
							or leave it blank to browse the tenant list.
						</p>
					</div>

					<div class="flex flex-wrap items-end gap-2">
						<label class="label flex-1">
							<span>Search software items</span>
							<input
								class="input"
								type="search"
								bind:value={searchQuery}
								placeholder="Search software items"
								aria-label="Search software items"
								disabled={loading}
								onkeydown={(event) => {
									if (event.key !== 'Enter') return;
									event.preventDefault();
									void runSearch();
								}}
							/>
						</label>
						<button class="btn preset-tonal-surface" type="button" disabled={loading} onclick={runSearch}>
							Search
						</button>
					</div>

					<div class="card p-4">
						{#if searchLoading}
							<p class="text-sm text-surface-500">Searching...</p>
						{:else if searchResults.length === 0}
							<p class="text-sm text-surface-500">Run a tenant-wide search to find more candidates.</p>
						{:else if availableSearchResults.length === 0}
							<p class="text-sm text-surface-500">No additional software items match this search.</p>
						{:else}
							<ul class="space-y-2 text-sm">
								{#each availableSearchResults as candidate (candidate.id)}
									<li class="flex items-start justify-between gap-3">
										<div class="min-w-0 flex-1">
											<p class="font-medium">{candidate.name}</p>
											<p class="text-surface-500">
												{candidate.host_count} host(s) · {pluginSummary(candidate)}
											</p>
										</div>
										<button
											class="btn btn-sm preset-tonal-surface"
											type="button"
											disabled={loading}
											aria-label={`Add ${candidate.name}`}
											onclick={() => addCandidate(candidate)}
										>
											Add
										</button>
									</li>
								{/each}
							</ul>
						{/if}
					</div>
				</section>
			{/if}

			<div class="space-y-3">
				{#each selectedCandidates as candidate (candidate.id)}
					<div class="card flex items-start gap-3 p-4">
						<input
							type="radio"
							class="radio mt-1"
							name="survivor"
							value={candidate.id}
							bind:group={survivorId}
							disabled={loading}
							aria-label={`Keep ${candidate.name}`}
						/>
						<div class="min-w-0 flex-1">
							<div class="flex flex-wrap items-center gap-2">
								<h5 class="font-semibold">{candidate.name}</h5>
								<span class="badge preset-tonal-surface text-xs">{candidate.host_count} host(s)</span>
								{#if candidate.id === seedItemId}
									<span class="badge preset-filled-primary-500 text-xs">Seed item</span>
								{/if}
							</div>
							<p class="mt-1 text-sm text-surface-500">Plugins: {pluginSummary(candidate)}</p>
						</div>
						{#if candidate.id !== seedItemId}
							<button
								class="btn btn-sm preset-tonal-surface"
								type="button"
								disabled={loading}
								aria-label={`Remove ${candidate.name}`}
								onclick={() => removeCandidate(candidate.id)}
							>
								Remove
							</button>
						{/if}
					</div>
				{/each}
			</div>

			<p class="text-sm text-surface-500">
				{selectedCandidates.length} candidate(s) selected. You need at least 2 to continue.
			</p>
		</div>
	{:else if preview}
		<div class="space-y-5">
			<div class="card preset-tonal-primary p-4">
				<p class="text-sm text-surface-700 dark:text-surface-200">
					Preview prepared for {preview.candidate_count} candidate(s). {preview.moved_link_count} link(s) will move and {preview.skipped_duplicate_link_count}
					duplicate link(s) are already present on the survivor.
				</p>
			</div>

			<section class="space-y-2">
				<h4 class="h4">Keep</h4>
				<div class="card p-4">
					<div class="flex flex-wrap items-center gap-2">
						<h5 class="font-semibold">{preview.survivor.name}</h5>
						<span class="badge preset-filled-primary-500 text-xs">Survivor</span>
					</div>
					<p class="mt-1 text-sm text-surface-500">{preview.survivor.host_count} host(s)</p>
				</div>
			</section>

			<section class="space-y-2">
				<h4 class="h4">Delete</h4>
				<div class="space-y-2">
					{#each preview.losers as loser (loser.id)}
						<div class="card p-4">
							<div class="flex flex-wrap items-center gap-2">
								<h5 class="font-semibold">{loser.name}</h5>
								<span class="badge preset-tonal-error text-xs">Merged away</span>
							</div>
							<p class="mt-1 text-sm text-surface-500">{loser.host_count} host(s)</p>
						</div>
					{/each}
				</div>
			</section>

			<section class="space-y-2">
				<h4 class="h4">Moved links</h4>
				<div class="card p-4">
					{#if preview.moved_links.length > 0}
						<ul class="space-y-2 text-sm">
							{#each preview.moved_links as link (link.id)}
								<li>
									<span class="font-medium">{link.friendly_name}</span>
									<span class="text-surface-500">
										({link.hostname}){link.qualifier ? ` - ${link.qualifier}` : ''}
									</span>
								</li>
							{/each}
						</ul>
					{:else}
						<p class="text-sm text-surface-500">No host links will move.</p>
					{/if}
				</div>
			</section>

			<section class="space-y-2">
				<h4 class="h4">Already present</h4>
				<div class="card p-4">
					{#if preview.skipped_duplicate_links.length > 0}
						<ul class="space-y-2 text-sm">
							{#each preview.skipped_duplicate_links as link (link.id)}
								<li>
									<span class="font-medium">{link.friendly_name}</span>
									<span class="text-surface-500">
										({link.hostname}){link.qualifier ? ` - ${link.qualifier}` : ''}
									</span>
								</li>
							{/each}
						</ul>
					{:else}
						<p class="text-sm text-surface-500">No duplicate host links are already present.</p>
					{/if}
				</div>
			</section>
		</div>
	{/if}

	{#snippet footer()}
		<button class="btn preset-tonal-surface" onclick={onclose} disabled={loading}>Cancel</button>
		{#if step === 2}
			<button class="btn preset-tonal-surface" onclick={() => (step = 1)} disabled={loading}>Back</button>
		{/if}
		<button class="btn preset-filled-primary-500" onclick={step === 1 ? goToPreview : merge} disabled={loading}>
			{#if step === 1}
				{loading ? 'Loading preview...' : 'Next'}
			{:else}
				{loading ? 'Merging...' : 'Merge'}
			{/if}
		</button>
	{/snippet}
</Modal>
