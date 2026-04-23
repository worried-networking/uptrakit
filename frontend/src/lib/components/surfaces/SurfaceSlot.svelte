<script lang="ts">
	import SurfaceRenderer from './SurfaceRenderer.svelte';
	import { getSurfaceDescriptorRenderKey, type SurfaceEncryptionContext } from '$lib/surfaces/interactions';
	import { getSurfacesBySlot } from '$lib/surfaces/registry.svelte';
	import type { InteractionDescriptor, SurfaceResponse } from '$lib/surfaces/contract';

	const STRUCTURAL_SLOTS = new Set(['settings.tabs', 'software.tabs']);

	let {
		slot,
		surfaces,
		interactionsBySurface = {},
		targetProviderIdBySurface = {},
		encryptionBySurface = {},
		dataBySurface = {}
	}: {
		slot: string;
		surfaces?: SurfaceResponse[];
		interactionsBySurface?: Record<string, InteractionDescriptor[]>;
		targetProviderIdBySurface?: Record<string, string>;
		encryptionBySurface?: Record<string, SurfaceEncryptionContext>;
		dataBySurface?: Record<string, Record<string, unknown>>;
	} = $props();

	// Compatibility seam: callers can pass explicit surfaces, otherwise we use the runtime registry slot index.
	const slotSurfaces = $derived(surfaces ?? getSurfacesBySlot(slot));
	const isStructuralSlot = $derived(STRUCTURAL_SLOTS.has(slot));

	function resolveVariantValue<T>(valuesBySurface: Record<string, T>, surface: SurfaceResponse): T | undefined {
		const descriptorKey = getSurfaceDescriptorRenderKey(surface);
		return valuesBySurface[descriptorKey] ?? valuesBySurface[surface.surface_id];
	}
</script>

{#if slotSurfaces.length > 0 || isStructuralSlot}
	<div class="space-y-4" data-ui="surface-slot-container" data-slot={slot}>
		{#each slotSurfaces as surface (getSurfaceDescriptorRenderKey(surface))}
			<section
				class="bg-[var(--bg-surface)] rounded-card border border-[var(--border-subtle)] space-y-4 p-4"
				data-ui="surface-slot-item"
			>
				<h2 class="text-subsection-title font-bold text-[var(--text-primary)]">{surface.label}</h2>
				<SurfaceRenderer
					surfaceId={surface.surface_id}
					node={surface.root_node}
					interactions={resolveVariantValue(interactionsBySurface, surface) ?? []}
					targetProviderId={resolveVariantValue(targetProviderIdBySurface, surface)}
					encryptionContext={resolveVariantValue(encryptionBySurface, surface)}
					dataBySource={resolveVariantValue(dataBySurface, surface) ?? {}}
				/>
			</section>
		{/each}
	</div>
{/if}
