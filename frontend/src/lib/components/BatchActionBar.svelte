<script lang="ts">
	let {
		selectedCount,
		actions,
		onaction,
		oncancel
	}: {
		selectedCount: number;
		actions: { id: string; label: string; destructive?: boolean }[];
		onaction: (actionId: string) => void;
		oncancel: () => void;
	} = $props();
</script>

{#if selectedCount > 0}
	<div
		class="fixed bottom-4 left-1/2 z-50 -translate-x-1/2 rounded-lg bg-surface-50 px-4 py-3 shadow-xl dark:bg-surface-900 border border-surface-200 dark:border-surface-700"
		role="toolbar"
		aria-label="Batch actions"
	>
		<div class="flex items-center gap-3">
			<span class="text-sm font-medium whitespace-nowrap">
				{selectedCount} selected
			</span>
			{#each actions as action (action.id)}
				<button
					class="btn btn-sm {action.destructive ? 'preset-filled-error-500' : 'preset-filled-primary-500'}"
					onclick={() => onaction(action.id)}
				>
					{action.label}
				</button>
			{/each}
			<button class="btn btn-sm preset-tonal-surface" onclick={oncancel}> Deselect all </button>
		</div>
	</div>
{/if}
