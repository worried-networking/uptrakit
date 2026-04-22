<script lang="ts" module>
	export const PUBLIC_ENTRY_FORM_CLASS = 'space-y-4';
	export const PUBLIC_ENTRY_INPUT_CLASS =
		'input h-8 w-full rounded-lg border border-[var(--border-default)] bg-[var(--bg-surface)] text-sm text-[var(--text-primary)] placeholder:text-[var(--text-muted)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] aria-[invalid=true]:border-[var(--color-error)] aria-[invalid=true]:bg-[var(--color-error-bg)]';
	export const PUBLIC_ENTRY_CHECKBOX_CLASS =
		'checkbox h-4 w-4 rounded border-[var(--border-default)] text-[var(--accent)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]';
	export const PUBLIC_ENTRY_LINK_CLASS =
		'font-medium text-[var(--accent)] underline underline-offset-4 hover:text-[var(--accent-bright)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]';
	export const PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS =
		'bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))] text-[var(--text-inverted)] ' +
		'hover:bg-[linear-gradient(90deg,var(--accent-dark),var(--accent-bright))]';
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
		class="relative w-full max-w-2xl rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)]/95 shadow-lg backdrop-blur"
		data-ui="public-entry-card"
	>
		<header class="space-y-2 border-b border-[var(--border-subtle)] px-6 py-5 sm:px-8" data-ui="public-entry-header">
			<p class="text-xs font-semibold uppercase tracking-[0.24em] text-[var(--text-muted)]">{eyebrow}</p>
			<h1 class="text-2xl font-semibold tracking-tight text-[var(--text-primary)]">{title}</h1>
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
