<script lang="ts">
	import type { ExtensionProviderInfo } from '$lib/types';
	import { listExtensionProviders } from '$lib/api';

	let {
		extensionId,
		selectedServiceId = $bindable(undefined),
		selectedEncryptionKey = $bindable(undefined),
		loaded = $bindable(false)
	}: {
		extensionId: string;
		selectedServiceId: string | undefined;
		selectedEncryptionKey: string | undefined;
		loaded: boolean;
	} = $props();

	let providers: ExtensionProviderInfo[] = $state([]);

	async function load() {
		try {
			providers = await listExtensionProviders(extensionId);
			if (providers.length === 1 && !selectedServiceId) {
				selectedServiceId = providers[0].service_id;
			}
		} catch {
			providers = [];
		} finally {
			loaded = true;
		}
	}

	$effect(() => {
		if (!loaded) void load();
	});

	// Keep the encryption key in sync with the selected service.
	// Reads the current value before writing to satisfy the no-useless-assignment lint rule
	// and to avoid triggering parent re-renders when the key has not changed.
	$effect(() => {
		const next = providers.find((p) => p.service_id === selectedServiceId)?.encryption_public_key;
		if (selectedEncryptionKey !== next) {
			selectedEncryptionKey = next;
		}
	});
</script>

{#if providers.length > 1}
	<select class="select w-auto" bind:value={selectedServiceId}>
		<option value={undefined}>Select service...</option>
		{#each providers as p (p.service_id)}
			<option value={p.service_id}>
				{p.service_label}{p.hostname ? ` (${p.hostname})` : ''}
			</option>
		{/each}
	</select>
{:else if providers.length === 1}
	<span class="text-sm text-surface-500">{providers[0].service_label}</span>
{/if}
