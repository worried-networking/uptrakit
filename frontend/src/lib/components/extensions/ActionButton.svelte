<script lang="ts">
	import type { ActionDef, ApiSubmitDef } from '$lib/types';
	import { invokeExtensionAction, apiSubmitRequest } from '$lib/api';
	import { getUser } from '$lib/auth.svelte';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import SchemaForm from './SchemaForm.svelte';
	import WizardModal from './WizardModal.svelte';
	import Modal from '$lib/components/Modal.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import { hasPermissionValue } from '$lib/types';
	import { renderApiSubmitTemplate } from '$lib/utils';

	let {
		extensionId,
		action,
		serviceId,
		encryptionPublicKey,
		extraParams = {},
		size = 'md',
		onComplete
	}: {
		extensionId: string;
		action: ActionDef;
		serviceId?: string;
		encryptionPublicKey?: string;
		extraParams?: Record<string, unknown>;
		size?: 'sm' | 'md';
		onComplete?: (result?: Record<string, unknown>) => void | Promise<void>;
	} = $props();

	let showModal: boolean = $state(false);
	let showConfirm: boolean = $state(false);
	let loading: boolean = $state(false);
	let pendingParams: Record<string, unknown> | null = $state(null);

	/**
	 * Collect the set of field keys marked as `sensitive` in the action's form UI.
	 * Only `form` type UIs carry field definitions; all other UI types (wizard, none)
	 * have no sensitive field metadata and return an empty set.
	 */
	function sensitiveFieldKeys(): Set<string> {
		if (action.ui?.type === 'form') {
			return new Set(action.ui.fields.filter((f) => f.sensitive).map((f) => f.key));
		}
		return new Set();
	}

	async function invoke(params: Record<string, unknown> = {}) {
		loading = true;
		try {
			const merged = { ...extraParams, ...params };
			let result: Record<string, unknown> | undefined;
			if (action.api_submit) {
				// Direct REST submit bypasses the extension proxy; no sensitive param routing.
				const def: ApiSubmitDef = action.api_submit;
				const body = renderApiSubmitTemplate(def.body, merged) as Record<string, unknown>;
				result = await apiSubmitRequest(def.path, def.method, body);
			} else {
				// Separate sensitive field values from regular params before sending.
				const sensitiveKeys = sensitiveFieldKeys();
				const regularParams: Record<string, unknown> = {};
				const sensitiveParams: Record<string, unknown> = {};
				for (const [k, v] of Object.entries(merged)) {
					if (sensitiveKeys.has(k)) {
						sensitiveParams[k] = v;
					} else {
						regularParams[k] = v;
					}
				}
				await invokeExtensionAction(
					extensionId,
					action.action_id,
					regularParams,
					serviceId,
					sensitiveParams,
					encryptionPublicKey
				);
			}
			showSuccess(`${action.label} completed`);
			showModal = false;
			await onComplete?.(result);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Action failed');
		} finally {
			loading = false;
		}
	}

	function handleClick() {
		if (!hasPermissionValue(getUser(), action.permission)) return;
		if (action.ui) {
			showModal = true;
		} else if (action.destructive) {
			showConfirm = true;
		} else {
			void invoke();
		}
	}

	/** Derive the entity name for the confirmation dialog from row data. */
	function confirmEntityName(): string {
		if (action.confirm_entity_field) {
			const val = extraParams[action.confirm_entity_field];
			if (val != null && val !== '') return String(val);
		}
		return 'this item';
	}

	let btnClass = $derived(size === 'sm' ? 'btn btn-sm text-xs' : 'btn');
	let presetClass = $derived(action.destructive ? 'preset-filled-error-500' : 'preset-filled-primary-500');
	let canInvoke = $derived(hasPermissionValue(getUser(), action.permission));
</script>

{#if canInvoke}
	<button class="{btnClass} {presetClass}" disabled={loading} onclick={handleClick}>
		{loading ? 'Processing...' : action.label}
	</button>
{/if}

{#if showModal && action.ui?.type === 'form'}
	<Modal
		title={action.label}
		maxWidth="max-w-lg"
		onclose={() => {
			showModal = false;
		}}
	>
		<SchemaForm
			fields={action.ui.fields}
			onsubmit={async (values) => {
				if (action.destructive) {
					pendingParams = values;
					showConfirm = true;
					return;
				}
				await invoke(values);
			}}
			{loading}
			{extensionId}
			{serviceId}
			{extraParams}
			preLoadAction={action.ui.pre_load_action}
		/>
	</Modal>
{/if}

{#if showModal && action.ui?.type === 'wizard'}
	<WizardModal
		steps={action.ui.steps}
		actionLabel={action.label}
		{extensionId}
		{serviceId}
		{encryptionPublicKey}
		{extraParams}
		onclose={() => {
			showModal = false;
		}}
		oncomplete={async (result) => {
			showSuccess(`${action.label} completed`);
			showModal = false;
			await onComplete?.(result as Record<string, unknown> | undefined);
		}}
	/>
{/if}

{#if showConfirm}
	<ConfirmDialog
		title={action.label}
		messagePrefix="Are you sure you want to {action.label.toLowerCase()}"
		entityName={confirmEntityName()}
		confirmLabel={action.label}
		onconfirm={() => {
			showConfirm = false;
			const params = pendingParams;
			pendingParams = null;
			void invoke(params ?? {});
		}}
		oncancel={() => {
			showConfirm = false;
			pendingParams = null;
		}}
	/>
{/if}
