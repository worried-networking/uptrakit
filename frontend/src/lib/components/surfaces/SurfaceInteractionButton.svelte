<script lang="ts">
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import { invokeSurfaceInteraction } from '$lib/api';
	import SurfaceForm from './SurfaceForm.svelte';
	import SurfaceModal from './SurfaceModal.svelte';
	import { buildSurfaceInteractionRequest, type SurfaceEncryptionContext } from '$lib/surfaces/interactions';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import type { InteractionDescriptor } from '$lib/surfaces/contract';

	let {
		surfaceId,
		interaction,
		interactions = [],
		targetProviderId,
		encryptionContext,
		baseParams = {},
		rowSeed,
		size = 'md',
		oncomplete
	}: {
		surfaceId: string;
		interaction: InteractionDescriptor;
		interactions?: InteractionDescriptor[];
		targetProviderId?: string;
		encryptionContext?: SurfaceEncryptionContext;
		baseParams?: Record<string, unknown>;
		rowSeed?: Record<string, unknown>;
		size?: 'sm' | 'md';
		oncomplete?: (result: unknown) => void | Promise<void>;
	} = $props();

	let loading = $state(false);
	let showConfirm = $state(false);
	let showModal = $state(false);

	const actionLabel = $derived(interaction.label ?? interaction.interaction_id);
	const formBaseParams = $derived(rowSeed ? { ...baseParams, _row: rowSeed } : baseParams);
	const preLoadInteraction = $derived(
		interaction.form_ui?.pre_load_interaction_id
			? interactions.find((candidate) => candidate.interaction_id === interaction.form_ui?.pre_load_interaction_id)
			: undefined
	);
	const hasFormUi = $derived((interaction.form_ui?.fields?.length ?? 0) > 0);
	const buttonClass = $derived(size === 'sm' ? 'btn btn-sm text-xs' : 'btn');
	const presetClass = $derived(
		interaction.confirmation?.severity === 'danger' ? 'preset-filled-error-500' : 'preset-filled-primary-500'
	);

	async function invoke(params: Record<string, unknown>): Promise<void> {
		loading = true;
		try {
			const request = await buildSurfaceInteractionRequest(interaction, params, {
				targetProviderId,
				encryption: encryptionContext
			});
			const result = await invokeSurfaceInteraction(surfaceId, interaction.interaction_id, request);
			showSuccess(`${actionLabel} completed`);
			showModal = false;
			await oncomplete?.(result);
		} catch (error) {
			showError(error instanceof Error ? error.message : 'Interaction failed');
		} finally {
			loading = false;
		}
	}

	function requestAction(): void {
		if (hasFormUi) {
			showModal = true;
			return;
		}
		if (interaction.confirmation) {
			showConfirm = true;
			return;
		}
		void invoke(baseParams);
	}
</script>

<button type="button" class="{buttonClass} {presetClass}" disabled={loading} onclick={requestAction}>
	{loading ? 'Processing...' : actionLabel}
</button>

{#if showModal}
	<SurfaceModal
		open={showModal}
		title={actionLabel}
		onclose={() => {
			showModal = false;
		}}
	>
		<SurfaceForm
			{surfaceId}
			{interaction}
			{interactions}
			{preLoadInteraction}
			{targetProviderId}
			{encryptionContext}
			baseParams={formBaseParams}
			submitLabel={actionLabel}
			oncomplete={async (result) => {
				await oncomplete?.(result);
				showModal = false;
			}}
		/>
	</SurfaceModal>
{/if}

{#if showConfirm && interaction.confirmation}
	<ConfirmDialog
		title={interaction.confirmation.title}
		messagePrefix={interaction.confirmation.message}
		entityName={actionLabel}
		confirmLabel={interaction.confirmation.confirm_label ?? actionLabel}
		onconfirm={() => {
			showConfirm = false;
			void invoke(baseParams);
		}}
		oncancel={() => {
			showConfirm = false;
		}}
	/>
{/if}
