<script lang="ts">
	import type { SvelteSet } from 'svelte/reactivity';

	export type CheckboxListItem = {
		value: string;
		label: string;
		sublabel?: string;
		disabled?: boolean;
		color?: string;
	};

	let {
		items,
		selected,
		maxHeight = 'max-h-48',
		showCounter = true
	}: {
		items: CheckboxListItem[];
		selected: SvelteSet<string>;
		maxHeight?: string;
		showCounter?: boolean;
	} = $props();

	function toggle(item: CheckboxListItem) {
		if (item.disabled) return;
		if (selected.has(item.value)) {
			selected.delete(item.value);
		} else {
			selected.add(item.value);
		}
	}
</script>

<div class="{maxHeight} overflow-y-auto rounded-container-token border border-surface-300-600-token p-2 space-y-1">
	{#each items as item (item.value)}
		<label
			class="flex cursor-pointer items-center gap-2 rounded px-1 py-0.5 {item.disabled
				? 'opacity-50 cursor-not-allowed'
				: 'hover:bg-surface-100-800-token'}"
		>
			<input
				type="checkbox"
				class="checkbox"
				checked={selected.has(item.value)}
				disabled={item.disabled}
				onchange={() => toggle(item)}
			/>
			{#if item.color}
				<span class="inline-block h-3 w-3 rounded-full flex-shrink-0" style="background-color: {item.color}"></span>
			{/if}
			<span class="text-sm font-medium truncate">{item.label}</span>
			{#if item.sublabel}
				<span class="text-xs text-surface-500 truncate">{item.sublabel}</span>
			{/if}
		</label>
	{/each}
</div>
{#if showCounter}
	<p class="mt-1 text-xs text-surface-500">{selected.size} selected</p>
{/if}
