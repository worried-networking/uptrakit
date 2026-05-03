<script lang="ts">
	import { setContext } from 'svelte';
	import type { Snippet } from 'svelte';
	import { getFormLayout, LABEL_COL } from './form-layout-context';

	const labelColClass = LABEL_COL[getFormLayout()];

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

	const errorId = $derived(inputId && error ? `${inputId}-error` : undefined);
	setContext('form-field-row:aria-describedby', {
		get id() {
			return errorId;
		}
	});
</script>

<div class="grid gap-3 md:items-start {labelColClass}" data-ui="form-field-row">
	<div class="space-y-1">
		<div class="flex items-center gap-1">
			<label class="text-sm font-medium text-[var(--text-primary)]" for={inputId}>{label}</label>
			{#if required}
				<span aria-hidden="true" class="text-[var(--color-danger)]">*</span>
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
			<p id={errorId} class="text-sm text-[var(--color-danger)]">{error}</p>
		{/if}
	</div>
</div>
