<script lang="ts">
	import SurfaceRenderer from './SurfaceRenderer.svelte';
	import { getSurfaceProviders } from '$lib/surfaces/registry.svelte';
	import { buildStaticSurfaceData, isSurfaceReadRenderable } from '$lib/surfaces/read-model';
	import type { SurfaceEncryptionContext } from '$lib/surfaces/interactions';
	import type { SurfaceReadResponse, SurfaceResponse } from '$lib/surfaces/contract';

	let {
		surface,
		read,
		baseParams = {}
	}: {
		surface: SurfaceResponse;
		read?: SurfaceReadResponse;
		baseParams?: Record<string, unknown>;
	} = $props();

	let selectedProviderId = $state<string | undefined>(undefined);
	const descriptorMismatch = $derived(read ? read.descriptor.surface_id !== surface.surface_id : false);
	const descriptor = $derived(read ? read.descriptor : surface);

	const providers = $derived(getSurfaceProviders(descriptor.surface_id));
	const availableProviders = $derived(providers.filter((provider) => provider.availability === 'available'));
	const selectedProvider = $derived.by(() => {
		if (descriptor.targeting !== 'targeted') {
			return undefined;
		}
		return (
			availableProviders.find((provider) => provider.provider_id === selectedProviderId) ??
			availableProviders[0] ??
			providers[0]
		);
	});
	const encryptionContext = $derived.by<SurfaceEncryptionContext | undefined>(() => {
		const metadata = selectedProvider?.encryption_metadata;
		if (!metadata) {
			return undefined;
		}
		return {
			keyId: metadata.key_id,
			algorithm: metadata.algorithm,
			publicKey: metadata.public_key
		};
	});
	const staticData = $derived(read ? buildStaticSurfaceData(read.data_sources) : {});

	$effect(() => {
		if (descriptor.targeting !== 'targeted') {
			selectedProviderId = undefined;
			return;
		}
		if (selectedProviderId && availableProviders.some((provider) => provider.provider_id === selectedProviderId)) {
			return;
		}
		selectedProviderId = availableProviders[0]?.provider_id;
	});
</script>

{#if !read}
	<aside class="rounded-lg p-4 preset-filled-warning-500">Surface contract is not available yet.</aside>
{:else if descriptorMismatch}
	<aside class="rounded-lg p-4 preset-filled-warning-500">
		Surface contract mismatch detected. Please refresh and try again.
	</aside>
{:else if !isSurfaceReadRenderable(read)}
	<aside class="rounded-lg p-4 preset-filled-warning-500">
		This surface uses unsupported data sources and cannot be rendered yet.
	</aside>
{:else if descriptor.targeting === 'targeted'}
	{#if availableProviders.length === 0}
		<p class="py-8 text-center text-surface-500">No compatible provider is currently connected for this surface.</p>
	{:else}
		<div class="mb-4">
			<label class="label">
				<span>Provider</span>
				<select class="select" bind:value={selectedProviderId}>
					{#each availableProviders as provider (provider.provider_id)}
						<option value={provider.provider_id}>{provider.display_label}</option>
					{/each}
				</select>
			</label>
		</div>
		<SurfaceRenderer
			surfaceId={descriptor.surface_id}
			node={descriptor.root_node}
			interactions={read.interactions}
			targetProviderId={selectedProvider?.provider_id}
			{encryptionContext}
			dataBySource={staticData}
			{baseParams}
		/>
	{/if}
{:else}
	<SurfaceRenderer
		surfaceId={descriptor.surface_id}
		node={descriptor.root_node}
		interactions={read.interactions}
		dataBySource={staticData}
		{baseParams}
	/>
{/if}
