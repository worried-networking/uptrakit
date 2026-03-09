<script lang="ts">
	import type { ExtensionResponse, ActionDef } from '$lib/types';
	import SchemaTable from './SchemaTable.svelte';
	import SchemaForm from './SchemaForm.svelte';
	import SchemaKeyValue from './SchemaKeyValue.svelte';
	import ActionButton from './ActionButton.svelte';
	import ServiceSelector from './ServiceSelector.svelte';

	let { extension }: { extension: ExtensionResponse } = $props();

	let selectedServiceId: string | undefined = $state(undefined);
	let selectedEncryptionKey: string | undefined = $state(undefined);
	let serviceLoaded: boolean = $state(false);

	const isTargeted = $derived(extension.targeting === 'targeted');
	const contentReady = $derived(!isTargeted || serviceLoaded);

	function resolveAction(actionId: string): ActionDef | undefined {
		return extension.actions?.find((a) => a.action_id === actionId);
	}
</script>

{#if isTargeted}
	<div class="mb-4 flex justify-end">
		<ServiceSelector
			extensionId={extension.id}
			bind:selectedServiceId
			bind:selectedEncryptionKey
			bind:loaded={serviceLoaded}
		/>
	</div>
{/if}

{#if !contentReady}
	<p class="py-8 text-center text-surface-500">Loading...</p>
{:else if isTargeted && !selectedServiceId}
	<p class="py-8 text-center text-surface-500">No service is currently connected for this extension.</p>
{:else if extension.ui.type === 'data_table'}
	<SchemaTable
		extensionId={extension.id}
		ui={extension.ui}
		actions={extension.actions ?? []}
		serviceId={selectedServiceId}
		encryptionPublicKey={selectedEncryptionKey}
	/>
{:else if extension.ui.type === 'form'}
	<SchemaForm fields={extension.ui.fields} onsubmit={async () => {}} />
{:else if extension.ui.type === 'key_value'}
	<SchemaKeyValue extensionId={extension.id} dataAction={extension.ui.data_action} serviceId={selectedServiceId} />
{:else if extension.ui.type === 'actions'}
	<div class="flex flex-wrap gap-2">
		{#each extension.ui.actions as actionId (actionId)}
			{@const action = resolveAction(actionId)}
			{#if action}
				<ActionButton
					extensionId={extension.id}
					{action}
					serviceId={selectedServiceId}
					encryptionPublicKey={selectedEncryptionKey}
				/>
			{/if}
		{/each}
	</div>
{/if}
