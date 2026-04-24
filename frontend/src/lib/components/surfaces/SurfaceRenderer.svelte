<script lang="ts">
	import SurfaceActionBar from './SurfaceActionBar.svelte';
	import SurfaceForm from './SurfaceForm.svelte';
	import SurfaceKeyValue from './SurfaceKeyValue.svelte';
	import SurfaceModal from './SurfaceModal.svelte';
	import SurfaceRenderer from './SurfaceRenderer.svelte';
	import SurfaceTable from './SurfaceTable.svelte';
	import SurfaceWorkflow from './SurfaceWorkflow.svelte';
	import Button from '$lib/components/Button.svelte';
	import Callout from '$lib/components/ui/Callout.svelte';
	import EmptyState from '$lib/components/ui/EmptyState.svelte';
	import TabStrip from '$lib/components/ui/TabStrip.svelte';
	import { clampSurfaceTabIndex, type SurfaceEncryptionContext } from '$lib/surfaces/interactions';
	import type { DataSourceDescriptor, InteractionDescriptor, SurfaceNode } from '$lib/surfaces/contract';

	let {
		surfaceId,
		node,
		interactions = [],
		dataSources = [],
		targetProviderId,
		encryptionContext,
		dataBySource = {},
		baseParams = {},
		requiredContextParam,
		requiredForInteractionIds = []
	}: {
		surfaceId: string;
		node: SurfaceNode;
		interactions?: InteractionDescriptor[];
		dataSources?: DataSourceDescriptor[];
		targetProviderId?: string;
		encryptionContext?: SurfaceEncryptionContext;
		dataBySource?: Record<string, unknown>;
		baseParams?: Record<string, unknown>;
		requiredContextParam?: string;
		requiredForInteractionIds?: string[];
	} = $props();

	let selectedTab = $state(0);
	let modalOpen = $state(false);

	const selectedTabIndex = $derived(
		node.kind === 'tabs' ? clampSurfaceTabIndex(selectedTab, (node.tabs ?? []).length) : 0
	);

	$effect(() => {
		if (node.kind !== 'tabs') return;
		const next = clampSurfaceTabIndex(selectedTab, (node.tabs ?? []).length);
		if (next !== selectedTab) {
			selectedTab = next;
		}
	});

	function findInteraction(interactionId: string): InteractionDescriptor | undefined {
		return interactions.find((interaction) => interaction.interaction_id === interactionId);
	}

	function findDataSource(dataSourceId: string): DataSourceDescriptor | undefined {
		return dataSources.find((dataSource) => dataSource.data_source_id === dataSourceId);
	}

	function findTableDataLoadInteraction(dataSourceId: string): InteractionDescriptor | undefined {
		const dataSource = findDataSource(dataSourceId);
		if (!dataSource || dataSource.kind.kind !== 'provider_query') {
			return undefined;
		}
		return findInteraction(dataSource.kind.operation_id);
	}

	function calloutTone(level: 'info' | 'warning' | 'danger'): 'info' | 'warning' | 'danger' {
		switch (level) {
			default:
				return level;
		}
	}

	function renderUnavailableAction(message: string) {
		return {
			title: 'Action unavailable',
			message
		};
	}

	function interactionLabel(interaction: InteractionDescriptor): string {
		return typeof interaction.label === 'string' ? interaction.label.trim() : '';
	}
</script>

