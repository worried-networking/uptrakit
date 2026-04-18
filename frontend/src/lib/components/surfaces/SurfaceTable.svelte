<script lang="ts">
	import DataTable from '$lib/components/ui/DataTable.svelte';
	import TableFooterBar from '$lib/components/ui/TableFooterBar.svelte';
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
	const resolvedRowActions = $derived(
		(node.row_actions ?? [])
			.map((rowAction) => ({
				rowAction,
				interaction: interactionMap.get(rowAction.interaction_id)
			}))
			.filter((candidate): candidate is { rowAction: SurfaceTableRowAction; interaction: InteractionDescriptor } =>
				Boolean(candidate.interaction)
			)
	);
	const hasRowActions = $derived(resolvedRowActions.length > 0);
	const resolvedColumns = $derived(
		(node.columns?.length ?? 0) > 0
			? (node.columns ?? [])
			: tableRows.length > 0
				? Object.keys(tableRows[0]).map((key) => ({ key, label: key }))
				: []
	);
	const showProviderFooter = $derived(
		dataSource?.kind.kind === 'provider_query' &&
			!loadError &&
			!loading &&
			(tableRows.length > 0 || totalPages > 1 || total > perPage)
	);
	const showInlineFooter = $derived(showProviderFooter && tableRows.length > 0);
	const showStandaloneFooter = $derived(showProviderFooter && tableRows.length === 0);

	$effect(() => {
		if (dataSource?.kind.kind !== 'static') {
			return;
		}
		latestRequestId += 1;
		loading = false;
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

<div class="space-y-4">
	{#if hasRowActions}
		<DataTable
			columns={resolvedColumns}
			rows={tableRows}
			{loading}
			error={loadError}
			emptyTitle={dataSource?.empty_state?.title ?? 'No rows available'}
			emptyDescription={dataSource?.empty_state?.description}
		>
			{#snippet rowActions(row)}
				{#each resolvedRowActions as { rowAction, interaction } (rowAction.interaction_id)}
					{#if isRowActionVisible(rowAction, row)}
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
			{/snippet}
			{#snippet footer()}
				{#if showInlineFooter}
					<TableFooterBar {total} {currentPage} {totalPages} onPageChange={handlePageChange} />
				{/if}
			{/snippet}
		</DataTable>
	{:else}
		<DataTable
			columns={resolvedColumns}
			rows={tableRows}
			{loading}
			error={loadError}
			emptyTitle={dataSource?.empty_state?.title ?? 'No rows available'}
			emptyDescription={dataSource?.empty_state?.description}
		>
			{#snippet footer()}
				{#if showInlineFooter}
					<TableFooterBar {total} {currentPage} {totalPages} onPageChange={handlePageChange} />
				{/if}
			{/snippet}
		</DataTable>
	{/if}

	{#if showStandaloneFooter}
		<TableFooterBar {total} {currentPage} {totalPages} onPageChange={handlePageChange} />
	{/if}
</div>
