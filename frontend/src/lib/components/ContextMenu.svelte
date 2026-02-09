<script lang="ts">
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
</script>

<div
	class="card fixed z-50 w-40 overflow-hidden p-0 shadow-xl"
	style="top: {top}px; left: {left}px;"
	role="menu"
>
	<nav>
		<ul class="space-y-0.5 p-1">
			{@render children()}
		</ul>
	</nav>
</div>
