<script lang="ts">
	import Modal from '$lib/components/Modal.svelte';
	import Button from '$lib/components/Button.svelte';
	import Input from '$lib/components/Input.svelte';
	import Checkbox from '$lib/components/Checkbox.svelte';
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
		<Input
			id="add-software-name"
			type="text"
			bind:value={name}
			placeholder="Firefox"
			error={nameError ?? undefined}
			oninput={() => {
				if (nameError && name.trim()) nameError = null;
			}}
			class="w-full"
		/>
	</FormFieldRow>

	<FormFieldRow
		label="Icon URL"
		hint="Optional. Must be HTTPS when provided."
		inputId="add-software-icon-url"
		error={iconUrlError ?? undefined}
	>
		<Input
			id="add-software-icon-url"
			type="url"
			bind:value={iconUrl}
			placeholder="https://example.com/icon.png"
			error={iconUrlError ?? undefined}
			oninput={() => {
				const trimmed = iconUrl.trim();
				iconUrlError = trimmed.length > 0 && !isValidLogoUrl(trimmed) ? 'Icon URL must be a valid HTTPS URL.' : null;
			}}
			class="w-full"
		/>
	</FormFieldRow>

	<FormFieldRow label="Tracking" inputId="add-software-featured">
		<label class="flex items-center gap-3">
			<Checkbox id="add-software-featured" bind:checked={featured} />
			<span>Track updates for this item</span>
		</label>
	</FormFieldRow>

	{#snippet footer()}
		<Button variant="secondary" onclick={onclose}>Cancel</Button>
		<Button variant="primary" loading={submitting} onclick={submit}>Register Software</Button>
	{/snippet}
</Modal>
