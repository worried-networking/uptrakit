<script lang="ts" module>
	import type { FullAutoFill } from 'svelte/elements';

	export type InputType = 'text' | 'email' | 'password' | 'url' | 'number' | 'search' | 'datetime-local';

	export type InputProps = {
		id: string;
		type: InputType;
		value: string | number;
		name?: string;
		placeholder?: string;
		/** Accepts any autocomplete token string; passed through to the native attribute. */
		autocomplete?: string;
		disabled?: boolean;
		required?: boolean;
		error?: string;
		min?: number | string;
		max?: number | string;
		oninput?: (e: Event) => void;
		onblur?: (e: FocusEvent) => void;
		onkeydown?: (e: KeyboardEvent) => void;
		'aria-describedby'?: string;
		'aria-label'?: string;
		class?: string;
	};
</script>

<script lang="ts">
	import { getContext } from 'svelte';

	const BASE =
		'h-8 w-full py-0 px-[10px] rounded-card ' +
		'bg-[var(--bg-surface)] border border-[var(--border-default)] ' +
		'text-sm text-[var(--text-primary)] ' +
		'placeholder:text-[var(--text-muted)] ' +
		'focus-visible:outline-none ' +
		'focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] ' +
		'disabled:opacity-40 disabled:cursor-not-allowed ' +
		'aria-[invalid=true]:border-[var(--color-danger-border)] ' +
		'aria-[invalid=true]:bg-[var(--color-danger-bg)] ' +
		'transition-[background,border-color] duration-fast';

	let {
		id,
		type,
		value = $bindable(''),
		name,
		placeholder,
		autocomplete,
		disabled = false,
		required = false,
		error,
		min,
		max,
		oninput,
		onblur,
		onkeydown,
		'aria-describedby': ariaDescribedby,
		'aria-label': ariaLabel,
		class: className = ''
	}: InputProps = $props();

	const rowCtx = getContext<{ id: string | undefined } | undefined>('form-field-row:aria-describedby');
	const computedClass = $derived([BASE, className].filter(Boolean).join(' '));
	const hasError = $derived(!!error);
	const resolvedDescribedBy = $derived(ariaDescribedby ?? rowCtx?.id);
</script>

<input
	{id}
	{type}
	bind:value
	{name}
	{placeholder}
	autocomplete={autocomplete as FullAutoFill | undefined}
	{disabled}
	{required}
	{min}
	{max}
	{oninput}
	{onblur}
	{onkeydown}
	aria-invalid={hasError ? 'true' : undefined}
	aria-describedby={resolvedDescribedBy}
	aria-label={ariaLabel}
	class={computedClass}
/>
