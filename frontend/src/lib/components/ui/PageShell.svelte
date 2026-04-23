<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		eyebrow,
		title,
		description,
		actions,
		children
	}: {
		eyebrow?: string;
		title: string;
		description?: string;
		actions?: Snippet;
		children: Snippet;
	} = $props();
</script>

<section class="space-y-6" data-ui="page-shell">
	<header class="flex flex-col gap-4 md:flex-row md:items-start md:justify-between">
		<div class="space-y-2">
			{#if eyebrow}
				<p class="text-xs font-semibold uppercase tracking-[0.24em] text-[var(--text-muted)]">{eyebrow}</p>
			{/if}
			<h1 class="text-[20px] font-bold text-[var(--text-primary)]">{title}</h1>
			{#if description}
				<p class="max-w-3xl text-sm text-[var(--text-secondary)]">{description}</p>
			{/if}
		</div>
		{#if actions}
			<div class="flex shrink-0 flex-wrap items-center gap-2">
				{@render actions()}
			</div>
		{/if}
	</header>

	<div class="space-y-6">
		{@render children()}
	</div>
</section>
