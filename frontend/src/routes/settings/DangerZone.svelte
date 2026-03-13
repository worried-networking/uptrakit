<script lang="ts">
	import { resetData } from '$lib/api';
	import type { ResetDeletedCounts } from '$lib/types';
	import { getIsOnline } from '$lib/stores/network.svelte';
	import Modal from '$lib/components/Modal.svelte';

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

<div class="card mb-6 border-2 border-error-500 p-6">
	<h2 class="h3 mb-2 text-error-500">Danger Zone</h2>
	<p class="mb-4 text-surface-600 dark:text-surface-400">
		Permanently delete all hosts, software items, plugin configurations, host tags, and update history. This action
		cannot be undone. Services, users, enrollment tokens, and settings are preserved.
	</p>
	<button class="btn preset-filled-error-500" onclick={openDialog} disabled={!getIsOnline()}> Reset Data </button>
	{#if !getIsOnline()}
		<span class="text-warning-500 text-sm ml-2">Offline</span>
	{/if}
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
				<button class="btn preset-tonal-surface" onclick={closeDialog}>Close</button>
			{:else}
				<button class="btn preset-tonal-surface" onclick={closeDialog} disabled={submitting}> Cancel </button>
				<button class="btn preset-filled-error-500" disabled={!isConfirmed || submitting} onclick={handleReset}>
					{#if submitting}
						Resetting...
					{:else}
						Reset All Data
					{/if}
				</button>
			{/if}
		{/snippet}
	</Modal>
{/if}
