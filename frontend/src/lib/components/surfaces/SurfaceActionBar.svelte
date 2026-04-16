<script lang="ts">
	import SurfaceInteractionButton from './SurfaceInteractionButton.svelte';
	import type { SurfaceEncryptionContext } from '$lib/surfaces/interactions';
	import type { InteractionDescriptor, InteractionId } from '$lib/surfaces/contract';

	let {
		surfaceId,
		actionIds = [],
		interactions = [],
		targetProviderId,
		encryptionContext,
		baseParams = {}
	}: {
		surfaceId: string;
		actionIds?: InteractionId[];
		interactions?: InteractionDescriptor[];
		targetProviderId?: string;
		encryptionContext?: SurfaceEncryptionContext;
		baseParams?: Record<string, unknown>;
	} = $props();

	const interactionMap = $derived(
		new Map(interactions.map((interaction) => [interaction.interaction_id, interaction]))
	);

	function notifySurfaceReload(): void {
		if (typeof window === 'undefined') {
			return;
		}
		window.dispatchEvent(
			new CustomEvent('surface:reload', {
				detail: {
					surfaceId,
					targetProviderId: targetProviderId ?? null
				}
			})
		);
	}
</script>

{#if actionIds.length === 0}
	<p class="text-sm text-surface-500">No actions available.</p>
{:else}
	<div class="flex flex-wrap gap-2">
		{#each actionIds as actionId (actionId)}
			{@const interaction = interactionMap.get(actionId)}
			{#if interaction}
				<SurfaceInteractionButton
					{surfaceId}
					{interaction}
					{interactions}
					{targetProviderId}
					{encryptionContext}
					{baseParams}
					oncomplete={async () => {
						notifySurfaceReload();
					}}
				/>
			{/if}
		{/each}
	</div>
{/if}
