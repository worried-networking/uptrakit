<script lang="ts">
	import { resetData } from '$lib/api';
	import type { ResetDeletedCounts } from '$lib/types';
	import { getIsOnline } from '$lib/stores/network.svelte';
	import Modal from '$lib/components/Modal.svelte';
	import { SectionCard } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';

	let {
		onSuccess,
		onError
	}: {
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let showDialog: boolean = $state(false);
	let confirmText: string = $state('');
	let submitting: boolean = $state(false);
	let result: ResetDeletedCounts | null = $state(null);

	const isConfirmed = $derived(confirmText === 'RESET');

	function openDialog() {
		confirmText = '';
		result = null;
		showDialog = true;
	}

	function closeDialog() {
		showDialog = false;
		confirmText = '';
		result = null;
	}

	async function handleReset() {
		if (!isConfirmed) return;
		submitting = true;
		try {
			const response = await resetData({ confirm: confirmText });
			result = response.deleted;
			const total =
				result.hosts +
				result.software_items +
				result.plugin_configs +
				result.host_tags +
				result.update_history +
				result.update_batches;
			onSuccess(`Data reset complete. ${total} records deleted.`);
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to reset data');
			closeDialog();
		} finally {
			submitting = false;
		}
	}
</script>

<div class="danger-zone-wrap mb-6">
	<SectionCard
		title="Danger Zone"
		description="Permanently delete all hosts, software items, plugin configurations, host tags, and update history."
	>
		<p class="mb-4 text-[var(--text-secondary)]">
			This action cannot be undone. Services, users, enrollment tokens, and settings are preserved.
		</p>
		<Button variant="danger" disabled={!getIsOnline()} onclick={openDialog}>Reset Data</Button>
		{#if !getIsOnline()}
			<span class="text-[var(--color-warning)] text-sm ml-2">Offline</span>
		{/if}
	</SectionCard>
</div>

{#if showDialog}
	<Modal title="Reset All Data" onclose={closeDialog}>
		{#if result}
			<p class="mb-4">Data has been reset successfully. The following records were deleted:</p>
			<dl class="space-y-1 text-sm">
				<div class="flex justify-between">
					<dt>Hosts</dt>
					<dd class="font-mono">{result.hosts}</dd>
				</div>
				<div class="flex justify-between">
					<dt>Software Items</dt>
					<dd class="font-mono">{result.software_items}</dd>
				</div>
				<div class="flex justify-between">
					<dt>Plugin Configs</dt>
					<dd class="font-mono">{result.plugin_configs}</dd>
				</div>
				<div class="flex justify-between">
					<dt>Host Tags</dt>
					<dd class="font-mono">{result.host_tags}</dd>
				</div>
				<div class="flex justify-between">
					<dt>Update History</dt>
					<dd class="font-mono">{result.update_history}</dd>
				</div>
				<div class="flex justify-between">
					<dt>Update Batches</dt>
					<dd class="font-mono">{result.update_batches}</dd>
				</div>
			</dl>
		{:else}
			<p class="mb-4">
				This will permanently delete <strong>all</strong> hosts, software items, plugin configurations, host tags, and
				update history. This action
				<strong>cannot be undone</strong>.
			</p>
			<p class="mb-2">
				Type <code class="code">RESET</code> to confirm:
			</p>
			<input
				class="input"
				type="text"
				placeholder="Type RESET to confirm"
				bind:value={confirmText}
				disabled={submitting}
			/>
		{/if}
		{#snippet footer()}
			{#if result}
				<Button variant="secondary" onclick={closeDialog}>Close</Button>
			{:else}
				<Button variant="secondary" disabled={submitting} onclick={closeDialog}>Cancel</Button>
				<Button variant="danger" loading={submitting} disabled={!isConfirmed} onclick={handleReset}
					>Reset All Data</Button
				>
			{/if}
		{/snippet}
	</Modal>
{/if}

<style>
	/* Hazard tape rendered as a thin ring around the card border */
	.danger-zone-wrap {
		background-image: repeating-linear-gradient(
			-45deg,
			var(--color-warning-bg) 0px,
			var(--color-warning-bg) 10px,
			var(--color-warning) 10px,
			var(--color-warning) 20px
		);
		border-radius: var(--radius-panel);
		padding: 3px;
	}

	:global(.dark) .danger-zone-wrap {
		background-image: repeating-linear-gradient(
			-45deg,
			var(--color-warning-bg) 0px,
			var(--color-warning-bg) 10px,
			var(--color-warning) 10px,
			var(--color-warning) 20px
		);
	}

	/* Solid card interior — subtle error tint keeps the danger feel without obscuring text */
	.danger-zone-wrap :global([data-ui='section-card']) {
		background-color: var(--color-danger-bg);
	}

	:global(.dark) .danger-zone-wrap :global([data-ui='section-card']) {
		background-color: var(--color-danger-bg-hover);
	}
</style>
