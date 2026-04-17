<script lang="ts">
	let {
		label,
		destructive = false,
		disabled = false,
		onclick
	}: {
		label: string;
		destructive?: boolean;
		disabled?: boolean;
		onclick?: (event: MouseEvent) => void;
	} = $props();
</script>

<button
	type="button"
	role="menuitem"
	tabindex="-1"
	class={`flex min-h-8 w-full items-center rounded-[4px] px-3 text-left text-[12px] font-medium leading-[1.2] ${
		destructive ? 'text-[var(--color-error)]' : 'text-[var(--text-primary)]'
	} hover:bg-[var(--bg-raised)] disabled:cursor-default disabled:opacity-50`}
	data-ui="context-menu-item"
	{disabled}
	{onclick}
>
	{label}
</button>
