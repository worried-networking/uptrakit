<script lang="ts">
	import type { ActionDef, ApiSubmitDef } from '$lib/types';
	import { invokeExtensionAction, apiSubmitRequest } from '$lib/api';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import SchemaForm from './SchemaForm.svelte';
	import Modal from '$lib/components/Modal.svelte';

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
		onComplete?: (result?: Record<string, unknown>) => void | Promise<void>;
	} = $props();

	let showModal: boolean = $state(false);
	let loading: boolean = $state(false);

	/** Recursively substitute `{{field:coercion}}` placeholders in a JSON body template. */
	function applyTemplate(template: unknown, values: Record<string, unknown>): unknown {
		if (typeof template === 'string') {
			const match = template.match(/^\{\{(\w+)(?::(\w+))?\}\}$/);
			if (!match) return template;
			const [, fieldName, coercion] = match;
			const raw = String(values[fieldName] ?? '');
			if (coercion === 'bool') return raw === 'true';
			if (coercion === 'number') return Number(raw);
			if (coercion === 'csv_array')
				return raw
					.split(',')
					.map((s) => s.trim())
					.filter(Boolean);
			return raw;
		}
		if (Array.isArray(template)) return template.map((item) => applyTemplate(item, values));
		if (template !== null && typeof template === 'object') {
			return Object.fromEntries(
				Object.entries(template as Record<string, unknown>).map(([k, v]) => [k, applyTemplate(v, values)])
			);
		}
		return template;
	}

	async function invoke(params: Record<string, unknown> = {}) {
		loading = true;
		try {
			const merged = { ...extraParams, ...params };
			let result: Record<string, unknown> | undefined;
			if (action.api_submit) {
				const def: ApiSubmitDef = action.api_submit;
				const body = applyTemplate(def.body, merged) as Record<string, unknown>;
				result = await apiSubmitRequest(def.path, def.method, body);
			} else {
				await invokeExtensionAction(extensionId, action.action_id, merged, serviceId);
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
	<Modal
		title={action.label}
		maxWidth="max-w-lg"
		onclose={() => {
			showModal = false;
		}}
	>
		<SchemaForm fields={action.ui.fields} onsubmit={invoke} {loading} />
	</Modal>
{/if}
