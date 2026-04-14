<script lang="ts">
	import Pagination from '$lib/components/Pagination.svelte';
	import { invokeSurfaceInteraction } from '$lib/api';
	import SurfaceInteractionButton from './SurfaceInteractionButton.svelte';
	import { buildSurfaceInteractionRequest, type SurfaceEncryptionContext } from '$lib/surfaces/interactions';
	import type {
		DataSourceDescriptor,
		InteractionDescriptor,
		SurfaceNode,
		SurfaceTableRowAction
	} from '$lib/surfaces/contract';

	let {
		surfaceId,
		node,
		dataSource,
		dataLoadInteraction,
		interactions = [],
		targetProviderId,
		encryptionContext,
		baseParams = {},
		rows = []
	}: {
		surfaceId: string;
		node: Extract<SurfaceNode, { kind: 'table' }>;
		dataSource?: DataSourceDescriptor;
		dataLoadInteraction?: InteractionDescriptor;
		interactions?: InteractionDescriptor[];
		targetProviderId?: string;
		encryptionContext?: SurfaceEncryptionContext;
		baseParams?: Record<string, unknown>;
		rows?: Record<string, unknown>[];
	} = $props();

	let loading = $state(false);
	let loadError = $state<string | null>(null);
	let tableRows = $state<Record<string, unknown>[]>([]);
	let currentPage = $state(1);
	let totalPages = $state(1);
	let total = $state(0);
	let perPage = $derived(dataSource?.pagination?.default_page_size ?? 20);
	let latestRequestId = 0;

	const interactionMap = $derived(
		new Map(interactions.map((interaction) => [interaction.interaction_id, interaction]))
	);
	const resolvedColumns = $derived(
		(node.columns?.length ?? 0) > 0
			? (node.columns ?? [])
			: tableRows.length > 0
				? Object.keys(tableRows[0]).map((key) => ({ key, label: key }))
				: []
	);

	$effect(() => {
		if (dataSource?.kind.kind !== 'static') {
			return;
		}
		const staticRows = Array.isArray(dataSource.kind.data) ? (dataSource.kind.data as Record<string, unknown>[]) : rows;
		tableRows = staticRows;
		total = staticRows.length;
		totalPages = 1;
		currentPage = 1;
		loadError = null;
	});

	$effect(() => {
		if (!dataSource || dataSource.kind.kind !== 'provider_query' || !dataLoadInteraction) {
			return;
		}
		void loadPage(currentPage);
	});

	$effect(() => {
		if (typeof window === 'undefined') {
			return;
		}
		const handleReload = (event: Event) => {
			if (!dataSource || dataSource.kind.kind !== 'provider_query' || !dataLoadInteraction) {
				return;
			}
			const detail = (event as CustomEvent<{ surfaceId?: string; targetProviderId?: string | null }>).detail;
			if (!detail || detail.surfaceId !== surfaceId) {
				return;
			}
			if ((detail.targetProviderId ?? null) !== (targetProviderId ?? null)) {
				return;
			}
			void loadPage(currentPage);
		};
		window.addEventListener('surface:reload', handleReload);
		return () => {
			window.removeEventListener('surface:reload', handleReload);
		};
	});

	function isRowActionVisible(rowAction: SurfaceTableRowAction, row: Record<string, unknown>): boolean {
		if (!rowAction.visible_when) {
			return true;
		}
		const value = row[rowAction.visible_when.field];
		const isPresent = value != null && value !== '';
		return rowAction.visible_when.condition === 'present' ? isPresent : !isPresent;
	}

	function rowParams(row: Record<string, unknown>): Record<string, unknown> {
		return {
			...baseParams,
			...row
		};
	}

	async function loadPage(page: number): Promise<void> {
		if (!dataSource || dataSource.kind.kind !== 'provider_query' || !dataLoadInteraction) {
			return;
		}

		const requestId = ++latestRequestId;
		loading = true;
		loadError = null;
		try {
			const request = await buildSurfaceInteractionRequest(
				dataLoadInteraction,
				{
					...baseParams,
					page,
					per_page: perPage
				},
				{
					targetProviderId,
					encryption: encryptionContext
				}
			);
			const result = await invokeSurfaceInteraction(surfaceId, dataLoadInteraction.interaction_id, request);
			const resultObj = result as Record<string, unknown>;
			if (requestId !== latestRequestId) {
				return;
			}

			if (resultObj?.items && resultObj?.total_pages != null) {
				tableRows = (resultObj.items as Record<string, unknown>[]) ?? [];
				total = (resultObj.total as number) ?? tableRows.length;
				currentPage = (resultObj.page as number) ?? page;
				perPage = (resultObj.per_page as number) ?? perPage;
				totalPages = (resultObj.total_pages as number) ?? 1;
			} else {
				tableRows = Array.isArray(result) ? (result as Record<string, unknown>[]) : [];
				total = tableRows.length;
				currentPage = 1;
				totalPages = 1;
			}
		} catch (error) {
			if (requestId !== latestRequestId) {
				return;
			}
			loadError = error instanceof Error ? error.message : 'Failed to load table data.';
			tableRows = [];
			total = 0;
			totalPages = 1;
		} finally {
			if (requestId === latestRequestId) {
				loading = false;
			}
		}
	}

	function handlePageChange(page: number): void {
		currentPage = page;
	}
</script>

{#if loadError}
	<aside class="rounded-lg p-4 preset-filled-error-500">{loadError}</aside>
{:else if loading}
	<p class="py-8 text-center text-surface-500">Loading...</p>
{:else if tableRows.length === 0}
	<p class="py-8 text-center text-surface-500">No rows available.</p>
{:else}
	<div class="space-y-4">
		<div class="overflow-x-auto">
			<table class="w-full table-auto border-collapse text-sm">
				<thead>
					<tr class="border-b border-surface-200 dark:border-surface-700">
						{#each resolvedColumns as column (column.key)}
							<th class="p-2 text-left font-semibold">{column.label}</th>
						{/each}
						{#if (node.row_actions?.length ?? 0) > 0}
							<th class="p-2 text-left font-semibold">Actions</th>
						{/if}
					</tr>
				</thead>
				<tbody>
					{#each tableRows as row, idx (idx)}
						<tr class="border-b border-surface-100 dark:border-surface-800">
							{#each resolvedColumns as column (column.key)}
								<td class="p-2 align-top">{String(row[column.key] ?? '')}</td>
							{/each}
							{#if (node.row_actions?.length ?? 0) > 0}
								<td class="p-2 align-top">
									<div class="flex flex-wrap gap-2">
										{#each node.row_actions ?? [] as rowAction (rowAction.interaction_id)}
											{@const interaction = interactionMap.get(rowAction.interaction_id)}
											{#if interaction && isRowActionVisible(rowAction, row)}
												<SurfaceInteractionButton
													{surfaceId}
													{interaction}
													{interactions}
													{targetProviderId}
													{encryptionContext}
													baseParams={rowParams(row)}
													rowSeed={row}
													size="sm"
													oncomplete={async () => {
														await loadPage(currentPage);
													}}
												/>
											{/if}
										{/each}
									</div>
								</td>
							{/if}
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
		<Pagination {currentPage} {totalPages} {total} onPageChange={handlePageChange} />
	</div>
{/if}
