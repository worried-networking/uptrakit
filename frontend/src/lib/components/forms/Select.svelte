<script lang="ts" module>
	export type SelectOption = { value: string; label: string };

	export type SelectProps = {
		id: string;
		value?: string;
		options: SelectOption[];
		name?: string;
		placeholder?: string;
		disabled?: boolean;
		required?: boolean;
		error?: string;
		onchange?: (e: Event) => void;
		onblur?: (e: FocusEvent) => void;
		'aria-describedby'?: string;
		'aria-label'?: string;
		class?: string;
	};
</script>

<script lang="ts">
	import { getContext } from 'svelte';

	const BASE =
		'h-8 w-full py-0 pl-[10px] pr-10 rounded-card ' +
		'bg-[var(--bg-surface)] border border-[var(--border-default)] ' +
		'text-sm text-[var(--text-primary)] ' +
		'focus-visible:outline-none ' +
		'focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] ' +
		'disabled:opacity-40 disabled:cursor-not-allowed ' +
		'aria-[invalid=true]:border-[var(--color-danger-border)] ' +
		'aria-[invalid=true]:bg-[var(--color-danger-bg)] ' +
		'transition-[background,border-color] duration-fast';

	let {
		id,
		value = $bindable(),
		options,
		name,
		placeholder,
		disabled = false,
		required = false,
		error,
		onchange,
		onblur,
		'aria-describedby': ariaDescribedby,
		'aria-label': ariaLabel,
		class: className = ''
	}: SelectProps = $props();

	const rowCtx = getContext<{ id: string | undefined } | undefined>('form-field-row:aria-describedby');
	const computedClass = $derived([BASE, className].filter(Boolean).join(' '));
	const hasError = $derived(!!error);
	const resolvedDescribedBy = $derived(ariaDescribedby ?? rowCtx?.id);
</script>

<select
	{id}
	bind:value
	{name}
	{disabled}
	{required}
	{onchange}
	{onblur}
	aria-invalid={hasError ? 'true' : undefined}
	aria-describedby={resolvedDescribedBy}
	aria-label={ariaLabel}
	class={computedClass}
>
	{#if placeholder !== undefined}
		<option value="" disabled>{placeholder}</option>
	{/if}
	{#each options as opt (opt.value)}
		<option value={opt.value}>{opt.label}</option>
	{/each}
</select>
