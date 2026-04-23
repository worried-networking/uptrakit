<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		title,
		description,
		actions,
		children
	}: {
		title?: string;
		description?: string;
		actions?: Snippet;
		children: Snippet;
	} = $props();
</script>

<section
	class="rounded-card border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm"
	data-ui="section-card"
>
	{#if title || description || actions}
		<header
			class="flex flex-col gap-3 border-b border-[var(--border-subtle)] card-padding md:flex-row md:items-start md:justify-between"
		>
			<div class="space-y-1">
				{#if title}
					<h2 class="text-section-title font-semibold text-[var(--text-primary)]">{title}</h2>
				{/if}
				{#if description}
					<p class="text-sm text-[var(--text-secondary)]">{description}</p>
				{/if}
			</div>
			{#if actions}
				<div class="flex shrink-0 flex-wrap items-center gap-2">
					{@render actions()}
				</div>
			{/if}
		</header>
	{/if}

	<div class="card-padding">
		{@render children()}
	</div>
</section>
