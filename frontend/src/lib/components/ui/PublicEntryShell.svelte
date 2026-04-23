<script lang="ts" module>
	export const PUBLIC_ENTRY_FORM_CLASS = 'space-y-4';
</script>

<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		eyebrow = 'Uptrakit',
		title,
		subtitle,
		children,
		footer
	}: {
		eyebrow?: string;
		title: string;
		subtitle?: string;
		children: Snippet;
		footer?: Snippet;
	} = $props();
</script>

<section
	class="relative flex min-h-dvh items-center justify-center overflow-hidden px-4 py-8 sm:px-6"
	data-ui="public-entry-shell"
>
	<div
		class="pointer-events-none absolute inset-0 bg-[radial-gradient(circle_at_top,rgba(var(--accent-rgb),0.12),transparent_58%),linear-gradient(160deg,var(--bg-base),var(--bg-raised))]"
		aria-hidden="true"
	></div>

	<article
		class="relative w-full max-w-2xl rounded-panel border border-[var(--border-subtle)] bg-[var(--bg-surface)]/95 shadow-lg backdrop-blur"
		data-ui="public-entry-card"
	>
		<header class="space-y-2 border-b border-[var(--border-subtle)] px-6 py-5 sm:px-8" data-ui="public-entry-header">
			<p class="text-xs font-semibold uppercase tracking-eyebrow text-[var(--text-muted)]">{eyebrow}</p>
			<h1 class="text-entry-title font-semibold tracking-tight text-[var(--text-primary)]">{title}</h1>
			{#if subtitle}
				<p class="text-sm text-[var(--text-secondary)]">{subtitle}</p>
			{/if}
		</header>

		<div class="space-y-4 px-6 py-5 sm:px-8" data-ui="public-entry-content">
			{@render children()}
		</div>

		{#if footer}
			<footer class="border-t border-[var(--border-subtle)] px-6 py-4 text-sm text-[var(--text-secondary)] sm:px-8">
				{@render footer()}
			</footer>
		{/if}
	</article>
</section>
