<script lang="ts">
	import { onMount } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import { getSoftwareIgnores, createSoftwareIgnore, deleteSoftwareIgnore, batchSoftwareIgnores } from '$lib/api';
	import { formatDate, nextValidPage } from '$lib/utils';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import BatchActionBar from '$lib/components/BatchActionBar.svelte';
	import BatchResultDialog from '$lib/components/BatchResultDialog.svelte';
	import { DataTable, FormFieldRow, ModalShell, TableFooterBar, type DataTableColumn } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';
	import Input from '$lib/components/Input.svelte';
	import Checkbox from '$lib/components/Checkbox.svelte';
	import { getUser } from '$lib/auth.svelte';
	import { Permission } from '$lib/types';
	import type { SoftwareIgnoreResponse, BatchActionResponse } from '$lib/types';

	const canManage = $derived(getUser()?.permissions.includes(Permission.ManageIgnores) ?? false);

	let ignores: SoftwareIgnoreResponse[] = $state([]);
	let ignoresLoading: boolean = $state(true);
	let ignoresPage: number = $state(1);
	let ignoresTotalPages: number = $state(1);
	let ignoresTotalItems: number = $state(0);
	let showIgnoreModal: boolean = $state(false);
	let ignoreForm = $state({ name: '' });
	let ignoreDeleteConfirm: { id: string; name: string } | null = $state(null);

	let ignoreSelectedIds = new SvelteSet<string>();
	let ignoreBatchConfirmAction: string | null = $state(null);
	let ignoreBatchResult: BatchActionResponse | null = $state(null);
	let ignoreBatchSubmitting: boolean = $state(false);

	const ignoreBatchActions: { id: string; label: string; destructive?: boolean }[] = [
		{ id: 'delete', label: 'Delete', destructive: true }
	];
	const ignoreTableColumns = $derived.by<DataTableColumn[]>(() => {
		const columns: DataTableColumn[] = [
			{ key: 'name', label: 'Name' },
			{ key: 'created', label: 'Created' }
		];
		if (canManage) {
			return [{ key: 'select', label: 'Select' }, ...columns, { key: 'actions', label: 'Actions' }];
		}
		return columns;
	});

	onMount(() => {
		loadIgnores(1);
	});

	async function loadIgnores(p: number) {
		ignoresLoading = true;
		try {
			const res = await getSoftwareIgnores(p);
			ignores = res.items;
			ignoresPage = res.page;
			ignoresTotalPages = res.total_pages;
			ignoresTotalItems = res.total;
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load ignore rules');
		} finally {
			ignoresLoading = false;
		}
	}

	function openCreateIgnore() {
		ignoreForm = { name: '' };
		showIgnoreModal = true;
	}

	function closeIgnoreModal() {
		showIgnoreModal = false;
	}

	async function saveIgnore() {
		if (!ignoreForm.name.trim()) {
			showError('Name is required');
			return;
		}
		try {
			await createSoftwareIgnore({ name: ignoreForm.name.trim() });
			showSuccess('Ignore rule created.');
			closeIgnoreModal();
			loadIgnores(1);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to create ignore rule');
		}
	}

	async function executeDeleteIgnore() {
		if (!ignoreDeleteConfirm) return;
		const { id } = ignoreDeleteConfirm;
		ignoreDeleteConfirm = null;
		try {
			await deleteSoftwareIgnore(id);
			showSuccess('Ignore rule deleted.');
			await loadIgnores(ignoresPage);
			const p = nextValidPage(ignoresPage, ignoresTotalPages);
			if (p !== null) await loadIgnores(p);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to delete ignore rule');
		}
	}

	function toggleIgnoreSelectAll() {
		if (ignoreSelectedIds.size === ignores.length) {
			ignoreSelectedIds.clear();
		} else {
			ignoreSelectedIds.clear();
			for (const i of ignores) ignoreSelectedIds.add(i.id);
		}
	}

	function toggleIgnoreSelect(id: string) {
		if (ignoreSelectedIds.has(id)) {
			ignoreSelectedIds.delete(id);
		} else {
			ignoreSelectedIds.add(id);
		}
	}

	async function executeIgnoreBatchAction() {
		if (!ignoreBatchConfirmAction || ignoreBatchSubmitting) return;
		ignoreBatchConfirmAction = null;
		ignoreBatchSubmitting = true;
		try {
			const response = await batchSoftwareIgnores('delete', [...ignoreSelectedIds]);
			if (response.failed.length > 0) {
				ignoreBatchResult = response;
			} else {
				showSuccess(`${response.succeeded.length} ignore rule(s) deleted.`);
			}
			ignoreSelectedIds.clear();
			await loadIgnores(ignoresPage);
			const p = nextValidPage(ignoresPage, ignoresTotalPages);
			if (p !== null) await loadIgnores(p);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to delete ignore rules');
		} finally {
			ignoreBatchSubmitting = false;
		}
	}
</script>

