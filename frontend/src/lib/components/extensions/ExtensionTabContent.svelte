<script lang="ts">
	import type { ExtensionResponse, ActionDef } from '$lib/types';
	import { apiSubmitRequest, invokeExtensionAction } from '$lib/api';
	import { getUser } from '$lib/auth.svelte';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import { hasPermissionValue } from '$lib/types';
	import { renderApiSubmitTemplate } from '$lib/utils';
	import { resolveExtensionFormAction } from '$lib/extensions.svelte';
	import SchemaTable from './SchemaTable.svelte';
	import SchemaForm from './SchemaForm.svelte';
	import SchemaKeyValue from './SchemaKeyValue.svelte';
	import ActionButton from './ActionButton.svelte';
	import ServiceSelector from './ServiceSelector.svelte';

	let { extension }: { extension: ExtensionResponse } = $props();

	let selectedServiceId: string | undefined = $state(undefined);
	let selectedEncryptionKey: string | undefined = $state(undefined);
	let serviceLoaded: boolean = $state(false);
	let formSubmitting: boolean = $state(false);

	const isTargeted = $derived(extension.targeting === 'targeted');
	const contentReady = $derived(!isTargeted || serviceLoaded);
	const formAction = $derived(resolveExtensionFormAction(extension));

	function resolveAction(actionId: string): ActionDef | undefined {
		return extension.actions?.find((a) => a.action_id === actionId);
	}

	async function submitForm(values: Record<string, unknown>) {
		if (extension.ui.type !== 'form' || !formAction) {
			showError('This extension form is missing a submit action.');
			return;
		}
		if (!hasPermissionValue(getUser(), formAction.permission)) {
			showError('You do not have permission to run this action.');
			return;
		}

		formSubmitting = true;
		try {
			if (formAction.api_submit) {
				const body = renderApiSubmitTemplate(formAction.api_submit.body, values) as Record<string, unknown>;
				await apiSubmitRequest(formAction.api_submit.path, formAction.api_submit.method, body);
			} else {
				await invokeExtensionAction(
					extension.id,
					formAction.action_id,
					values,
					selectedServiceId,
					undefined,
					selectedEncryptionKey
				);
			}
			showSuccess(`${formAction.label} completed`);
		} catch (error) {
			showError(error instanceof Error ? error.message : 'Action failed');
		} finally {
			formSubmitting = false;
		}
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
	{#if !formAction}
		<aside class="rounded-lg p-4 preset-filled-warning-500">This extension form is missing a submit action.</aside>
	{:else}
		<SchemaForm
			fields={extension.ui.fields}
			onsubmit={submitForm}
			loading={formSubmitting}
			extensionId={extension.id}
			serviceId={selectedServiceId}
			preLoadAction={extension.ui.pre_load_action}
		/>
	{/if}
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
