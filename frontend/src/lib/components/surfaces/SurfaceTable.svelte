<script lang="ts">
	let {
		rows = [],
		columns = []
	}: {
		rows?: Record<string, unknown>[];
		columns?: string[];
	} = $props();

	const resolvedColumns = $derived(columns.length > 0 ? columns : rows.length > 0 ? Object.keys(rows[0]) : []);
</script>

{#if rows.length === 0}
	<p class="py-8 text-center text-surface-500">No rows available.</p>
{:else}
	<div class="overflow-x-auto">
		<table class="w-full table-auto border-collapse text-sm">
			<thead>
				<tr class="border-b border-surface-200 dark:border-surface-700">
					{#each resolvedColumns as column (column)}
						<th class="p-2 text-left font-semibold">{column}</th>
					{/each}
				</tr>
			</thead>
			<tbody>
				{#each rows as row, idx (idx)}
					<tr class="border-b border-surface-100 dark:border-surface-800">
						{#each resolvedColumns as column (column)}
							<td class="p-2 align-top">{String(row[column] ?? '')}</td>
						{/each}
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}
