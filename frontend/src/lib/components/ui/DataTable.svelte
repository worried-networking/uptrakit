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
		rowActions?: Snippet<[Record<string, unknown>]>;
		rowActionsLabel?: string;
	} = $props();
</script>

{#if error}
	<Callout tone="danger" title="Unable to load data" message={error} />
{:else if loading}
	<p class="py-8 text-center text-sm text-[var(--text-secondary)]">Loading...</p>
{:else if rows.length === 0}
	<EmptyState title={emptyTitle} description={emptyDescription} />
{:else}
	<div
		class="overflow-hidden rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm"
		data-ui="data-table"
	>
		<div class="overflow-x-auto">
			<table class="min-w-full border-collapse text-sm">
				{#if caption}
					<caption class="sr-only">{caption}</caption>
				{/if}
				<thead>
					<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
						{#each columns as column (column.key)}
							<th
								class={`px-4 py-3 text-xs font-semibold uppercase tracking-[0.14em] ${
									column.align === 'right' ? 'text-right' : column.align === 'center' ? 'text-center' : 'text-left'
								}`}
								scope="col"
							>
								{column.label}
							</th>
						{/each}
						{#if rowActions}
							<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">
								{rowActionsLabel}
							</th>
						{/if}
					</tr>
				</thead>
				<tbody>
					{#each rows as row, index (`${index}`)}
						<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
							{#each columns as column (column.key)}
								<td
									class={`px-4 py-3 text-[var(--text-primary)] ${
										column.align === 'right' ? 'text-right' : column.align === 'center' ? 'text-center' : 'text-left'
									}`}
								>
									{String(row[column.key] ?? '')}
								</td>
							{/each}
							{#if rowActions}
								<td class="px-4 py-3">
									<div class="flex flex-wrap gap-2">
										{@render rowActions(row)}
									</div>
								</td>
							{/if}
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	</div>
{/if}
