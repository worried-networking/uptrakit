<script lang="ts">
	import Button from './Button.svelte';

	let {
		selectedCount,
		actions,
		onaction,
		oncancel,
		selectAllPages
	}: {
		selectedCount: number;
		actions: {
			id: string;
			label: string;
			destructive?: boolean;
			variant?: 'primary' | 'secondary' | 'danger';
			loading?: boolean;
		}[];
		onaction: (actionId: string) => void;
		oncancel: () => void;
		selectAllPages?: { total: number; loading: boolean; onSelect: () => void };
	} = $props();
</script>

{#if selectedCount > 0}
	<div
		class="fixed bottom-4 left-1/2 -translate-x-1/2 rounded-card border border-[var(--border-default)] bg-[var(--bg-surface)] px-4 py-3 shadow-xl"
		data-ui="batch-action-bar"
		role="toolbar"
		aria-label="Batch actions"
	>
		{#if selectAllPages}
			<div class="mb-2 text-center text-sm text-[var(--text-muted)]">
				{#if selectAllPages.loading}
					<span class="inline-flex items-center gap-1.5">
						<span
							class="inline-block h-3 w-3 animate-spin rounded-full border-2 border-[var(--border-default)] border-t-[var(--accent)]"
						></span>
						Selecting all items&hellip;
					</span>
				{:else}
					<Button variant="ghost" onclick={selectAllPages.onSelect}>
						Select all {selectAllPages.total} items across all pages
					</Button>
				{/if}
			</div>
		{/if}
		<div class="flex items-center gap-3">
			<span class="whitespace-nowrap text-sm font-medium">
				{selectedCount} selected
			</span>
			{#each actions as action (action.id)}
				<Button
					variant={action.variant ?? (action.destructive ? 'danger' : 'primary')}
					size="sm"
					loading={action.loading}
					onclick={() => onaction(action.id)}>{action.label}</Button
				>
			{/each}
			<Button variant="secondary" size="sm" onclick={oncancel}>Deselect all</Button>
		</div>
	</div>
{/if}
