<script lang="ts">
	import ModalBackdrop from './ModalBackdrop.svelte';
	import type { Snippet } from 'svelte';

	let {
		onclose,
		title,
		maxWidth = 'max-w-md',
		children,
		footer
	}: {
		onclose: () => void;
		title?: string;
		maxWidth?: string;
		children: Snippet;
		footer?: Snippet;
	} = $props();
</script>

<ModalBackdrop {onclose}>
	<div
		class="card bg-surface-50 dark:bg-surface-900 w-full {maxWidth} space-y-4 p-6 shadow-xl"
		role="dialog"
		aria-modal="true"
		aria-labelledby={title ? 'modal-title' : undefined}
	>
		{#if title}
			<h3 class="h3" id="modal-title">{title}</h3>
		{/if}
		{@render children()}
		{#if footer}
			<div class="flex justify-end gap-2">
				{@render footer()}
			</div>
		{/if}
	</div>
</ModalBackdrop>
