<script lang="ts">
	let {
		data = {},
		loading = false,
		emptyMessage = 'No data available.'
	}: {
		data?: Record<string, unknown>;
		loading?: boolean;
		emptyMessage?: string;
	} = $props();

	const entries = $derived(Object.entries(data));
</script>

{#if loading}
	<p class="py-8 text-center text-surface-500">Loading...</p>
{:else if entries.length === 0}
	<p class="py-8 text-center text-surface-500">{emptyMessage}</p>
{:else}
	<dl class="divide-y divide-surface-200 dark:divide-surface-700">
		{#each entries as [key, value] (key)}
			<div class="flex justify-between py-2">
				<dt class="text-sm font-medium text-surface-600 dark:text-surface-400">{key}</dt>
				<dd class="text-sm">{String(value ?? '')}</dd>
			</div>
		{/each}
	</dl>
{/if}
