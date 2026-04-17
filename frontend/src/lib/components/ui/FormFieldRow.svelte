<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		label,
		hint,
		error,
		inputId,
		required = false,
		children
	}: {
		label: string;
		hint?: string;
		error?: string;
		inputId?: string;
		required?: boolean;
		children: Snippet;
	} = $props();
</script>

<div class="grid gap-3 md:grid-cols-[minmax(0,16rem)_minmax(0,1fr)] md:items-start" data-ui="form-field-row">
	<div class="space-y-1">
		<div class="flex items-center gap-1">
			<label class="text-sm font-medium text-[var(--text-primary)]" for={inputId}>{label}</label>
			{#if required}
				<span aria-hidden="true" class="text-[var(--color-error)]">*</span>
			{/if}
		</div>
		{#if hint}
			<p class="text-xs text-[var(--text-muted)]">{hint}</p>
		{/if}
	</div>

	<div class="space-y-2">
		<div>
			{@render children()}
		</div>
		{#if error}
			<p class="text-sm text-[var(--color-error)]">{error}</p>
		{/if}
	</div>
</div>
