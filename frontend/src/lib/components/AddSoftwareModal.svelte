<script lang="ts">
	import Modal from '$lib/components/Modal.svelte';
	import { FormFieldRow, Callout } from '$lib/components/ui';
	import { createSoftwareItem } from '$lib/api';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import { isValidLogoUrl } from '$lib/utils';
	import type { SoftwareItemResponse } from '$lib/types';

	let {
		onclose,
		onsuccess
	}: {
		onclose: () => void;
		onsuccess: (created: SoftwareItemResponse) => void;
	} = $props();

	let name: string = $state('');
	let featured: boolean = $state(true);
	let iconUrl: string = $state('');
	let submitting: boolean = $state(false);
	let nameError: string | null = $state(null);
	let iconUrlError: string | null = $state(null);
	let submitError: string | null = $state(null);

	function validateForm(): boolean {
		const trimmedName = name.trim();
		const trimmedIconUrl = iconUrl.trim();

		nameError = trimmedName ? null : 'Name is required.';
		iconUrlError =
			trimmedIconUrl.length > 0 && !isValidLogoUrl(trimmedIconUrl) ? 'Icon URL must be a valid HTTPS URL.' : null;

		return !nameError && !iconUrlError;
	}

	async function submit() {
		if (submitting) return;
		submitError = null;
		if (!validateForm()) return;

		const trimmedName = name.trim();
		submitting = true;
		try {
			const trimmedIcon = iconUrl.trim() || null;
			const created = await createSoftwareItem({ name: trimmedName, featured, icon_url: trimmedIcon });
			showSuccess('Software item registered.');
			onsuccess(created);
		} catch (e) {
			submitError = e instanceof Error ? e.message : 'Failed to register software item';
			showError(submitError);
		} finally {
			submitting = false;
		}
	}
</script>

<Modal title="Add Software" {onclose}>
	<p class="text-sm text-surface-500">Register a software item to start tracking updates.</p>

	{#if submitError}
		<Callout tone="danger" message={submitError} />
	{/if}

	<FormFieldRow label="Name" inputId="add-software-name" required error={nameError ?? undefined}>
		<input
			id="add-software-name"
			class="input w-full"
			bind:value={name}
			placeholder="Firefox"
			aria-invalid={nameError ? 'true' : 'false'}
			oninput={() => {
				if (nameError && name.trim()) nameError = null;
			}}
		/>
	</FormFieldRow>

	<FormFieldRow
		label="Icon URL"
		hint="Optional. Must be HTTPS when provided."
		inputId="add-software-icon-url"
		error={iconUrlError ?? undefined}
	>
		<input
			id="add-software-icon-url"
			class="input w-full"
			bind:value={iconUrl}
			placeholder="https://example.com/icon.png"
			aria-invalid={iconUrlError ? 'true' : 'false'}
			oninput={() => {
				const trimmed = iconUrl.trim();
				iconUrlError = trimmed.length > 0 && !isValidLogoUrl(trimmed) ? 'Icon URL must be a valid HTTPS URL.' : null;
			}}
		/>
	</FormFieldRow>

	<FormFieldRow label="Tracking" inputId="add-software-featured">
		<label class="flex items-center gap-3">
			<input id="add-software-featured" class="checkbox" type="checkbox" bind:checked={featured} />
			<span>Track updates for this item</span>
		</label>
	</FormFieldRow>

	{#snippet footer()}
		<button class="btn preset-tonal-surface" onclick={onclose}>Cancel</button>
		<button class="btn preset-filled-primary-500" disabled={submitting} onclick={submit}>
			{submitting ? 'Registering...' : 'Register Software'}
		</button>
	{/snippet}
</Modal>
