<script lang="ts">
	import Callout from '$lib/components/ui/Callout.svelte';
	import EmptyState from '$lib/components/ui/EmptyState.svelte';
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
	const resolvedActions = $derived(
		actionIds
			.map((actionId) => interactionMap.get(actionId))
			.filter((interaction): interaction is InteractionDescriptor => Boolean(interaction))
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
	<EmptyState title="No actions available" />
{:else if resolvedActions.length === 0}
	<Callout tone="warning" title="Action unavailable" message="This action is not available right now." />
{:else}
	<div class="flex flex-wrap gap-2" data-ui="surface-action-bar">
		{#each resolvedActions as interaction (interaction.interaction_id)}
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
		{/each}
	</div>
{/if}
