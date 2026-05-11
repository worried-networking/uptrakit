<script lang="ts">
	import type { Snippet } from 'svelte';
	import { getFormLayout, LABEL_COL } from './form-layout-context';

	const labelColClass = LABEL_COL[getFormLayout()];

	let {
		label,
		hint,
		value,
		mono = false,
		children
	}: {
		label: string;
		hint?: string;
		value?: string;
		mono?: boolean;
		children?: Snippet;
	} = $props();
</script>

<div class="grid gap-3 md:items-start {labelColClass}" data-ui="form-field-read-only">
	<div class="space-y-1">
		<p class="text-sm font-medium text-[var(--text-primary)]">{label}</p>
		{#if hint}
			<p class="text-xs text-[var(--text-muted)]">{hint}</p>
		{/if}
	</div>

	<div class="space-y-2">
		<div>
			{#if children}
				{@render children()}
			{:else}
				<p class="text-sm text-[var(--text-primary)] {mono ? 'font-mono' : ''}">{value}</p>
			{/if}
		</div>
	</div>
</div>
