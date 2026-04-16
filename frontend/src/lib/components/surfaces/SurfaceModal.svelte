<script lang="ts">
	import type { Snippet } from 'svelte';
	import Modal from '$lib/components/Modal.svelte';

	let {
		open = false,
		title,
		onclose,
		children
	}: {
		open?: boolean;
		title?: string;
		onclose: () => void;
		children: Snippet;
	} = $props();
</script>

{#if open}
	<Modal {title} {onclose}>
		{@render children()}
	</Modal>
{/if}
