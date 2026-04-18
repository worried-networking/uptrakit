<script lang="ts">
	export type TabStripItem = {
		id: string;
		label: string;
		panelId?: string;
		tabId?: string;
		disabled?: boolean;
	};

	let {
		items = [],
		activeId,
		ariaLabel = 'Tabs',
		idBase,
		onSelect
	}: {
		items: TabStripItem[];
		activeId?: string;
		ariaLabel?: string;
		idBase?: string;
		onSelect?: (id: string) => void;
	} = $props();

	let tabRefs: Array<HTMLButtonElement | undefined> = $state([]);

	function isActive(id: string): boolean {
		return id === activeId;
	}

	function isTabbable(item: TabStripItem, index: number): boolean {
		if (isActive(item.id)) {
			return true;
		}
		if (activeId !== undefined) {
			return false;
		}
		return getEnabledItems()[0]?.id === items[index]?.id;
	}

	function getEnabledItems(): TabStripItem[] {
		return items.filter((item) => !item.disabled);
	}

	function resolveTabId(item: TabStripItem): string | undefined {
		return item.tabId ?? (idBase ? `${idBase}-tab-${item.id}` : undefined);
	}

	function resolvePanelId(item: TabStripItem): string | undefined {
		return item.panelId ?? (idBase ? `${idBase}-panel-${item.id}` : undefined);
	}

	function focusTab(itemId: string): void {
		const index = items.findIndex((item) => item.id === itemId);
		tabRefs[index]?.focus();
	}

	function selectAndFocus(item: TabStripItem): void {
		onSelect?.(item.id);
		focusTab(item.id);
	}

	function handleKeydown(event: KeyboardEvent, item: TabStripItem): void {
		const enabledItems = getEnabledItems();
		if (enabledItems.length === 0) {
			return;
		}

		const currentIndex = enabledItems.findIndex((candidate) => candidate.id === item.id);
		if (currentIndex < 0) {
			return;
		}

		switch (event.key) {
			case 'ArrowRight':
			case 'ArrowDown': {
				event.preventDefault();
				selectAndFocus(enabledItems[(currentIndex + 1) % enabledItems.length]);
				break;
			}
			case 'ArrowLeft':
			case 'ArrowUp': {
				event.preventDefault();
				selectAndFocus(enabledItems[(currentIndex - 1 + enabledItems.length) % enabledItems.length]);
				break;
			}
			case 'Home': {
				event.preventDefault();
				selectAndFocus(enabledItems[0]);
				break;
			}
			case 'End': {
				event.preventDefault();
				selectAndFocus(enabledItems[enabledItems.length - 1]);
				break;
			}
		}
	}
</script>

<div
	class="flex flex-wrap gap-2 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-1"
	data-ui="tab-strip"
	role="tablist"
	aria-label={ariaLabel}
>
	{#each items as item, index (item.id)}
		<button
			type="button"
			role="tab"
			id={resolveTabId(item)}
			class="rounded-lg px-3 py-2 text-sm font-medium transition-[background,border-color,color] duration-[120ms] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-40 {isActive(
				item.id
			)
				? 'bg-[var(--accent)] text-[var(--text-inverted)] shadow-sm'
				: 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'}"
			aria-selected={isActive(item.id)}
			aria-controls={resolvePanelId(item)}
			data-state={isActive(item.id) ? 'active' : 'inactive'}
			tabindex={isTabbable(item, index) ? 0 : -1}
			bind:this={tabRefs[index]}
			disabled={item.disabled}
			onclick={() => onSelect?.(item.id)}
			onkeydown={(event) => handleKeydown(event, item)}
		>
			{item.label}
		</button>
	{/each}
</div>
