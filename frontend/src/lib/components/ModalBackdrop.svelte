<script lang="ts">
	import type { Snippet } from 'svelte';

	let { onclose, children }: { onclose: () => void; children: Snippet } = $props();
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
	class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
	role="presentation"
	onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}
	onkeydown={(e) => { if (e.key === 'Escape') onclose(); }}
>
	{@render children()}
</div>
