<script lang="ts">
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import { invokeSurfaceInteraction } from '$lib/api';
	import { buildSurfaceInteractionRequest, type SurfaceEncryptionContext } from '$lib/surfaces/interactions';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import type { InteractionDescriptor, InteractionId } from '$lib/surfaces/contract';

	let {
		surfaceId,
		actionIds = [],
		interactions = [],
		targetProviderId,
		encryptionContext
	}: {
		surfaceId: string;
		actionIds?: InteractionId[];
		interactions?: InteractionDescriptor[];
		targetProviderId?: string;
		encryptionContext?: SurfaceEncryptionContext;
	} = $props();

	const interactionMap = $derived(
		new Map(interactions.map((interaction) => [interaction.interaction_id, interaction]))
	);
	let activeActionId = $state<string | null>(null);
	let confirmActionId: string | null = $state(null);

	async function invokeAction(action: InteractionDescriptor): Promise<void> {
		const actionId = action.interaction_id;
		activeActionId = actionId;
		try {
			const request = await buildSurfaceInteractionRequest(
				action,
				{},
				{
					targetProviderId,
					encryption: encryptionContext
				}
			);
			await invokeSurfaceInteraction(surfaceId, actionId, request);
			showSuccess(`${actionId} completed`);
		} catch (error) {
			showError(error instanceof Error ? error.message : 'Interaction failed');
		} finally {
			activeActionId = null;
		}
	}

	function actionLabel(actionId: InteractionId): string {
		const interaction = interactionMap.get(actionId);
		return interaction?.interaction_id ?? actionId;
	}

	function requestAction(actionId: InteractionId): void {
		const interaction = interactionMap.get(actionId);
		if (!interaction) {
			return;
		}
		if (interaction.confirmation) {
			confirmActionId = actionId;
			return;
		}
		void invokeAction(interaction);
	}

	const confirmAction = $derived(confirmActionId ? interactionMap.get(confirmActionId) : undefined);
</script>

{#if actionIds.length === 0}
	<p class="text-sm text-surface-500">No actions available.</p>
{:else}
	<div class="flex flex-wrap gap-2">
		{#each actionIds as actionId (actionId)}
			<button
				class="btn preset-filled-primary-500"
				type="button"
				disabled={activeActionId !== null}
				onclick={() => {
					requestAction(actionId);
				}}
			>
				{#if activeActionId === actionId}
					Running...
				{:else}
					{actionLabel(actionId)}
				{/if}
			</button>
		{/each}
	</div>
{/if}

{#if confirmAction?.confirmation}
	<ConfirmDialog
		title={confirmAction.confirmation.title}
		messagePrefix={confirmAction.confirmation.message}
		entityName={confirmAction.interaction_id}
		confirmLabel={confirmAction.confirmation.confirm_label ?? 'Confirm'}
		onconfirm={() => {
			const interaction = confirmAction;
			confirmActionId = null;
			if (interaction) {
				void invokeAction(interaction);
			}
		}}
		oncancel={() => {
			confirmActionId = null;
		}}
	/>
{/if}
