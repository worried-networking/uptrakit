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
		class="card bg-surface-50 dark:bg-surface-900 flex w-full max-h-[calc(100vh-4rem)] flex-col overflow-hidden {maxWidth} shadow-xl"
		role="dialog"
		aria-modal="true"
		aria-labelledby={title ? 'modal-title' : undefined}
	>
		{#if title}
			<div class="flex-shrink-0 px-6 pt-6 pb-2">
				<h3 class="h3" id="modal-title">{title}</h3>
			</div>
		{/if}
		<div class="min-h-0 flex-1 overflow-y-auto px-6 {title ? 'pt-2' : 'pt-6'} {footer ? 'pb-2' : 'pb-6'}">
			{@render children()}
		</div>
		{#if footer}
			<div class="flex flex-shrink-0 justify-end gap-2 px-6 pb-6 pt-2">
				{@render footer()}
			</div>
		{/if}
	</div>
</ModalBackdrop>
