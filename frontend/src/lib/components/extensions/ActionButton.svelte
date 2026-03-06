<script lang="ts">
	import type { ActionDef } from '$lib/types';
	import { invokeExtensionAction } from '$lib/api';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import SchemaForm from './SchemaForm.svelte';
	import ModalBackdrop from '$lib/components/ModalBackdrop.svelte';

	let {
		extensionId,
		action,
		serviceId,
		extraParams = {},
		size = 'md',
		onComplete
	}: {
		extensionId: string;
		action: ActionDef;
		serviceId?: string;
		extraParams?: Record<string, unknown>;
		size?: 'sm' | 'md';
		onComplete?: () => void;
	} = $props();

	let showModal: boolean = $state(false);
	let loading: boolean = $state(false);

	async function invoke(params: Record<string, unknown> = {}) {
		loading = true;
		try {
			const merged = { ...extraParams, ...params };
			await invokeExtensionAction(extensionId, action.action_id, merged, serviceId);
			showSuccess(`${action.label} completed`);
			showModal = false;
			onComplete?.();
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Action failed');
		} finally {
			loading = false;
		}
	}

	function handleClick() {
		if (action.ui) {
			showModal = true;
		} else {
			void invoke();
		}
	}

	let btnClass = $derived(size === 'sm' ? 'btn btn-sm text-xs' : 'btn');
	let presetClass = $derived(action.destructive ? 'preset-filled-error-500' : 'preset-filled-primary-500');
</script>

<button class="{btnClass} {presetClass}" disabled={loading} onclick={handleClick}>
	{loading ? '...' : action.label}
</button>

{#if showModal && action.ui?.type === 'form'}
	<ModalBackdrop
		onclose={() => {
			showModal = false;
		}}
	>
		<div class="card p-6 w-full max-w-lg">
			<h3 class="h4 mb-4">{action.label}</h3>
			<SchemaForm fields={action.ui.fields} onsubmit={invoke} {loading} />
		</div>
	</ModalBackdrop>
{/if}
