<script lang="ts">
	import type { ComponentType, SvelteComponent } from 'svelte';
	import type { Snippet } from 'svelte';

	let {
		title,
		description,
		actions,
		icon: IconComponent
	}: {
		title: string;
		description?: string;
		actions?: Snippet;
		icon?: ComponentType<SvelteComponent>;
	} = $props();
</script>

<section
	class="rounded-card border border-dashed border-[var(--border-default)] bg-[var(--bg-surface)] px-6 py-8 text-center"
	data-ui="empty-state"
>
	<div class="mx-auto max-w-md space-y-2">
		{#if IconComponent}
			<div class="mb-3 flex justify-center">
				<IconComponent size={32} class="text-[var(--text-muted)]" aria-hidden="true" />
			</div>
		{/if}
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