{#if node.kind === 'section'}
	<div class="space-y-4">
		{#if node.title}
			<h3 class="text-subsection-title font-bold text-[var(--text-primary)]">{node.title}</h3>
		{/if}
		{#each node.children ?? [] as child, idx (idx)}
			<SurfaceRenderer
				{surfaceId}
				node={child}
				{interactions}
				{dataSources}
				{targetProviderId}
				{encryptionContext}
				{dataBySource}
				{baseParams}
			/>
		{/each}
	</div>
{:else if node.kind === 'text_block'}
	<p class="whitespace-pre-wrap text-sm">{node.text}</p>
{:else if node.kind === 'key_value'}
	<SurfaceKeyValue data={(dataBySource[node.data_source_id] as Record<string, unknown>) ?? {}} />
{:else if node.kind === 'table'}
	<SurfaceTable
		{surfaceId}
		{node}
		dataSource={findDataSource(node.data_source_id)}
		dataLoadInteraction={findTableDataLoadInteraction(node.data_source_id)}
		{interactions}
		{targetProviderId}
		{encryptionContext}
		{baseParams}
		rows={(dataBySource[node.data_source_id] as Record<string, unknown>[]) ?? []}
	/>
{:else if node.kind === 'form'}
	{@const interaction = findInteraction(node.interaction_id)}
	{#if interaction}
		<SurfaceForm
			{surfaceId}
			{interaction}
			{interactions}
			preLoadInteraction={interaction.form_ui?.pre_load_interaction_id
				? findInteraction(interaction.form_ui.pre_load_interaction_id)
				: undefined}
			{targetProviderId}
			{encryptionContext}
			{baseParams}
		/>
	{:else}
		{@const unavailable = renderUnavailableAction('This form is not available right now.')}
		<Callout tone="warning" title={unavailable.title} message={unavailable.message} />
	{/if}
{:else if node.kind === 'action_bar'}
	<SurfaceActionBar
		{surfaceId}
		actionIds={node.action_ids ?? []}
		{interactions}
		{targetProviderId}
		{encryptionContext}
		{baseParams}
		{requiredContextParam}
		{requiredForInteractionIds}
	/>
{:else if node.kind === 'tabs'}
	{@const tabs = node.tabs ?? []}
	{#if tabs.length === 0}
		<EmptyState title="No tabs available" />
	{:else}
		<div class="space-y-4">
			<TabStrip
				items={tabs.map((tab) => ({
					id: tab.id,
					label: tab.label
				}))}
				activeId={tabs[selectedTabIndex]?.id}
				ariaLabel="Surface tabs"
				onSelect={(tabId) => {
					const nextIndex = tabs.findIndex((tab) => tab.id === tabId);
					if (nextIndex >= 0) {
						selectedTab = nextIndex;
					}
				}}
			/>
			<SurfaceRenderer
				{surfaceId}
				node={tabs[selectedTabIndex].root}
				{interactions}
				{dataSources}
				{targetProviderId}
				{encryptionContext}
				{dataBySource}
				{baseParams}
			/>
		</div>
	{/if}
{:else if node.kind === 'callout'}
	<Callout tone={calloutTone(node.level)} message={node.text} />
{:else if node.kind === 'empty_state'}
	<EmptyState title={node.title} description={node.description} />
{:else if node.kind === 'modal_trigger'}
	{@const interaction = findInteraction(node.interaction_id)}
	{#if interaction}
		{#if interactionLabel(interaction)}
			<Button variant="secondary" type="button" data-ui="modal-trigger" onclick={() => (modalOpen = true)}>
				{interactionLabel(interaction)}
			</Button>
			<SurfaceModal
				open={modalOpen}
				title={interactionLabel(interaction)}
				onclose={() => {
					modalOpen = false;
				}}
			>
				<div class="space-y-4">
					{#each node.modal_nodes ?? [] as child, idx (idx)}
						<SurfaceRenderer
							{surfaceId}
							node={child}
							{interactions}
							{dataSources}
							{targetProviderId}
							{encryptionContext}
							{dataBySource}
							{baseParams}
						/>
					{/each}
				</div>
			</SurfaceModal>
		{:else}
			{@const unavailable = renderUnavailableAction('This action is not available right now.')}
			<Callout tone="warning" title={unavailable.title} message={unavailable.message} />
		{/if}
	{:else}
		{@const unavailable = renderUnavailableAction('This action is not available right now.')}
		<Callout tone="warning" title={unavailable.title} message={unavailable.message} />
	{/if}
{:else if node.kind === 'workflow_trigger'}
	{@const interaction = findInteraction(node.interaction_id)}
	{#if interaction}
		<SurfaceWorkflow {surfaceId} {interaction} {interactions} {targetProviderId} {encryptionContext} {baseParams} />
	{:else}
		{@const unavailable = renderUnavailableAction('This action is not available right now.')}
		<Callout tone="warning" title={unavailable.title} message={unavailable.message} />
	{/if}
{/if}
