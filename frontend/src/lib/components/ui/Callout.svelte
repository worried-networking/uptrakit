<script lang="ts">
	import type { Snippet } from 'svelte';
	import { Info, CircleCheck, TriangleAlert, OctagonAlert } from 'lucide-svelte';

	export type CalloutTone = 'info' | 'success' | 'warning' | 'danger';

	const toneClasses: Record<CalloutTone, string> = {
		info: 'border-[var(--color-info-border)] bg-[var(--color-info-bg)] text-[var(--color-info)]',
		success: 'border-[var(--color-success-border)] bg-[var(--color-success-bg)] text-[var(--color-success)]',
		warning: 'border-[var(--color-warning-border)] bg-[var(--color-warning-bg)] text-[var(--color-warning)]',
		danger: 'border-[var(--color-danger-border)] bg-[var(--color-danger-bg)] text-[var(--color-danger)]'
	};

	const toneIcons = {
		info: Info,
		success: CircleCheck,
		warning: TriangleAlert,
		danger: OctagonAlert
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

	const ToneIcon = $derived(toneIcons[tone]);
	const role = $derived(tone === 'danger' || tone === 'warning' ? 'alert' : 'status');
</script>

<aside class={`rounded-panel border px-4 py-3 text-sm ${toneClasses[tone]}`} data-ui="callout" data-tone={tone} {role}>
	<div class="flex items-start gap-3">
		<ToneIcon size={16} class="mt-0.5 shrink-0" aria-hidden="true" />
		<div class="flex-1 min-w-0 space-y-1">
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
	</div>
</aside>
