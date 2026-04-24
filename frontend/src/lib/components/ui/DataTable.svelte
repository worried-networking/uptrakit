<script lang="ts">
	import type { Snippet } from 'svelte';
	import Callout from './Callout.svelte';
	import EmptyState from './EmptyState.svelte';

	export type DataTableColumn = {
		key: string;
		label: string;
		align?: 'left' | 'center' | 'right';
		mobileHide?: boolean;
		mobileTitle?: boolean;
	};

	let {
		columns = [],
		rows = [],
		caption,
		loading = false,
		error,
		emptyTitle = 'No rows available',
		emptyDescription,
		header,
		row,
		footer,
		rowKey,
		errorActions,
		rowActions,
		rowActionsLabel = 'Actions',
		mobileMode,
		mobileRow
	}: {
		columns: DataTableColumn[];
		rows: Record<string, unknown>[];
		caption?: string;
		loading?: boolean;
		error?: string | null;
		emptyTitle?: string;
		emptyDescription?: string;
		header?: Snippet;
		row?: Snippet<[Record<string, unknown>]>;
		footer?: Snippet;
		rowKey?: (row: Record<string, unknown>, index: number) => string | number;
		errorActions?: Snippet;
		rowActions?: Snippet<[Record<string, unknown>]>;
		rowActionsLabel?: string;
		mobileMode?: 'scroll' | 'cards';
		mobileRow?: Snippet<[Record<string, unknown>]>;
	} = $props();

	function resolveRowKey(rowValue: Record<string, unknown>, index: number): string | number {
		return rowKey ? rowKey(rowValue, index) : `${index}`;
	}

	// mobileMode directly drives layout. No fallback — row snippet is desktop-only and
	// does not affect mobile card generation. cards + no mobileRow = auto-generated dl/dt/dd.
	const effectiveMobileMode = $derived(mobileMode);

	// Explicit 'scroll' mode: w-max lets table overflow and trigger horizontal scroll.
	// absent or 'cards': keep min-w-full.
	const tableWidthClass = $derived(mobileMode === 'scroll' ? 'w-max' : 'min-w-full');

	// Columns visible in the auto-generated cards layout.
	// Columns with empty label (action columns) and mobileHide columns are excluded.
	const visibleMobileColumns = $derived(columns.filter((col) => !col.mobileHide && col.label !== ''));
	const titleCol = $derived(visibleMobileColumns.find((col) => col.mobileTitle) ?? visibleMobileColumns[0]);
	const valueColumns = $derived(titleCol ? visibleMobileColumns.filter((col) => col.key !== titleCol.key) : []);
</script>

{#if error}
	<Callout tone="danger" title="Unable to load data" message={error}>
		{#if errorActions}
			{@render errorActions()}
		{/if}
	</Callout>
{:else if loading}
	<p class="py-8 text-center text-sm text-[var(--text-secondary)]">Loading...</p>
{:else if rows.length === 0}
	<EmptyState title={emptyTitle} description={emptyDescription} />
{:else}
	<!-- Table layout. Hidden on mobile when effectiveMobileMode='cards'. -->
	<div
		class="overflow-hidden rounded-panel border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm{effectiveMobileMode ===
		'cards'
			? ' max-sm:hidden'
			: ''}"
		data-ui="data-table"
	>
		<div class="overflow-x-auto">
			<table class="{tableWidthClass} border-collapse text-table-body">
				{#if caption}
					<caption class="sr-only">{caption}</caption>
				{/if}
				<thead>
					{#if header}
						{@render header()}
					{:else}
						<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
							{#each columns as column (column.key)}
								<th
									class="table-cell-pad text-table-header font-semibold uppercase tracking-table-header {column.align ===
									'right'
										? 'text-right'
										: column.align === 'center'
											? 'text-center'
											: 'text-left'}"
									scope="col"
								>
									{column.label}
								</th>
							{/each}
							{#if rowActions}
								<th
									class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
									scope="col"
								>
									{rowActionsLabel}
								</th>
							{/if}
						</tr>
					{/if}
				</thead>
				<tbody>
					{#each rows as rowValue, index (resolveRowKey(rowValue, index))}
						{#if row}
							{@render row(rowValue)}
						{:else}
							<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
								{#each columns as column (column.key)}
									<td
										class="table-cell-pad text-[var(--text-primary)] {column.align === 'right'
											? 'text-right'
											: column.align === 'center'
												? 'text-center'
												: 'text-left'}"
									>
										{String(rowValue[column.key] ?? '')}
									</td>
								{/each}
								{#if rowActions}
									<td class="table-cell-pad">
										<div class="flex flex-wrap gap-2">
											{@render rowActions(rowValue)}
										</div>
									</td>
								{/if}
							</tr>
						{/if}
					{/each}
				</tbody>
			</table>
		</div>
		{#if footer}
			{@render footer()}
		{/if}
	</div>

	<!-- Cards layout: only in DOM when effectiveMobileMode='cards'.
	     Visible only on mobile via sm:hidden; table layout above is hidden on mobile via max-sm:hidden.
	     Both exist in DOM simultaneously — CSS controls visibility, not JS. -->
	{#if effectiveMobileMode === 'cards'}
		<div
			class="sm:hidden overflow-hidden rounded-panel border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm divide-y divide-[var(--border-subtle)]"
			data-ui="data-table-cards"
			role="list"
			aria-label={caption ?? undefined}
		>
			{#each rows as rowValue, index (resolveRowKey(rowValue, index))}
				{#if mobileRow}
					{@render mobileRow(rowValue)}
				{:else}
					<div role="listitem" class="px-4 py-3">
						{#if titleCol}
							<p class="truncate text-sm font-semibold text-[var(--text-primary)]">
								{String(rowValue[titleCol.key] ?? '')}
							</p>
						{/if}
						{#if valueColumns.length > 0}
							<dl class="mt-1.5 space-y-1">
								{#each valueColumns as col (col.key)}
									<div class="flex items-baseline gap-2">
										<dt
											class="shrink-0 text-table-header font-semibold uppercase tracking-table-header text-[var(--text-secondary)]"
										>
											{col.label}
										</dt>
										<dd
											class="min-w-0 truncate text-sm text-[var(--text-primary)]{col.align === 'right'
												? ' ml-auto'
												: ''}"
										>
											{String(rowValue[col.key] ?? '')}
										</dd>
									</div>
								{/each}
							</dl>
						{/if}
						{#if rowActions}
							<div role="group" aria-label={rowActionsLabel} class="mt-2 flex flex-wrap gap-2">
								{@render rowActions(rowValue)}
							</div>
						{/if}
					</div>
				{/if}
			{/each}
			{#if footer}
				{@render footer()}
			{/if}
		</div>
	{/if}
{/if}
