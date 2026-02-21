<script lang="ts">
	import ModalBackdrop from './ModalBackdrop.svelte';

	let {
		title,
		messagePrefix,
		entityName,
		confirmLabel,
		confirmClass = 'preset-filled-error-500',
		confirmDisabled = false,
		onconfirm,
		oncancel
	}: {
		title: string;
		messagePrefix: string;
		entityName: string;
		confirmLabel: string;
		confirmClass?: string;
		confirmDisabled?: boolean;
		onconfirm: () => void;
		oncancel: () => void;
	} = $props();
</script>

<ModalBackdrop onclose={oncancel}>
	<div
		class="card bg-surface-50 dark:bg-surface-900 w-full max-w-md space-y-4 p-6 shadow-xl"
		role="dialog"
		aria-modal="true"
	>
		<h3 class="h3">{title}</h3>
		<p>{messagePrefix} <strong>{entityName}</strong>?</p>
		<div class="flex justify-end gap-2">
			<button class="btn preset-tonal-surface" onclick={oncancel}>Cancel</button>
			<button class="btn {confirmClass}" disabled={confirmDisabled} onclick={onconfirm}>
				{confirmLabel}
			</button>
		</div>
	</div>
</ModalBackdrop>
