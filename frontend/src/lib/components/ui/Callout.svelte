<script lang="ts">
	import type { Snippet } from 'svelte';

	export type CalloutTone = 'info' | 'success' | 'warning' | 'danger';

	const toneClasses: Record<CalloutTone, string> = {
		info: 'border-[var(--color-info-border)] bg-[var(--color-info-bg)] text-[var(--color-info)]',
		success: 'border-[var(--color-success-border)] bg-[var(--color-success-bg)] text-[var(--color-success)]',
		warning: 'border-[var(--color-warning-border)] bg-[var(--color-warning-bg)] text-[var(--color-warning)]',
		danger: 'border-[var(--color-error-border)] bg-[var(--color-error-bg)] text-[var(--color-error)]'
	};

	let {
		tone = 'info',
		title,
		message,
		children
	}: {
		tone?: CalloutTone;
		title?: string;
		message?: string;
		children?: Snippet;
	} = $props();

	const role = $derived(tone === 'danger' || tone === 'warning' ? 'alert' : 'status');
</script>

<aside class={`rounded-xl border px-4 py-3 text-sm ${toneClasses[tone]}`} data-ui="callout" data-tone={tone} {role}>
	<div class="space-y-1">
		{#if title}
			<p class="font-semibold">{title}</p>
		{/if}
		{#if message}
			<p class="text-current/90">{message}</p>
		{/if}
		{#if children}
			<div>
				{@render children()}
			</div>
		{/if}
	</div>
</aside>
