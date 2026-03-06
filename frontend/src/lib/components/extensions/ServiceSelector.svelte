<script lang="ts">
	import type { ExtensionProviderInfo } from '$lib/types';
	import { listExtensionProviders } from '$lib/api';

	let {
		extensionId,
		selectedServiceId = $bindable(undefined)
	}: {
		extensionId: string;
		selectedServiceId: string | undefined;
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
		}
	}

	$effect(() => {
		void load();
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
