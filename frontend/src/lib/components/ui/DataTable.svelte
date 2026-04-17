<script lang="ts">
	import type { Snippet } from 'svelte';
	import Callout from './Callout.svelte';
	import EmptyState from './EmptyState.svelte';

	export type DataTableColumn = {
		key: string;
		label: string;
		align?: 'left' | 'center' | 'right';
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
		rowActionsLabel = 'Actions'
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
	} = $props();

	function resolveRowKey(rowValue: Record<string, unknown>, index: number): string | number {
		return rowKey ? rowKey(rowValue, index) : `${index}`;
	}
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
	<div
		class="overflow-hidden rounded-[4px] border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm"
		data-ui="data-table"
	>
		<div class="overflow-x-auto">
			<table class="min-w-full border-collapse text-[12px]">
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
									class={`px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.12em] ${
										column.align === 'right' ? 'text-right' : column.align === 'center' ? 'text-center' : 'text-left'
									}`}
									scope="col"
								>
									{column.label}
								</th>
							{/each}
							{#if rowActions}
								<th class="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-[0.12em]" scope="col">
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
										class={`px-4 py-3 text-[var(--text-primary)] ${
											column.align === 'right' ? 'text-right' : column.align === 'center' ? 'text-center' : 'text-left'
										}`}
									>
										{String(rowValue[column.key] ?? '')}
									</td>
								{/each}
								{#if rowActions}
									<td class="px-4 py-3">
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
{/if}
