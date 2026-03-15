<script lang="ts">
	import { fly } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';

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
	let moreMenuEl: HTMLElement | undefined = $state();
	let focusedIndex = $state(-1);

	function getMenuItems(): HTMLElement[] {
		if (!moreMenuEl) return [];
		return Array.from(moreMenuEl.querySelectorAll<HTMLElement>('[role="menuitem"]'));
	}

	function focusItem(index: number) {
		const items = getMenuItems();
		if (!items.length) return;
		focusedIndex = Math.max(0, Math.min(index, items.length - 1));
		items[focusedIndex]?.focus();
	}

	function handleMenuKeydown(event: KeyboardEvent) {
		const items = getMenuItems();
		switch (event.key) {
			case 'ArrowDown':
				event.preventDefault();
				focusItem(focusedIndex < items.length - 1 ? focusedIndex + 1 : 0);
				break;
			case 'ArrowUp':
				event.preventDefault();
				focusItem(focusedIndex > 0 ? focusedIndex - 1 : items.length - 1);
				break;
			case 'Home':
				event.preventDefault();
				focusItem(0);
				break;
			case 'End':
				event.preventDefault();
				focusItem(items.length - 1);
				break;
			case 'Enter':
			case ' ':
				event.preventDefault();
				items[focusedIndex]?.click();
				break;
			case 'Escape':
				event.preventDefault();
				closeMoreMenu();
				break;
		}
	}

	// Auto-focus first menu item when dropdown opens
	$effect(() => {
		if (showMoreMenu && moreMenuEl) {
			focusedIndex = 0;
			getMenuItems()[0]?.focus();
		}
	});

	function closeMoreMenu() {
		showMoreMenu = false;
	}

	function toggleMoreMenu(e: MouseEvent) {
		e.stopPropagation();
		showMoreMenu = !showMoreMenu;
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
				<div class="relative">
					<button
						class="btn btn-sm preset-tonal-surface"
						onclick={toggleMoreMenu}
						aria-label="More actions"
						aria-haspopup="menu"
						aria-expanded={showMoreMenu}
					>
						&hellip; More
					</button>

					{#if showMoreMenu}
						<div
							bind:this={moreMenuEl}
							transition:fly={{ y: 6, duration: 150, easing: cubicOut }}
							class="absolute bottom-full left-0 mb-2 min-w-[10rem] overflow-hidden rounded-lg border border-surface-200 bg-surface-50 p-1 shadow-xl dark:border-surface-700 dark:bg-surface-900"
							role="menu"
							tabindex="-1"
							onkeydown={handleMenuKeydown}
							onclick={(e) => e.stopPropagation()}
						>
							<ul class="space-y-0.5">
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
							</ul>
						</div>
					{/if}
				</div>
			{/if}
			<button class="btn btn-sm preset-tonal-surface" onclick={oncancel}>Deselect all</button>
		</div>
	</div>
{/if}
