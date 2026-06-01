<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		filters,
		actions
	}: {
		filters?: Snippet;
		actions?: Snippet;
	} = $props();
</script>

<header
	class="flex flex-col gap-3 border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] card-padding md:flex-row md:items-center md:justify-between"
	data-ui="filter-bar"
>
	<div class="flex flex-wrap items-center gap-3">
		{@render filters?.()}
	</div>
	{#if actions}
		<div class="shrink-0">
			{@render actions()}
		</div>
	{/if}
</header>
