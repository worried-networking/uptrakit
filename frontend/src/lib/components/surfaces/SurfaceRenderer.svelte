<script lang="ts">
	import SurfaceActionBar from './SurfaceActionBar.svelte';
	import SurfaceForm from './SurfaceForm.svelte';
	import SurfaceKeyValue from './SurfaceKeyValue.svelte';
	import SurfaceModal from './SurfaceModal.svelte';
	import SurfaceRenderer from './SurfaceRenderer.svelte';
	import SurfaceTable from './SurfaceTable.svelte';
	import SurfaceWorkflow from './SurfaceWorkflow.svelte';
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
		baseParams = {}
	}: {
		surfaceId: string;
		node: SurfaceNode;
		interactions?: InteractionDescriptor[];
		dataSources?: DataSourceDescriptor[];
		targetProviderId?: string;
		encryptionContext?: SurfaceEncryptionContext;
		dataBySource?: Record<string, unknown>;
		baseParams?: Record<string, unknown>;
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

	function calloutClass(level: 'info' | 'warning' | 'danger'): string {
		switch (level) {
			case 'danger':
				return 'preset-filled-error-500';
			case 'warning':
				return 'preset-filled-warning-500';
			default:
				return 'preset-tonal-primary';
		}
	}
</script>

{#if node.kind === 'section'}
	<div class="space-y-4">
		{#if node.title}
			<h3 class="h3">{node.title}</h3>
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
		<p class="text-sm text-error-600">Missing interaction `{node.interaction_id}`</p>
	{/if}
{:else if node.kind === 'action_bar'}
	<SurfaceActionBar
		{surfaceId}
		actionIds={node.action_ids ?? []}
		{interactions}
		{targetProviderId}
		{encryptionContext}
		{baseParams}
	/>
{:else if node.kind === 'tabs'}
	{@const tabs = node.tabs ?? []}
	{#if tabs.length === 0}
		<p class="text-sm text-surface-500">No tabs available.</p>
	{:else}
		<div class="space-y-4">
			<div class="flex flex-wrap gap-2">
				{#each tabs as tab, index (tab.id)}
					<button
						type="button"
						class="btn {selectedTab === index ? 'preset-filled-primary-500' : 'preset-tonal-surface'}"
						onclick={() => {
							selectedTab = index;
						}}
					>
						{tab.label}
					</button>
				{/each}
			</div>
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
	<aside class="rounded-lg p-3 text-sm {calloutClass(node.level)}">{node.text}</aside>
{:else if node.kind === 'empty_state'}
	<div class="rounded-lg border border-dashed border-surface-300 p-6 text-center dark:border-surface-700">
		<h4 class="text-base font-semibold">{node.title}</h4>
		{#if node.description}
			<p class="mt-2 text-sm text-surface-500">{node.description}</p>
		{/if}
	</div>
{:else if node.kind === 'modal_trigger'}
	{@const interaction = findInteraction(node.interaction_id)}
	<button class="btn preset-tonal-surface" type="button" onclick={() => (modalOpen = true)}>
		{interaction?.interaction_id ?? node.interaction_id}
	</button>
	<SurfaceModal
		open={modalOpen}
		title={interaction?.interaction_id ?? 'Details'}
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
{:else if node.kind === 'workflow_trigger'}
	{@const interaction = findInteraction(node.interaction_id)}
	{#if interaction}
		<SurfaceWorkflow {surfaceId} {interaction} {interactions} {targetProviderId} {encryptionContext} {baseParams} />
	{:else}
		<p class="text-sm text-error-600">Missing workflow interaction `{node.interaction_id}`</p>
	{/if}
{/if}
