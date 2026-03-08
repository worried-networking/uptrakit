<script lang="ts">
	import { page } from '$app/stores';
	import { getExtensions, getExtensionsLoaded } from '$lib/extensions.svelte';
	import type { ExtensionResponse, ActionDef } from '$lib/types';
	import SchemaTable from '$lib/components/extensions/SchemaTable.svelte';
	import SchemaForm from '$lib/components/extensions/SchemaForm.svelte';
	import SchemaKeyValue from '$lib/components/extensions/SchemaKeyValue.svelte';
	import ActionButton from '$lib/components/extensions/ActionButton.svelte';
	import ServiceSelector from '$lib/components/extensions/ServiceSelector.svelte';

	let extensionId = $derived($page.params.id);
	let extension: ExtensionResponse | undefined = $derived(getExtensions().find((e) => e.id === extensionId));

	let selectedServiceId: string | undefined = $state(undefined);
	let selectedEncryptionKey: string | undefined = $state(undefined);
	let serviceLoaded: boolean = $state(false);

	let isTargeted = $derived(extension?.targeting === 'targeted');
	// For targeted extensions, content should only render once we know the service state.
	// ServiceSelector sets selectedServiceId automatically when there is exactly one provider.
	let contentReady = $derived(!isTargeted || serviceLoaded);

	/** Resolve an action ID to its ActionDef from the extension's action library. */
	function resolveAction(actionId: string): ActionDef | undefined {
		return extension?.actions?.find((a) => a.action_id === actionId);
	}
</script>

<svelte:head>
	<title>{extension?.label ?? 'Extension'} | Uptrakit</title>
</svelte:head>

{#if !getExtensionsLoaded()}
	<p>Loading...</p>
{:else if !extension}
	<div class="py-8 text-center">
		<p class="text-lg font-medium">Extension not found</p>
		<p class="mt-1 text-sm text-surface-500">The extension "{extensionId}" is not available.</p>
	</div>
{:else}
	<div class="flex items-center justify-between mb-6">
		<h1 class="h1">{extension.label}</h1>
		{#if isTargeted}
			<ServiceSelector
				extensionId={extension.id}
				bind:selectedServiceId
				bind:selectedEncryptionKey
				bind:loaded={serviceLoaded}
			/>
		{/if}
	</div>

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
{/if}
