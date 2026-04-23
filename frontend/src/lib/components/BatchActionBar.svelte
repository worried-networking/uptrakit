<script lang="ts">
	import { fly } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import ContextMenuItem from './ui/ContextMenuItem.svelte';
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
		class="fixed bottom-4 left-1/2 z-50 -translate-x-1/2 rounded-card bg-[var(--bg-surface)] px-4 py-3 shadow-xl border border-[var(--border-default)]"
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
			<span class="text-sm font-medium whitespace-nowrap">
				{selectedCount} selected
			</span>
			{#each primaryActions as action (action.id)}
				<Button
					variant={action.variant ?? (action.destructive ? 'danger' : 'primary')}
					size="sm"
					loading={action.loading}
					onclick={() => onaction(action.id)}>{action.label}</Button
				>
			{/each}
			{#if secondaryActions.length > 0}
				<div class="relative">
					<Button
						variant="secondary"
						size="sm"
						ariaLabel="More actions"
						aria-haspopup="menu"
						aria-expanded={showMoreMenu}
						onclick={toggleMoreMenu}>&hellip; More</Button
					>

					{#if showMoreMenu}
						<div
							bind:this={moreMenuEl}
							transition:fly={{ y: 6, duration: 150, easing: cubicOut }}
							class="absolute bottom-full left-0 mb-2 min-w-[10rem] overflow-hidden rounded-card border border-[var(--border-default)] bg-[var(--bg-surface)] p-1 shadow-xl"
							role="menu"
							tabindex="-1"
							onkeydown={handleMenuKeydown}
							onclick={(e) => e.stopPropagation()}
						>
							<ul class="space-y-0.5">
								{#each secondaryActions as action (action.id)}
									<li>
										<ContextMenuItem
											label={action.label}
											destructive={true}
											onclick={() => {
												closeMoreMenu();
												onaction(action.id);
											}}
										/>
									</li>
								{/each}
							</ul>
						</div>
					{/if}
				</div>
			{/if}
			<Button variant="secondary" size="sm" onclick={oncancel}>Deselect all</Button>
		</div>
	</div>
{/if}
