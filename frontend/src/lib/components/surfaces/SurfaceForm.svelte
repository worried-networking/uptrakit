<script lang="ts">
	import { invokeSurfaceInteraction } from '$lib/api';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import { buildSurfaceInteractionRequest, type SurfaceEncryptionContext } from '$lib/surfaces/interactions';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import type { InteractionDescriptor } from '$lib/surfaces/contract';

	let {
		surfaceId,
		interaction,
		targetProviderId,
		encryptionContext,
		baseParams = {},
		submitLabel = 'Submit',
		oncomplete
	}: {
		surfaceId: string;
		interaction: InteractionDescriptor;
		targetProviderId?: string;
		encryptionContext?: SurfaceEncryptionContext;
		baseParams?: Record<string, unknown>;
		submitLabel?: string;
		oncomplete?: (result: unknown) => void | Promise<void>;
	} = $props();

	let payloadText = $state('{}');
	let submitting = $state(false);
	let showConfirm = $state(false);
	let pendingParams: Record<string, unknown> | null = $state(null);

	async function submitInteraction(params: Record<string, unknown>) {
		submitting = true;
		try {
			const request = await buildSurfaceInteractionRequest(interaction, params, {
				targetProviderId,
				encryption: encryptionContext
			});
			const result = await invokeSurfaceInteraction(surfaceId, interaction.interaction_id, request);
			showSuccess(`${interaction.interaction_id} completed`);
			await oncomplete?.(result);
		} catch (error) {
			showError(error instanceof Error ? error.message : 'Interaction failed');
		} finally {
			submitting = false;
		}
	}

	function parsePayload(): Record<string, unknown> | null {
		try {
			const parsed = JSON.parse(payloadText);
			if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
				throw new Error('Payload must be a JSON object.');
			}
			return parsed as Record<string, unknown>;
		} catch (error) {
			showError(error instanceof Error ? error.message : 'Invalid JSON payload.');
			return null;
		}
	}

	async function handleSubmit(event: SubmitEvent) {
		event.preventDefault();
		const params = parsePayload();
		if (!params) return;
		const mergedParams = { ...params, ...baseParams };

		if (interaction.confirmation) {
			pendingParams = mergedParams;
			showConfirm = true;
			return;
		}

		await submitInteraction(mergedParams);
	}
</script>

<form class="space-y-3" onsubmit={handleSubmit}>
	<label class="label">
		<span>JSON Payload</span>
		<textarea class="textarea font-mono text-xs" bind:value={payloadText} rows="6"></textarea>
	</label>
	<button class="btn preset-filled-primary-500" type="submit" disabled={submitting}>
		{submitting ? 'Submitting...' : submitLabel}
	</button>
</form>

{#if showConfirm && interaction.confirmation}
	<ConfirmDialog
		title={interaction.confirmation.title}
		messagePrefix={interaction.confirmation.message}
		entityName={interaction.interaction_id}
		confirmLabel={interaction.confirmation.confirm_label ?? 'Confirm'}
		onconfirm={() => {
			showConfirm = false;
			const params = pendingParams;
			pendingParams = null;
			if (params) {
				void submitInteraction(params);
			}
		}}
		oncancel={() => {
			showConfirm = false;
			pendingParams = null;
		}}
	/>
{/if}
