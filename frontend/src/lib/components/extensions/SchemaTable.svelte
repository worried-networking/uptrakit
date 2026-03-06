<script lang="ts">
	import type { ExtensionUi } from '$lib/types';
	import { invokeExtensionAction, getPluginConfigs } from '$lib/api';
	import { showError } from '$lib/notifications.svelte';
	import ActionButton from './ActionButton.svelte';

	let {
		extensionId,
		ui,
		serviceId
	}: {
		extensionId: string;
		ui: Extract<ExtensionUi, { type: 'data_table' }>;
		serviceId?: string;
	} = $props();

	interface SelectorOption {
		value: string;
		label: string;
	}

	const cs = $derived(ui.context_selector);

	let contextOptions: SelectorOption[] = $state([]);
	let contextLoaded: boolean = $state(false);
	let selectedContext: string | undefined = $state(undefined);

	let rows: Record<string, unknown>[] = $state([]);
	let loading: boolean = $state(false);

	// Data is ready when either there's no context selector, or a context value is selected.
	let dataReady = $derived(!cs || (contextLoaded && selectedContext != null));

	async function loadContextOptions(autoSelectId?: string) {
		if (!cs) return;
		try {
			const source = cs.source;
			if (source.type === 'plugin_configs') {
				const result = await getPluginConfigs();
				contextOptions = result.items
					.filter((c) => c.plugin_type === source.plugin_type)
					.map((c) => ({ value: c.id, label: c.name }));
			} else if (source.type === 'action') {
				const result = await invokeExtensionAction(extensionId, source.action_id, {}, serviceId);
				const arr = Array.isArray(result) ? result : (((result as Record<string, unknown>)?.items as unknown[]) ?? []);
				contextOptions = arr.map((item) => {
					const i = item as Record<string, unknown>;
					return { value: String(i.value ?? i.id ?? ''), label: String(i.label ?? i.name ?? '') };
				});
			}

			if (autoSelectId) {
				// After creating a new item, auto-select it if it appears in the list.
				if (contextOptions.some((o) => o.value === autoSelectId)) {
					selectedContext = autoSelectId;
				}
			} else if (contextOptions.length === 1 && !selectedContext) {
				selectedContext = contextOptions[0].value;
			}
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load options');
			contextOptions = [];
		} finally {
			contextLoaded = true;
		}
	}

	async function loadData() {
		if (!dataReady) return;
		loading = true;
		try {
			const params: Record<string, unknown> = {};
			if (cs && selectedContext != null) {
				params[cs.param_key] = selectedContext;
			}
			const result = await invokeExtensionAction(extensionId, ui.data_action, params, serviceId);
			rows = Array.isArray(result)
				? result
				: (((result as Record<string, unknown>)?.rows as Record<string, unknown>[]) ?? []);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load data');
			rows = [];
		} finally {
			loading = false;
		}
	}

	async function handleAddActionComplete(result?: Record<string, unknown>) {
		const idField = cs?.add_action?.api_submit?.response_id_field;
		const newId = idField && result ? String(result[idField] ?? '') : undefined;
		await loadContextOptions(newId || undefined);
	}

	$effect(() => {
		if (cs) {
			void loadContextOptions();
		} else {
			contextLoaded = true;
		}
	});

	// Reload table data when context selection changes or on initial load (no context selector).
	$effect(() => {
		if (dataReady) {
			void loadData();
		}
	});

	// Extra params injected into all row/primary actions when a context is selected.
	let contextParams = $derived(cs && selectedContext != null ? { [cs.param_key]: selectedContext } : {});
</script>

<div class="card p-4">
	{#if cs}
		<div class="mb-4 flex items-center gap-3">
			<span class="text-sm font-medium text-surface-600 dark:text-surface-400">{cs.label}:</span>

			{#if !contextLoaded}
				<span class="text-sm text-surface-500">Loading...</span>
			{:else if contextOptions.length === 0}
				{#if cs.empty_message}
					<span class="text-sm text-surface-500">{cs.empty_message}</span>
				{:else}
					<span class="text-sm text-surface-500">No options available.</span>
				{/if}
			{:else if contextOptions.length === 1}
				<span class="text-sm">{contextOptions[0].label}</span>
			{:else}
				<select class="select w-auto" bind:value={selectedContext}>
					<option value={undefined}>Select...</option>
					{#each contextOptions as opt (opt.value)}
						<option value={opt.value}>{opt.label}</option>
					{/each}
				</select>
			{/if}

			{#if cs.add_action}
				<ActionButton {extensionId} action={cs.add_action} {serviceId} size="sm" onComplete={handleAddActionComplete} />
			{/if}
		</div>
	{/if}

	{#if !dataReady}
		<p class="py-8 text-center text-surface-500">Select a configuration to continue.</p>
	{:else}
		{#if ui.primary_actions.length > 0}
			<div class="mb-4 flex flex-wrap gap-2">
				{#each ui.primary_actions as action (action.action_id)}
					<ActionButton {extensionId} {action} {serviceId} extraParams={contextParams} onComplete={loadData} />
				{/each}
			</div>
		{/if}

		{#if loading}
			<p class="py-8 text-center text-surface-500">Loading...</p>
		{:else if rows.length === 0}
			<p class="py-8 text-center text-surface-500">No data available.</p>
		{:else}
			<div class="overflow-x-auto">
				<table class="w-full text-left text-sm">
					<thead>
						<tr class="border-b border-surface-300 dark:border-surface-600">
							{#each ui.columns as col (col.key)}
								<th class="px-3 py-2 font-medium">{col.label}</th>
							{/each}
							{#if ui.row_actions.length > 0}
								<th class="px-3 py-2 font-medium">Actions</th>
							{/if}
						</tr>
					</thead>
					<tbody>
						{#each rows as row, i (i)}
							<tr class="border-b border-surface-200 dark:border-surface-700">
								{#each ui.columns as col (col.key)}
									<td class="px-3 py-2">{String(row[col.key] ?? '')}</td>
								{/each}
								{#if ui.row_actions.length > 0}
									<td class="px-3 py-2">
										<div class="flex gap-1">
											{#each ui.row_actions as action (action.action_id)}
												<ActionButton
													{extensionId}
													{action}
													{serviceId}
													extraParams={{ ...contextParams, _row: row }}
													size="sm"
													onComplete={loadData}
												/>
											{/each}
										</div>
									</td>
								{/if}
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}
	{/if}
</div>
