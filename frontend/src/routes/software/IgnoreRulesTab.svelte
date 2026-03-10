<script lang="ts">
	import { onMount } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import { getSoftwareIgnores, createSoftwareIgnore, deleteSoftwareIgnore, batchSoftwareIgnores } from '$lib/api';
	import { formatDate } from '$lib/utils';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import Modal from '$lib/components/Modal.svelte';
	import Pagination from '$lib/components/Pagination.svelte';
	import BatchActionBar from '$lib/components/BatchActionBar.svelte';
	import BatchResultDialog from '$lib/components/BatchResultDialog.svelte';
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
			ignores = ignores.filter((i) => i.id !== id);
			showSuccess('Ignore rule deleted.');
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
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to delete ignore rules');
		} finally {
			ignoreBatchSubmitting = false;
		}
	}
</script>

<div class="mb-4 flex justify-end">
	{#if canManage}
		<button class="btn preset-filled-primary-500" onclick={openCreateIgnore}>Add Ignore Rule</button>
	{/if}
</div>

{#if ignoresLoading}
	<p class="text-center py-4">Loading...</p>
{:else}
	<div class="table-wrap">
		<table class="table">
			<thead>
				<tr>
					{#if canManage}
						<th class="w-10">
							<input
								type="checkbox"
								class="checkbox"
								checked={ignores.length > 0 && ignoreSelectedIds.size === ignores.length}
								indeterminate={ignoreSelectedIds.size > 0 && ignoreSelectedIds.size < ignores.length}
								onchange={toggleIgnoreSelectAll}
								aria-label="Select all"
							/>
						</th>
					{/if}
					<th>Name</th>
					<th>Created</th>
					{#if canManage}<th class="w-24">Actions</th>{/if}
				</tr>
			</thead>
			<tbody>
				{#each ignores as ignore (ignore.id)}
					<tr>
						{#if canManage}
							<td>
								<input
									type="checkbox"
									class="checkbox"
									checked={ignoreSelectedIds.has(ignore.id)}
									onchange={() => toggleIgnoreSelect(ignore.id)}
									aria-label="Select {ignore.name}"
								/>
							</td>
						{/if}
						<td><span class="font-medium">{ignore.name}</span></td>
						<td>{formatDate(ignore.created_at)}</td>
						{#if canManage}
							<td>
								<button
									class="btn btn-sm preset-tonal-error"
									onclick={() => (ignoreDeleteConfirm = { id: ignore.id, name: ignore.name })}
								>
									Delete
								</button>
							</td>
						{/if}
					</tr>
				{:else}
					<tr>
						<td colspan={canManage ? 4 : 2} class="py-8 text-center">
							<p class="text-lg font-medium">No ignore rules</p>
							<p class="mt-1 text-sm text-surface-500">
								Add ignore rules to suppress software items from autodiscovery by name.
							</p>
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
	<Pagination
		currentPage={ignoresPage}
		totalPages={ignoresTotalPages}
		total={ignoresTotalItems}
		onPageChange={loadIgnores}
	/>
{/if}

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
		confirmClass="preset-filled-error-500"
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
	<Modal title="Add Ignore Rule" onclose={closeIgnoreModal}>
		<label class="label">
			<span>Software Item Name</span>
			<input class="input" type="text" placeholder="e.g. FreshRSS or Plex Media Server" bind:value={ignoreForm.name} />
		</label>
		{#snippet footer()}
			<button class="btn preset-tonal-surface" onclick={closeIgnoreModal}>Cancel</button>
			<button class="btn preset-filled-primary-500" onclick={saveIgnore} disabled={!ignoreForm.name.trim()}>
				Create
			</button>
		{/snippet}
	</Modal>
{/if}

{#if ignoreDeleteConfirm}
	<ConfirmDialog
		title="Delete Ignore Rule"
		messagePrefix="Are you sure you want to delete the ignore rule for"
		entityName={ignoreDeleteConfirm.name}
		confirmLabel="Delete"
		confirmClass="preset-filled-error-500"
		onconfirm={executeDeleteIgnore}
		oncancel={() => (ignoreDeleteConfirm = null)}
	/>
{/if}
