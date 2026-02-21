<script lang="ts">
	import { onMount } from 'svelte';
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

	function handleKeydown(event: KeyboardEvent) {
		const items = getMenuItems();
		if (items.length === 0) return;

		switch (event.key) {
			case 'ArrowDown': {
				event.preventDefault();
				const next = focusedIndex < items.length - 1 ? focusedIndex + 1 : 0;
				focusItem(next);
				break;
			}
			case 'ArrowUp': {
				event.preventDefault();
				const prev = focusedIndex > 0 ? focusedIndex - 1 : items.length - 1;
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
				if (focusedIndex >= 0 && focusedIndex < items.length) {
					items[focusedIndex].click();
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

	onMount(() => {
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
		focusItem(0);
	});
</script>

<div
	bind:this={menuEl}
	class="card fixed z-50 w-40 overflow-hidden bg-surface-50 dark:bg-surface-900 p-0 shadow-xl"
	class:invisible={!visible}
	style="top: {adjustedTop}px; left: {adjustedLeft}px;"
	role="menu"
	tabindex="-1"
	onkeydown={handleKeydown}
>
	<nav>
		<ul class="space-y-0.5 p-1">
			{@render children()}
		</ul>
	</nav>
</div>
