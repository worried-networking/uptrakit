<script lang="ts">
	import type { ActionDef, ExtensionUi } from '$lib/types';
	import { invokeExtensionAction, getPluginConfigs } from '$lib/api';
	import { showError } from '$lib/notifications.svelte';
	import ActionButton from './ActionButton.svelte';
	import Pagination from '$lib/components/Pagination.svelte';

	/** Check whether a row action should be visible for a given row. */
	function isRowActionVisible(action: ActionDef, row: Record<string, unknown>): boolean {
		const cond = action.row_visible_when;
		if (!cond) return true;
		const val = row[cond.field];
		const isPresent = val != null && val !== '';
		return cond.condition === 'present' ? isPresent : !isPresent;
	}

	let {
		extensionId,
		ui,
		actions = [],
		serviceId,
		encryptionPublicKey
	}: {
		extensionId: string;
		ui: Extract<ExtensionUi, { type: 'data_table' }>;
		actions: ActionDef[];
		serviceId?: string;
		encryptionPublicKey?: string;
	} = $props();

	/** Resolve an action ID to its ActionDef from the action library. */
	function resolveAction(actionId: string): ActionDef | undefined {
		return actions.find((a) => a.action_id === actionId);
	}

	interface SelectorOption {
		value: string;
		label: string;
	}

	const cs = $derived(ui.context_selector);
	const addAction = $derived(cs?.add_action ? resolveAction(cs.add_action) : undefined);

	let contextOptions: SelectorOption[] = $state([]);
	let contextLoaded: boolean = $state(false);
	let selectedContext: string | undefined = $state(undefined);

	let rows: Record<string, unknown>[] = $state([]);
	let loading: boolean = $state(false);

	// Pagination state
	const defaultPerPage = $derived(ui.default_per_page ?? 20);
	let currentPage: number = $state(1);
	let totalPages: number = $state(1);
	let total: number = $state(0);
	let perPage: number = $state(20);

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
				// Paginate through all pages to collect complete option list.
				const allItems: unknown[] = [];
				let page = 1;
				let pageTotalPages = 1;
				do {
					const result = await invokeExtensionAction(
						extensionId,
						source.action_id,
						{ page, per_page: 1000 },
						serviceId
					);
					const resultObj = result as Record<string, unknown>;
					if (resultObj?.items && resultObj?.total_pages != null) {
						allItems.push(...(resultObj.items as unknown[]));
						pageTotalPages = resultObj.total_pages as number;
					} else {
						const arr = Array.isArray(result) ? result : ((resultObj?.items as unknown[]) ?? []);
						allItems.push(...arr);
						break;
					}
					page++;
				} while (page <= pageTotalPages);

				contextOptions = allItems.map((item) => {
					const i = item as Record<string, unknown>;
					return {
						value: String(i.value ?? i.id ?? ''),
						label: String(i.label ?? i.name ?? '')
					};
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
			const params: Record<string, unknown> = {
				page: currentPage,
				per_page: perPage
			};
			if (cs && selectedContext != null) {
				params[cs.param_key] = selectedContext;
			}
			const result = await invokeExtensionAction(extensionId, ui.data_action, params, serviceId);
			const resultObj = result as Record<string, unknown>;

			if (resultObj?.items && resultObj?.total_pages != null) {
				rows = resultObj.items as Record<string, unknown>[];
				total = (resultObj.total as number) ?? 0;
				currentPage = (resultObj.page as number) ?? 1;
				perPage = (resultObj.per_page as number) ?? defaultPerPage;
				totalPages = (resultObj.total_pages as number) ?? 1;
			} else {
				// Fallback for actions that don't return paginated responses.
				rows = Array.isArray(result) ? result : ((resultObj?.rows as Record<string, unknown>[]) ?? []);
				total = rows.length;
				totalPages = 1;
				currentPage = 1;
			}
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load data');
			rows = [];
			total = 0;
			totalPages = 1;
		} finally {
			loading = false;
		}
	}

	function handlePageChange(page: number) {
		currentPage = page;
		void loadData();
	}

	async function handleAddActionComplete(result?: Record<string, unknown>) {
		const idField = addAction?.api_submit?.response_id_field;
		const newId = idField && result ? String(result[idField] ?? '') : undefined;
		await loadContextOptions(newId || undefined);
	}

	/** Reload data and reset to page 1. */
	async function reloadData() {
		currentPage = 1;
		await loadData();
	}

	$effect(() => {
		if (cs) {
			void loadContextOptions();
		} else {
			contextLoaded = true;
		}
	});

	// Reload table data when context selection changes or on initial load (no context selector).
	// Reset to page 1 when context changes.
	let prevContext: string | undefined = $state(undefined);
	$effect(() => {
		if (dataReady) {
			if (selectedContext !== prevContext) {
				prevContext = selectedContext;
				currentPage = 1;
			}
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

			{#if addAction}
				<ActionButton
					{extensionId}
					action={addAction}
					{serviceId}
					{encryptionPublicKey}
					size="sm"
					onComplete={handleAddActionComplete}
				/>
			{/if}
		</div>
	{/if}

	{#if !dataReady}
		<p class="py-8 text-center text-surface-500">Select a configuration to continue.</p>
	{:else}
		{#if ui.primary_actions.length > 0}
			<div class="mb-4 flex flex-wrap gap-2">
				{#each ui.primary_actions as actionId (actionId)}
					{@const action = resolveAction(actionId)}
					{#if action}
						<ActionButton
							{extensionId}
							{action}
							{serviceId}
							{encryptionPublicKey}
							extraParams={contextParams}
							onComplete={reloadData}
						/>
					{/if}
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
											{#each ui.row_actions as actionId (actionId)}
												{@const action = resolveAction(actionId)}
												{#if action && isRowActionVisible(action, row)}
													<ActionButton
														{extensionId}
														{action}
														{serviceId}
														{encryptionPublicKey}
														extraParams={{ ...contextParams, ...row, _row: row }}
														size="sm"
														onComplete={reloadData}
													/>
												{/if}
											{/each}
										</div>
									</td>
								{/if}
							</tr>
						{/each}
					</tbody>
				</table>
			</div>

			{#if totalPages > 1}
				<div class="mt-2 flex items-center justify-between text-sm text-surface-500">
					<span>{total} total</span>
					<Pagination {currentPage} {totalPages} onPageChange={handlePageChange} />
				</div>
			{/if}
		{/if}
	{/if}
</div>
