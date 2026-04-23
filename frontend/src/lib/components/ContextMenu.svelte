<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		top,
		left,
		onclose,
		children
	}: {
		top: number;
		left: number;
		onclose?: () => void;
		children: Snippet;
	} = $props();

	let menuEl: HTMLDivElement = $state(undefined!);
	let adjustedTop: number = $state(0);
	let adjustedLeft: number = $state(0);
	let visible: boolean = $state(false);
	let focusedIndex: number = $state(-1);

	function getMenuItems(): HTMLElement[] {
		if (!menuEl) return [];
		return Array.from(menuEl.querySelectorAll<HTMLElement>('[role="menuitem"]'));
	}

	function focusItem(index: number) {
		const items = getMenuItems();
		if (items.length === 0) return;
		focusedIndex = Math.max(0, Math.min(index, items.length - 1));
		items[focusedIndex].focus();
	}

	function getCurrentFocusedIndex(items: HTMLElement[]): number {
		const activeElement = document.activeElement;
		const activeIndex = items.findIndex((item) => item === activeElement);
		if (activeIndex >= 0) {
			focusedIndex = activeIndex;
			return activeIndex;
		}
		return focusedIndex;
	}

	function handleKeydown(event: KeyboardEvent) {
		const items = getMenuItems();
		if (items.length === 0) return;
		const currentIndex = getCurrentFocusedIndex(items);

		switch (event.key) {
			case 'ArrowDown': {
				event.preventDefault();
				const next = currentIndex < items.length - 1 ? currentIndex + 1 : 0;
				focusItem(next);
				break;
			}
			case 'ArrowUp': {
				event.preventDefault();
				const prev = currentIndex > 0 ? currentIndex - 1 : items.length - 1;
				focusItem(prev);
				break;
			}
			case 'Home': {
				event.preventDefault();
				focusItem(0);
				break;
			}
			case 'End': {
				event.preventDefault();
				focusItem(items.length - 1);
				break;
			}
			case 'Enter':
			case ' ': {
				event.preventDefault();
				if (currentIndex >= 0 && currentIndex < items.length) {
					items[currentIndex].click();
				}
				break;
			}
			case 'Escape': {
				event.preventDefault();
				onclose?.();
				break;
			}
		}
	}

	$effect(() => {
		const rect = menuEl.getBoundingClientRect();
		const vw = window.innerWidth;
		const vh = window.innerHeight;
		const pad = 8;

		let newLeft = left;
		let newTop = top;

		if (newLeft + rect.width > vw - pad) {
			newLeft = vw - rect.width - pad;
		}
		if (newLeft < pad) {
			newLeft = pad;
		}
		if (newTop + rect.height > vh - pad) {
			newTop = vh - rect.height - pad;
		}
		if (newTop < pad) {
			newTop = pad;
		}

		adjustedTop = newTop;
		adjustedLeft = newLeft;
		visible = true;

		// Auto-focus first menu item after positioning.
		// Inline the initial focus instead of calling focusItem() to avoid tracking
		// focusedIndex as an effect dependency (focusItem reads it after writing it).
		const initialItems = getMenuItems();
		if (initialItems.length > 0) {
			focusedIndex = 0;
			initialItems[0].focus();
		}
	});
</script>

<div
	bind:this={menuEl}
	class="fixed z-[100] w-44 overflow-hidden rounded-panel border border-[var(--border-default)] bg-[var(--bg-surface)] p-1.5 shadow-xl"
	class:invisible={!visible}
	data-ui="context-menu-shell"
	style="top: {adjustedTop}px; left: {adjustedLeft}px;"
	role="menu"
	tabindex="-1"
	onkeydown={handleKeydown}
	onfocusin={() => {
		focusedIndex = getCurrentFocusedIndex(getMenuItems());
	}}
	onclick={(e) => e.stopPropagation()}
>
	<nav>
		<ul class="space-y-1">
			{@render children()}
		</ul>
	</nav>
</div>
