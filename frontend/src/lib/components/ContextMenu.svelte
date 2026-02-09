<script lang="ts">
	import { onMount } from 'svelte';
	import type { Snippet } from 'svelte';

	let {
		top,
		left,
		children
	}: {
		top: number;
		left: number;
		children: Snippet;
	} = $props();

	let menuEl: HTMLDivElement = $state(undefined!);
	let adjustedTop: number = $state(0);
	let adjustedLeft: number = $state(0);
	let visible: boolean = $state(false);

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
	});
</script>

<div
	bind:this={menuEl}
	class="card fixed z-50 w-40 overflow-hidden bg-surface-50 dark:bg-surface-900 p-0 shadow-xl"
	class:invisible={!visible}
	style="top: {adjustedTop}px; left: {adjustedLeft}px;"
	role="menu"
>
	<nav>
		<ul class="space-y-0.5 p-1">
			{@render children()}
		</ul>
	</nav>
</div>
