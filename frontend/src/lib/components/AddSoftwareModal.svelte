<script lang="ts">
	import Modal from '$lib/components/Modal.svelte';
	import { createSoftwareItem } from '$lib/api';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import type { SoftwareItemResponse } from '$lib/types';

	let {
		onclose,
		onsuccess
	}: {
		onclose: () => void;
		onsuccess: (created: SoftwareItemResponse) => void;
	} = $props();

	let name: string = $state('');
	let enabled: boolean = $state(true);
	let submitting: boolean = $state(false);

	async function submit() {
		if (submitting) return;
		const trimmedName = name.trim();
		if (!trimmedName) {
			showError('Name is required.');
			return;
		}
		submitting = true;
		try {
			const created = await createSoftwareItem({ name: trimmedName, enabled });
			showSuccess('Software item registered.');
			onsuccess(created);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to register software item');
		} finally {
			submitting = false;
		}
	}
</script>

<Modal title="Add Software" {onclose}>
	<p class="text-sm text-surface-500">Register a software item to start tracking updates.</p>

	<label class="label">
		<span>Name</span>
		<input class="input" bind:value={name} placeholder="Firefox" />
	</label>

	<label class="flex items-center gap-3">
		<input class="checkbox" type="checkbox" bind:checked={enabled} />
		<span>Track updates for this item</span>
	</label>

	{#snippet footer()}
		<button class="btn preset-tonal-surface" onclick={onclose}>Cancel</button>
		<button class="btn preset-filled-primary-500" disabled={submitting} onclick={submit}>
			{submitting ? 'Registering...' : 'Register Software'}
		</button>
	{/snippet}
</Modal>
