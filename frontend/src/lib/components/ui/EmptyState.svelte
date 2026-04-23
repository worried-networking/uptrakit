<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		title,
		description,
		actions
	}: {
		title: string;
		description?: string;
		actions?: Snippet;
	} = $props();
</script>

<section
	class="rounded-card border border-dashed border-[var(--border-default)] bg-[var(--bg-surface)] px-6 py-8 text-center"
	data-ui="empty-state"
>
	<div class="mx-auto max-w-md space-y-2">
		<h2 class="text-section-title font-semibold text-[var(--text-primary)]">{title}</h2>
		{#if description}
			<p class="text-sm text-[var(--text-secondary)]">{description}</p>
		{/if}
		{#if actions}
			<div class="flex justify-center pt-2">
				{@render actions()}
			</div>
		{/if}
	</div>
</section>
