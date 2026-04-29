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
		'h-4 w-4 rounded-badge ' +
		'border border-[var(--border-default)] ' +
		'text-[var(--accent)] ' +
		'focus-visible:outline-none ' +
		'focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] ' +
		'disabled:opacity-40 disabled:cursor-not-allowed';

	let {
		id,
		checked = $bindable(false),
		indeterminate = false,
		name,
		disabled = false,
		onchange,
		class: className = '',
		'aria-label': ariaLabel
	}: CheckboxProps = $props();

	const computedClass = $derived([BASE, className].filter(Boolean).join(' '));

	function syncIndeterminate(node: HTMLInputElement, value: boolean) {
		node.indeterminate = value;
		return {
			update(v: boolean) {
				node.indeterminate = v;
			}
		};
	}
</script>

<input
	type="checkbox"
	{id}
	bind:checked
	use:syncIndeterminate={indeterminate}
	{name}
	{disabled}
	{onchange}
	class={computedClass}
	aria-label={ariaLabel}
/>
