<script lang="ts">
	import Modal from './Modal.svelte';

	let {
		title,
		messagePrefix,
		entityName,
		confirmLabel,
		confirmClass = 'preset-filled-error-500',
		confirmDisabled = false,
		warnings = [],
		onconfirm,
		oncancel
	}: {
		title: string;
		messagePrefix: string;
		entityName: string;
		confirmLabel: string;
		confirmClass?: string;
		confirmDisabled?: boolean;
		warnings?: string[];
		onconfirm: () => void;
		oncancel: () => void;
	} = $props();
</script>

<Modal {title} onclose={oncancel}>
	<p>{messagePrefix} <strong>{entityName}</strong>?</p>
	{#if warnings.length > 0}
		<aside class="rounded-lg p-3 preset-filled-warning-500 text-sm space-y-1">
			{#each warnings as warning (warning)}
				<p>{warning}</p>
			{/each}
		</aside>
	{/if}
	{#snippet footer()}
		<button class="btn preset-tonal-surface" onclick={oncancel}>Cancel</button>
		<button class="btn {confirmClass}" disabled={confirmDisabled} onclick={onconfirm}>
			{confirmLabel}
		</button>
	{/snippet}
</Modal>
