<script lang="ts">
	import { invokeSurfaceInteraction } from '$lib/api';
	import SchemaForm from '$lib/components/surfaces/SchemaForm.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import { buildSurfaceInteractionRequest, type SurfaceEncryptionContext } from '$lib/surfaces/interactions';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import type { SelectOption } from '$lib/types';
	import type { InteractionDescriptor } from '$lib/surfaces/contract';

	let {
		surfaceId,
		interaction,
		interactions = [],
		preLoadInteraction,
		targetProviderId,
		encryptionContext,
		baseParams = {},
		submitLabel = 'Submit',
		oncomplete
	}: {
		surfaceId: string;
		interaction: InteractionDescriptor;
		interactions?: InteractionDescriptor[];
		preLoadInteraction?: InteractionDescriptor;
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
	const schemaFields = $derived(interaction.form_ui?.fields ?? []);
	const actionLabel = $derived(interaction.label?.trim() || submitLabel);
	const effectiveSubmitLabel = $derived(submitLabel?.trim() || 'Submit');
	const confirmLabel = $derived(interaction.confirmation?.confirm_label?.trim() || actionLabel);
	const requestBaseParams = $derived(Object.fromEntries(Object.entries(baseParams).filter(([key]) => key !== '_row')));

	async function submitInteraction(params: Record<string, unknown>) {
		submitting = true;
		try {
			const request = await buildSurfaceInteractionRequest(interaction, params, {
				targetProviderId,
				encryption: encryptionContext
			});
			const result = await invokeSurfaceInteraction(surfaceId, interaction.interaction_id, request);
			showSuccess(`${actionLabel} completed`);
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

	async function loadInitialValues(): Promise<Record<string, unknown>> {
		if (!preLoadInteraction) {
			return {};
		}
		const request = await buildSurfaceInteractionRequest(preLoadInteraction, requestBaseParams, {
			targetProviderId
		});
		const result = await invokeSurfaceInteraction(surfaceId, preLoadInteraction.interaction_id, request);
		if (result && typeof result === 'object' && !Array.isArray(result)) {
			return result as Record<string, unknown>;
		}
		return {};
	}

	async function loadSelectOptions(actionId: string): Promise<SelectOption[]> {
		const loadOptionsInteraction = interactions.find((candidate) => candidate.interaction_id === actionId);
		if (!loadOptionsInteraction) {
			return [];
		}
		const request = await buildSurfaceInteractionRequest(loadOptionsInteraction, requestBaseParams, {
			targetProviderId,
			encryption: encryptionContext
		});
		const result = await invokeSurfaceInteraction(surfaceId, loadOptionsInteraction.interaction_id, request);
		if (!result || typeof result !== 'object' || Array.isArray(result)) {
			return [];
		}
		return ((result as Record<string, unknown>).options as SelectOption[]) ?? [];
	}
</script>

{#if schemaFields.length > 0}
	<SchemaForm
		fields={schemaFields}
		extraParams={baseParams}
		submitLabel={effectiveSubmitLabel}
		loading={submitting}
		loadInitialValues={preLoadInteraction ? loadInitialValues : undefined}
		{loadSelectOptions}
		onsubmit={submitInteraction}
	/>
{:else}
	<form class="space-y-3" onsubmit={handleSubmit}>
		<label class="label">
			<span>JSON Payload</span>
			<textarea class="textarea font-mono text-xs" bind:value={payloadText} rows="6"></textarea>
		</label>
		<button class="btn preset-filled-primary-500" type="submit" disabled={submitting}>
			{submitting ? 'Submitting...' : effectiveSubmitLabel}
		</button>
	</form>
{/if}

{#if showConfirm && interaction.confirmation}
	<ConfirmDialog
		title={interaction.confirmation.title}
		messagePrefix={interaction.confirmation.message}
		entityName={actionLabel}
		{confirmLabel}
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
