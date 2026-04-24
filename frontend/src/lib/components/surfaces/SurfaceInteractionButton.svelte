<script lang="ts">
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import Callout from '$lib/components/ui/Callout.svelte';
	import Button from '$lib/components/Button.svelte';
	import { invokeSurfaceInteraction } from '$lib/api';
	import SurfaceForm from './SurfaceForm.svelte';
	import SurfaceWorkflow from './SurfaceWorkflow.svelte';
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
		oncomplete,
		requiredContextParam
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
		requiredContextParam?: string;
	} = $props();

	let loading = $state(false);
	let showConfirm = $state(false);
	let showModal = $state(false);

	const actionLabel = $derived(typeof interaction.label === 'string' ? interaction.label.trim() : '');
	const hasActionLabel = $derived(actionLabel.length > 0);
	const confirmLabel = $derived(interaction.confirmation?.confirm_label?.trim() || actionLabel);
	const formBaseParams = $derived(rowSeed ? { ...baseParams, _row: rowSeed } : baseParams);
	const preLoadInteraction = $derived(
		interaction.form_ui?.pre_load_interaction_id
			? interactions.find((candidate) => candidate.interaction_id === interaction.form_ui?.pre_load_interaction_id)
			: undefined
	);
	const hasFormUi = $derived((interaction.form_ui?.fields?.length ?? 0) > 0);
	const isWorkflow = $derived(interaction.kind === 'workflow');
	const isContextGated = $derived(
		!!requiredContextParam && (!baseParams[requiredContextParam] || baseParams[requiredContextParam] === '')
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

{#if !hasActionLabel}
	<Callout tone="warning" title="Action unavailable" message="This action is not available right now." />
{:else if isWorkflow}
	<SurfaceWorkflow
		{surfaceId}
		{interaction}
		{interactions}
		{targetProviderId}
		{encryptionContext}
		baseParams={formBaseParams}
		{size}
		{oncomplete}
	/>
{:else}
	{#if isContextGated}
		<span title="Select a configuration first">
			<Button variant={interaction.confirmation?.severity === 'danger' ? 'danger' : 'primary'} {size} disabled>
				{actionLabel}
			</Button>
		</span>
	{:else}
		<Button
			variant={interaction.confirmation?.severity === 'danger' ? 'danger' : 'primary'}
			{size}
			{loading}
			onclick={requestAction}
		>
			{actionLabel}
		</Button>
	{/if}

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
			{confirmLabel}
			confirmVariant={interaction.confirmation?.severity === 'danger' ? 'danger' : 'primary'}
			onconfirm={() => {
				showConfirm = false;
				void invoke(baseParams);
			}}
			oncancel={() => {
				showConfirm = false;
			}}
		/>
	{/if}
{/if}
