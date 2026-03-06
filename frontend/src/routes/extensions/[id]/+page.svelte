<script lang="ts">
	import { page } from '$app/stores';
	import { getExtensions, getExtensionsLoaded } from '$lib/extensions.svelte';
	import type { ExtensionResponse } from '$lib/types';
	import SchemaTable from '$lib/components/extensions/SchemaTable.svelte';
	import SchemaForm from '$lib/components/extensions/SchemaForm.svelte';
	import SchemaKeyValue from '$lib/components/extensions/SchemaKeyValue.svelte';
	import ActionButton from '$lib/components/extensions/ActionButton.svelte';
	import ServiceSelector from '$lib/components/extensions/ServiceSelector.svelte';

	let extensionId = $derived($page.params.id);
	let extension: ExtensionResponse | undefined = $derived(getExtensions().find((e) => e.id === extensionId));

	let selectedServiceId: string | undefined = $state(undefined);

	let needsServiceSelector = $derived(extension?.targeting === 'targeted' && (extension?.provider_count ?? 0) > 0);
</script>

<svelte:head>
	<title>{extension?.label ?? 'Extension'} | Uptrakit</title>
</svelte:head>

{#if !getExtensionsLoaded()}
	<p>Loading...</p>
{:else if !extension}
	<div class="card p-6 text-center">
		<h2 class="h3 mb-2">Extension not found</h2>
		<p class="text-surface-500">The extension "{extensionId}" is not available.</p>
	</div>
{:else}
	<div class="space-y-4">
		<div class="flex items-center justify-between">
			<h1 class="h3">{extension.label}</h1>
			{#if needsServiceSelector}
				<ServiceSelector extensionId={extension.id} bind:selectedServiceId />
			{/if}
		</div>

		{#if extension.ui.type === 'data_table'}
			<SchemaTable extensionId={extension.id} ui={extension.ui} serviceId={selectedServiceId} />
		{:else if extension.ui.type === 'form'}
			<SchemaForm fields={extension.ui.fields} onsubmit={async () => {}} />
		{:else if extension.ui.type === 'key_value'}
			<SchemaKeyValue extensionId={extension.id} dataAction={extension.ui.data_action} serviceId={selectedServiceId} />
		{:else if extension.ui.type === 'actions'}
			<div class="flex flex-wrap gap-2">
				{#each extension.ui.actions as action (action.action_id)}
					<ActionButton extensionId={extension.id} {action} serviceId={selectedServiceId} />
				{/each}
			</div>
		{/if}
	</div>
{/if}
