<script lang="ts" module>
	export type CheckboxProps = {
		id: string;
		checked?: boolean;
		indeterminate?: boolean;
		name?: string;
		disabled?: boolean;
		onchange?: (e: Event) => void;
		class?: string;
		'aria-label'?: string;
	};
</script>

<script lang="ts">
	const BASE =
		'h-4 w-4 rounded-[2px] ' +
		'border border-[var(--border-default)] ' +
		'accent-[var(--accent)] ' +
		'focus-visible:outline-none ' +
		'focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] ' +
		'disabled:opacity-40 disabled:cursor-not-allowed';

	let {
		id,
		checked = $bindable(false),
		indeterminate = $bindable(false),
		name,
		disabled = false,
		onchange,
		class: className = '',
		'aria-label': ariaLabel
	}: CheckboxProps = $props();

	const computedClass = $derived([BASE, className].filter(Boolean).join(' '));
</script>

<input
	type="checkbox"
	{id}
	bind:checked
	bind:indeterminate
	{name}
	{disabled}
	{onchange}
	class={computedClass}
	aria-label={ariaLabel}
/>
