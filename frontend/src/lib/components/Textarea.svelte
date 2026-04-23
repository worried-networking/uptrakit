<script lang="ts" module>
	export type TextareaVariant = 'default' | 'mono';

	export type TextareaProps = {
		id: string;
		value: string;
		name?: string;
		placeholder?: string;
		rows?: number;
		disabled?: boolean;
		required?: boolean;
		error?: string;
		variant?: TextareaVariant;
		oninput?: (e: Event) => void;
		onblur?: (e: FocusEvent) => void;
		'aria-describedby'?: string;
		class?: string;
	};
</script>

<script lang="ts">
	const BASE =
		'w-full rounded-card p-2 ' +
		'bg-[var(--bg-surface)] border border-[var(--border-default)] ' +
		'text-sm text-[var(--text-primary)] ' +
		'placeholder:text-[var(--text-muted)] ' +
		'focus-visible:outline-none ' +
		'focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] ' +
		'disabled:opacity-40 disabled:cursor-not-allowed ' +
		'aria-[invalid=true]:border-[var(--color-danger-border)] ' +
		'aria-[invalid=true]:bg-[var(--color-danger-bg)] ' +
		'transition-[background,border-color] duration-fast ' +
		'resize-y min-h-[4rem]';

	const VARIANT_CLASSES: Record<TextareaVariant, string> = {
		default: '',
		mono: 'font-mono text-subsection-title'
	};

	let {
		id,
		value = $bindable(''),
		name,
		placeholder,
		rows,
		disabled = false,
		required = false,
		error,
		variant = 'default',
		oninput,
		onblur,
		'aria-describedby': ariaDescribedby,
		class: className = ''
	}: TextareaProps = $props();

	const computedClass = $derived([BASE, VARIANT_CLASSES[variant], className].filter(Boolean).join(' '));
	const hasError = $derived(!!error);
</script>

<textarea
	{id}
	bind:value
	{name}
	{placeholder}
	{rows}
	{disabled}
	{required}
	{oninput}
	{onblur}
	aria-invalid={hasError ? 'true' : undefined}
	aria-describedby={ariaDescribedby}
	class={computedClass}
></textarea>
