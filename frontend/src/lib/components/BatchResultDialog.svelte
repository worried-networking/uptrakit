<script lang="ts">
	import Modal from './Modal.svelte';
	import Button from './Button.svelte';
	import type { BatchActionResponse } from '$lib/types';

	let {
		title,
		response,
		onclose
	}: {
		title: string;
		response: BatchActionResponse;
		onclose: () => void;
	} = $props();
</script>

<Modal {title} {onclose} maxWidth="max-w-lg">
	<div class="space-y-3">
		{#if response.succeeded.length > 0}
			<p class="text-sm">
				<span class="font-medium text-success-500">{response.succeeded.length}</span> item{response.succeeded.length ===
				1
					? ''
					: 's'} succeeded.
			</p>
		{/if}

		{#if response.failed.length > 0}
			<p class="text-sm">
				<span class="font-medium text-error-500">{response.failed.length}</span> item{response.failed.length === 1
					? ''
					: 's'} failed:
			</p>
			<ul class="max-h-60 space-y-1 overflow-y-auto text-sm">
				{#each response.failed as failure (failure.id)}
					<li class="rounded bg-surface-100 px-3 py-2 dark:bg-surface-800">
						<code class="text-xs text-surface-500">{failure.id}</code>
						<p class="text-error-500">{failure.error}</p>
					</li>
				{/each}
			</ul>
		{/if}
	</div>

	{#snippet footer()}
		<Button variant="primary" onclick={onclose}>Close</Button>
	{/snippet}
</Modal>
