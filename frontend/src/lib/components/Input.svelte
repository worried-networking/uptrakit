<script lang="ts" module>
	import type { FullAutoFill } from 'svelte/elements';

	export type InputType = 'text' | 'email' | 'password' | 'url' | 'number' | 'search';

	export type InputProps = {
		id: string;
		type: InputType;
		value: string;
		name?: string;
		placeholder?: string;
		/** Accepts any autocomplete token string; passed through to the native attribute. */
		autocomplete?: string;
		disabled?: boolean;
		required?: boolean;
		error?: string;
		oninput?: (e: Event) => void;
		onblur?: (e: FocusEvent) => void;
		'aria-describedby'?: string;
		class?: string;
	};
</script>

<script lang="ts">
	const BASE =
		'h-8 w-full px-[10px] rounded-[3px] ' +
		'bg-[var(--bg-surface)] border border-[var(--border-default)] ' +
		'text-sm text-[var(--text-primary)] ' +
		'placeholder:text-[var(--text-muted)] ' +
		'focus-visible:outline-none ' +
		'focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] ' +
		'disabled:opacity-40 disabled:cursor-not-allowed ' +
		'aria-[invalid=true]:border-[var(--color-error-border)] ' +
		'aria-[invalid=true]:bg-[var(--color-error-bg)] ' +
		'transition-[background,border-color] duration-[0.12s]';

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
		oninput,
		onblur,
		'aria-describedby': ariaDescribedby,
		class: className = ''
	}: InputProps = $props();

	const computedClass = $derived([BASE, className].filter(Boolean).join(' '));
	const hasError = $derived(!!error);
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
	{oninput}
	{onblur}
	aria-invalid={hasError ? 'true' : undefined}
	aria-describedby={ariaDescribedby}
	class={computedClass}
/>
