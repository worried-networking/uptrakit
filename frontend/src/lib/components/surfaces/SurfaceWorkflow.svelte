<script lang="ts">
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import { invokeSurfaceInteraction } from '$lib/api';
	import { buildSurfaceInteractionRequest, type SurfaceEncryptionContext } from '$lib/surfaces/interactions';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import type { InteractionDescriptor } from '$lib/surfaces/contract';

	let {
		surfaceId,
		interaction,
		targetProviderId,
		encryptionContext,
		baseParams = {}
	}: {
		surfaceId: string;
		interaction: InteractionDescriptor;
		targetProviderId?: string;
		encryptionContext?: SurfaceEncryptionContext;
		baseParams?: Record<string, unknown>;
	} = $props();

	let running = $state(false);
	let showConfirm = $state(false);

	async function invokeWorkflow(): Promise<void> {
		running = true;
		try {
			const request = await buildSurfaceInteractionRequest(interaction, baseParams, {
				targetProviderId,
				encryption: encryptionContext
			});
			await invokeSurfaceInteraction(surfaceId, interaction.interaction_id, request);
			showSuccess(`${interaction.interaction_id} completed`);
		} catch (error) {
			showError(error instanceof Error ? error.message : 'Workflow failed');
		} finally {
			running = false;
		}
	}

	function startWorkflow(): void {
		if (interaction.confirmation) {
			showConfirm = true;
			return;
		}
		void invokeWorkflow();
	}
</script>

<div class="space-y-3">
	{#if interaction.workflow_steps && interaction.workflow_steps.length > 0}
		<ol class="list-decimal space-y-1 pl-5 text-sm text-surface-600 dark:text-surface-300">
			{#each interaction.workflow_steps as step (step.step_id)}
				<li>{step.step_id}</li>
			{/each}
		</ol>
	{/if}
	<button class="btn preset-filled-primary-500" type="button" disabled={running} onclick={startWorkflow}>
		{running ? 'Running...' : 'Run Workflow'}
	</button>
</div>

{#if showConfirm && interaction.confirmation}
	<ConfirmDialog
		title={interaction.confirmation.title}
		messagePrefix={interaction.confirmation.message}
		entityName={interaction.interaction_id}
		confirmLabel={interaction.confirmation.confirm_label ?? 'Confirm'}
		onconfirm={() => {
			showConfirm = false;
			void invokeWorkflow();
		}}
		oncancel={() => {
			showConfirm = false;
		}}
	/>
{/if}
