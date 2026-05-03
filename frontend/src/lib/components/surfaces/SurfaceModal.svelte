<script lang="ts">
	import type { Snippet } from 'svelte';
	import Modal from '$lib/components/Modal.svelte';

	let {
		open = false,
		title,
		subtitle,
		maxWidth = 'max-w-md',
		onclose,
		children
	}: {
		open?: boolean;
		title?: string;
		subtitle?: Snippet;
		maxWidth?: string;
		onclose: () => void;
		children: Snippet;
	} = $props();
</script>

{#if open}
	<Modal {title} {subtitle} {maxWidth} {onclose}>
		{@render children()}
	</Modal>
{/if}