<div class="mb-4 flex justify-end">
	{#if canManage}
		<Button variant="primary" size="sm" onclick={openCreateIgnore}>Add Ignore Rule</Button>
	{/if}
</div>

<DataTable
	columns={ignoreTableColumns}
	rows={ignores as unknown as Record<string, unknown>[]}
	loading={ignoresLoading}
	emptyTitle="No ignore rules"
	emptyDescription="Add ignore rules to suppress software items from autodiscovery by name."
	rowKey={(rowValue) => (rowValue as unknown as SoftwareIgnoreResponse).id}
>
	{#snippet header()}
		<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
			{#if canManage}
				<th
					class="w-10 px-4 py-3 text-left text-table-header font-semibold uppercase tracking-table-header"
					scope="col"
				>
					<Checkbox
						id="ignore-rules-select-all"
						checked={ignores.length > 0 && ignoreSelectedIds.size === ignores.length}
						indeterminate={ignoreSelectedIds.size > 0 && ignoreSelectedIds.size < ignores.length}
						onchange={toggleIgnoreSelectAll}
						aria-label="Select all"
					/>
				</th>
			{/if}
			<th class="px-4 py-3 text-left text-table-header font-semibold uppercase tracking-table-header" scope="col"
				>Name</th
			>
			<th class="px-4 py-3 text-left text-table-header font-semibold uppercase tracking-table-header" scope="col"
				>Created</th
			>
			{#if canManage}
				<th class="w-24 px-4 py-3 text-left text-table-header font-semibold uppercase tracking-table-header" scope="col"
					>Actions</th
				>
			{/if}
		</tr>
	{/snippet}
	{#snippet row(rowValue)}
		{@const ignore = rowValue as unknown as SoftwareIgnoreResponse}
		<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
			{#if canManage}
				<td class="table-cell-pad">
					<Checkbox
						id="ignore-rule-{ignore.id}"
						checked={ignoreSelectedIds.has(ignore.id)}
						onchange={() => toggleIgnoreSelect(ignore.id)}
						aria-label="Select {ignore.name}"
					/>
				</td>
			{/if}
			<td class="table-cell-pad text-[var(--text-primary)]"><span class="font-medium">{ignore.name}</span></td>
			<td class="table-cell-pad text-[var(--text-primary)]">{formatDate(ignore.created_at)}</td>
			{#if canManage}
				<td class="table-cell-pad">
					<Button
						variant="danger"
						size="sm"
						onclick={() => (ignoreDeleteConfirm = { id: ignore.id, name: ignore.name })}>Delete</Button
					>
				</td>
			{/if}
		</tr>
	{/snippet}
	{#snippet footer()}
		<TableFooterBar
			total={ignoresTotalItems}
			currentPage={ignoresPage}
			totalPages={ignoresTotalPages}
			onPageChange={loadIgnores}
		/>
	{/snippet}
</DataTable>

{#if canManage && ignoreSelectedIds.size > 0}
	<BatchActionBar
		selectedCount={ignoreSelectedIds.size}
		actions={ignoreBatchActions}
		onaction={() => (ignoreBatchConfirmAction = 'delete')}
		oncancel={() => ignoreSelectedIds.clear()}
	/>
{/if}

{#if ignoreBatchConfirmAction}
	<ConfirmDialog
		title="Batch Delete Ignore Rules"
		messagePrefix="Are you sure you want to delete"
		entityName="{ignoreSelectedIds.size} ignore rule(s)"
		confirmLabel={ignoreBatchSubmitting ? 'Deleting...' : 'Delete'}
		confirmDisabled={ignoreBatchSubmitting}
		onconfirm={executeIgnoreBatchAction}
		oncancel={() => (ignoreBatchConfirmAction = null)}
	/>
{/if}

{#if ignoreBatchResult}
	<BatchResultDialog
		title="Batch Action Results"
		response={ignoreBatchResult}
		onclose={() => (ignoreBatchResult = null)}
	/>
{/if}

{#if showIgnoreModal}
	<ModalShell title="Add Ignore Rule" onclose={closeIgnoreModal}>
		<FormFieldRow label="Software Item Name" inputId="ignore-rule-create-name">
			<Input
				id="ignore-rule-create-name"
				type="text"
				placeholder="e.g. FreshRSS or Plex Media Server"
				bind:value={ignoreForm.name}
			/>
		</FormFieldRow>
		{#snippet footer()}
			<Button variant="secondary" onclick={closeIgnoreModal}>Cancel</Button>
			<Button variant="primary" disabled={!ignoreForm.name.trim()} onclick={saveIgnore}>Create</Button>
		{/snippet}
	</ModalShell>
{/if}

{#if ignoreDeleteConfirm}
	<ConfirmDialog
		title="Delete Ignore Rule"
		messagePrefix="Are you sure you want to delete the ignore rule for"
		entityName={ignoreDeleteConfirm.name}
		confirmLabel="Delete"
		onconfirm={executeDeleteIgnore}
		oncancel={() => (ignoreDeleteConfirm = null)}
	/>
{/if}
