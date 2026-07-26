<script lang="ts">
	import Callout from '$lib/components/ui/Callout.svelte';
	import EmptyState from '$lib/components/ui/EmptyState.svelte';
	import SurfaceInteractionButton from './SurfaceInteractionButton.svelte';
	import {
		actionRefId,
		actionRefMethod,
		resolveInteraction,
		type SurfaceEncryptionContext
	} from '$lib/surfaces/interactions';
	import type { ActionRef, InteractionDescriptor } from '$lib/surfaces/contract';

	let {
		surfaceId,
		actionIds = [],
		interactions = [],
		targetProviderId,
		encryptionContext,
		baseParams = {},
		requiredContextParam,
		requiredForInteractionIds = []
	}: {
		surfaceId: string;
		actionIds?: ActionRef[];
		interactions?: InteractionDescriptor[];
		targetProviderId?: string;
		encryptionContext?: SurfaceEncryptionContext;
		baseParams?: Record<string, unknown>;
		requiredContextParam?: string;
		requiredForInteractionIds?: string[];
	} = $props();

	const resolvedActions = $derived(
		actionIds
			.map((ref) => resolveInteraction(interactions, actionRefId(ref), actionRefMethod(ref)))
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
	<div class="@container/buttons flex flex-wrap justify-end gap-2" data-ui="surface-action-bar">
		{#each resolvedActions as interaction, idx (`${interaction.interaction_id}:${interaction.http_method}:${idx}`)}
			<SurfaceInteractionButton
				{surfaceId}
				{interaction}
				{interactions}
				{targetProviderId}
				{encryptionContext}
				{baseParams}
				labelDisplay={interaction.icon ? 'auto' : 'always'}
				requiredContextParam={requiredForInteractionIds.includes(interaction.interaction_id)
					? requiredContextParam
					: undefined}
				oncomplete={async () => {
					notifySurfaceReload();
				}}
			/>
		{/each}
	</div>
{/if}
