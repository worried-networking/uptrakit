<script lang="ts">
	import { Callout, ModalShell, SectionCard, StatusBadge } from '$lib/components/ui';
	import Button from './Button.svelte';

	let {
		title,
		messagePrefix,
		entityName,
		confirmLabel,
		confirmVariant = 'danger' as 'primary' | 'danger',
		confirmDisabled = false,
		warnings = [],
		onconfirm,
		oncancel
	}: {
		title: string;
		messagePrefix: string;
		entityName: string;
		confirmLabel: string;
		confirmVariant?: 'primary' | 'danger';
		confirmDisabled?: boolean;
		warnings?: string[];
		onconfirm: () => void;
		oncancel: () => void;
	} = $props();

	const confirmTone = $derived(confirmVariant === 'danger' ? 'danger' : 'info');
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
		<Button variant="secondary" onclick={oncancel}>Cancel</Button>
		<Button variant={confirmVariant} disabled={confirmDisabled} onclick={onconfirm}>
			{confirmLabel}
		</Button>
	{/snippet}
</ModalShell>
