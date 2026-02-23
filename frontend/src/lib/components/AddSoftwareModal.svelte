<script lang="ts">
	import ModalBackdrop from '$lib/components/ModalBackdrop.svelte';
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

<ModalBackdrop {onclose}>
	<div
		class="card bg-surface-50 dark:bg-surface-900 w-full max-w-md space-y-4 p-6 shadow-xl"
		role="dialog"
		aria-modal="true"
		aria-labelledby="add-software-title"
	>
		<div>
			<h3 class="h3" id="add-software-title">Add Software</h3>
			<p class="text-sm text-surface-500">Register a software item to start tracking updates.</p>
		</div>

		<label class="label">
			<span>Name</span>
			<input class="input" bind:value={name} placeholder="Firefox" />
		</label>

		<label class="flex items-center gap-3">
			<input class="checkbox" type="checkbox" bind:checked={enabled} />
			<span>Track updates for this item</span>
		</label>

		<div class="flex justify-end gap-2">
			<button class="btn preset-tonal-surface" onclick={onclose}>Cancel</button>
			<button class="btn preset-filled-primary-500" disabled={submitting} onclick={submit}>
				{submitting ? 'Registering...' : 'Register Software'}
			</button>
		</div>
	</div>
</ModalBackdrop>
