<script lang="ts">
	import { SvelteSet } from 'svelte/reactivity';
	import type { ActionDef, ExtensionUi } from '$lib/types';
	import { invokeExtensionAction, getPluginConfigs } from '$lib/api';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import ActionButton from './ActionButton.svelte';
	import BatchActionBar from '$lib/components/BatchActionBar.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
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

	// Count visible row actions for colspan calculation.
	let hasRowActions = $derived(ui.row_actions.length > 0);

	// Batch-capable actions
	const batchCapableActions = $derived(actions.filter((a) => a.batch_action));
	const hasBatchActions = $derived(batchCapableActions.length > 0);
	let selectedIds = new SvelteSet<string>();
	let batchConfirmAction: ActionDef | null = $state(null);
	let batchSubmitting: boolean = $state(false);

	const batchBarActions = $derived(
		batchCapableActions.map((a) => ({ id: a.action_id, label: a.label, destructive: a.destructive }))
	);

	let columnCount = $derived(ui.columns.length + (hasRowActions ? 1 : 0) + (hasBatchActions ? 1 : 0));

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

	function toggleSelectAll() {
		if (selectedIds.size === rows.length) {
			selectedIds.clear();
		} else {
			selectedIds.clear();
			for (const row of rows) {
				const id = String(row.id ?? '');
				if (id) selectedIds.add(id);
			}
		}
	}

	function toggleSelect(id: string) {
		if (selectedIds.has(id)) {
			selectedIds.delete(id);
		} else {
			selectedIds.add(id);
		}
	}

	function requestBatchAction(actionId: string) {
		const action = batchCapableActions.find((a) => a.action_id === actionId);
		if (action) {
			if (action.destructive) {
				batchConfirmAction = action;
			} else {
				void executeBatchAction(action);
			}
		}
	}

	async function executeBatchAction(action: ActionDef) {
		if (batchSubmitting) return;
		batchConfirmAction = null;
		batchSubmitting = true;
		try {
			const ids = [...selectedIds];
			await invokeExtensionAction(extensionId, action.action_id, { ...contextParams, ids }, serviceId);
			showSuccess(`Batch ${action.label} completed for ${ids.length} item(s).`);
			selectedIds.clear();
			await reloadData();
		} catch (e) {
			showError(e instanceof Error ? e.message : `Batch ${action.label} failed`);
		} finally {
			batchSubmitting = false;
		}
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

	<div class="table-wrap">
		<table class="table">
			<thead>
				<tr>
					{#if hasBatchActions}
						<th class="w-10">
							<input
								type="checkbox"
								class="checkbox"
								checked={rows.length > 0 && selectedIds.size === rows.length}
								indeterminate={selectedIds.size > 0 && selectedIds.size < rows.length}
								onchange={toggleSelectAll}
								aria-label="Select all"
							/>
						</th>
					{/if}
					{#each ui.columns as col (col.key)}
						<th>{col.label}</th>
					{/each}
					{#if hasRowActions}
						<th class="w-20"></th>
					{/if}
				</tr>
			</thead>
			<tbody>
				{#if loading}
					<tr>
						<td colspan={columnCount} class="py-8 text-center">
							<p class="text-surface-500">Loading...</p>
						</td>
					</tr>
				{:else}
					{#each rows as row, i (i)}
						{@const rowId = String(row.id ?? '')}
						<tr>
							{#if hasBatchActions}
								<td>
									<input
										type="checkbox"
										class="checkbox"
										checked={selectedIds.has(rowId)}
										onchange={() => toggleSelect(rowId)}
										aria-label="Select row {i + 1}"
									/>
								</td>
							{/if}
							{#each ui.columns as col (col.key)}
								<td>{String(row[col.key] ?? '')}</td>
							{/each}
							{#if hasRowActions}
								<td>
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
					{:else}
						<tr>
							<td colspan={columnCount} class="py-8 text-center">
								<p class="text-lg font-medium">No data available</p>
								<p class="mt-1 text-sm text-surface-500">There are no items to display.</p>
							</td>
						</tr>
					{/each}
				{/if}
			</tbody>
		</table>
	</div>

	<Pagination {currentPage} {totalPages} {total} onPageChange={handlePageChange} />

	{#if hasBatchActions && selectedIds.size > 0}
		<BatchActionBar
			selectedCount={selectedIds.size}
			actions={batchBarActions}
			onaction={requestBatchAction}
			oncancel={() => selectedIds.clear()}
		/>
	{/if}

	{#if batchConfirmAction}
		<ConfirmDialog
			title="Batch {batchConfirmAction.label}"
			messagePrefix="Are you sure you want to {batchConfirmAction.label.toLowerCase()}"
			entityName="{selectedIds.size} item(s)"
			confirmLabel={batchSubmitting ? 'Processing...' : batchConfirmAction.label}
			confirmClass="preset-filled-error-500"
			confirmDisabled={batchSubmitting}
			onconfirm={() => {
				if (batchConfirmAction) void executeBatchAction(batchConfirmAction);
			}}
			oncancel={() => (batchConfirmAction = null)}
		/>
	{/if}
{/if}
