<script lang="ts">
	import { Callout, ModalShell, SectionCard, StatusBadge } from '$lib/components/ui';
	import type { StatusBadgeTone } from '$lib/components/ui';

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

	function resolveConfirmTone(cssClass: string): StatusBadgeTone {
		if (cssClass.includes('error') || cssClass.includes('danger')) return 'danger';
		if (cssClass.includes('warning')) return 'warning';
		if (cssClass.includes('success')) return 'success';
		if (cssClass.includes('info') || cssClass.includes('primary')) return 'info';
		return 'neutral';
	}

	const confirmTone = $derived(resolveConfirmTone(confirmClass));
</script>

<ModalShell {title} onclose={oncancel}>
	<SectionCard>
		{#snippet actions()}
			<StatusBadge tone={confirmTone} label="Confirmation required" />
		{/snippet}

		<p>
			{messagePrefix}
			<strong>{entityName}</strong>?
		</p>

		{#if warnings.length > 0}
			<div class="mt-3 space-y-2">
				{#each warnings as warning (warning)}
					<Callout tone="warning" message={warning} />
				{/each}
			</div>
		{/if}
	</SectionCard>

	{#snippet footer()}
		<button class="btn preset-tonal-surface" onclick={oncancel}>Cancel</button>
		<button class="btn {confirmClass}" disabled={confirmDisabled} onclick={onconfirm}>
			{confirmLabel}
		</button>
	{/snippet}
</ModalShell>
