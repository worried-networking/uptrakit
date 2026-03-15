<script lang="ts">
	import ContextMenu from './ContextMenu.svelte';

	let {
		selectedCount,
		actions,
		onaction,
		oncancel,
		selectAllPages
	}: {
		selectedCount: number;
		actions: { id: string; label: string; destructive?: boolean }[];
		onaction: (actionId: string) => void;
		oncancel: () => void;
		selectAllPages?: { total: number; loading: boolean; onSelect: () => void };
	} = $props();

	// If every action is destructive, show them all as primary buttons (no More dropdown).
	const allDestructive = $derived(actions.length > 0 && actions.every((a) => a.destructive));
	const primaryActions = $derived(allDestructive ? actions : actions.filter((a) => !a.destructive));
	const secondaryActions = $derived(allDestructive ? [] : actions.filter((a) => a.destructive));

	let showMoreMenu = $state(false);
	let moreButtonEl: HTMLElement | undefined = $state();
	let moreMenuPos = $state({ top: 0, left: 0 });

	function openMoreMenu() {
		if (!moreButtonEl) return;
		const rect = moreButtonEl.getBoundingClientRect();
		const estimatedHeight = secondaryActions.length * 40 + 16;
		moreMenuPos = { top: rect.top - estimatedHeight - 4, left: rect.left };
		showMoreMenu = true;
	}

	function closeMoreMenu() {
		showMoreMenu = false;
	}

	function toggleMoreMenu(e: MouseEvent) {
		e.stopPropagation();
		if (showMoreMenu) {
			closeMoreMenu();
		} else {
			openMoreMenu();
		}
	}

	function handleWindowClick() {
		if (showMoreMenu) closeMoreMenu();
	}
</script>

<svelte:window onclick={handleWindowClick} />

{#if selectedCount > 0}
	<div
		class="fixed bottom-4 left-1/2 z-50 -translate-x-1/2 rounded-lg bg-surface-50 px-4 py-3 shadow-xl dark:bg-surface-900 border border-surface-200 dark:border-surface-700"
		role="toolbar"
		aria-label="Batch actions"
	>
		{#if selectAllPages}
			<div class="mb-2 text-center text-sm text-surface-500">
				{#if selectAllPages.loading}
					<span class="inline-flex items-center gap-1.5">
						<span
							class="inline-block h-3 w-3 animate-spin rounded-full border-2 border-surface-300 border-t-primary-500"
						></span>
						Selecting all items&hellip;
					</span>
				{:else}
					<button
						class="cursor-pointer underline hover:text-surface-700 dark:hover:text-surface-300"
						onclick={selectAllPages.onSelect}
					>
						Select all {selectAllPages.total} items across all pages
					</button>
				{/if}
			</div>
		{/if}
		<div class="flex items-center gap-3">
			<span class="text-sm font-medium whitespace-nowrap">
				{selectedCount} selected
			</span>
			{#each primaryActions as action (action.id)}
				<button class="btn btn-sm preset-filled-primary-500" onclick={() => onaction(action.id)}>
					{action.label}
				</button>
			{/each}
			{#if secondaryActions.length > 0}
				<button
					bind:this={moreButtonEl}
					class="btn btn-sm preset-tonal-surface"
					onclick={toggleMoreMenu}
					aria-label="More actions"
					aria-haspopup="menu"
					aria-expanded={showMoreMenu}
				>
					&hellip; More
				</button>
			{/if}
			<button class="btn btn-sm preset-tonal-surface" onclick={oncancel}>Deselect all</button>
		</div>
	</div>

	{#if showMoreMenu && secondaryActions.length > 0}
		<ContextMenu top={moreMenuPos.top} left={moreMenuPos.left} onclose={closeMoreMenu}>
			{#each secondaryActions as action (action.id)}
				<li>
					<button
						class="w-full rounded-md px-3 py-2 text-left text-sm text-error-500 hover:bg-surface-200 dark:hover:bg-surface-800"
						role="menuitem"
						tabindex="-1"
						onclick={() => {
							closeMoreMenu();
							onaction(action.id);
						}}
					>
						{action.label}
					</button>
				</li>
			{/each}
		</ContextMenu>
	{/if}
{/if}
