<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import {
		getSoftwareItems,
		deleteSoftwareItem,
		approveSoftwareItem,
		checkSoftwareItemVersions,
		updateSoftwareItem,
		listPluginTypes,
		getSoftwareItem,
		triggerSoftwareUpdate,
		unassignHostFromSoftwareItemWithIgnore,
		batchSoftwareItems
	} from '$lib/api';
	import { formatDate, formatVersion, parseUrlPage } from '$lib/utils';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import { subscribeToEvent } from '$lib/stores/events.svelte';
	import Pagination from '$lib/components/Pagination.svelte';
	import AddSoftwareModal from '$lib/components/AddSoftwareModal.svelte';
	import AssignToHostModal from '$lib/components/AssignToHostModal.svelte';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import Modal from '$lib/components/Modal.svelte';
	import BatchActionBar from '$lib/components/BatchActionBar.svelte';
	import BatchResultDialog from '$lib/components/BatchResultDialog.svelte';
	import type { SoftwareItemResponse, SoftwareItemDetailResponse, BatchActionResponse } from '$lib/types';
	import { Permission } from '$lib/types';
	import { getTabExtensions } from '$lib/extensions.svelte';
	import IgnoreRulesTab from './IgnoreRulesTab.svelte';
	import ExtensionTabContent from '$lib/components/extensions/ExtensionTabContent.svelte';

	let items: SoftwareItemResponse[] = $state([]);
	let error: string | null = $state(null);
	let currentPage: number = $state(parseUrlPage(page.url));
	let totalPages: number = $state(1);
	let totalItems: number = $state(0);
	let loading: boolean = $state(false);
	let showAddModal: boolean = $state(false);
	let openMenuId: string | null = $state(null);
	let menuPos: { top: number; left: number } = $state({ top: 0, left: 0 });
	let confirmDelete: { id: string; name: string } | null = $state(null);
	let assignItem: { id: string; name: string } | null = $state(null);
	let submitting: boolean = $state(false);
	let checkingVersionsId: string | null = $state(null);
	let approvingId: string | null = $state(null);
	let activeTab: string = $state(page.url.searchParams.get('tab') ?? 'all');

	const tabExtensions = $derived(getTabExtensions('software'));
	const isItemsTab = $derived(activeTab === 'all' || activeTab === 'pending' || activeTab === 'active');
	let editItem: { id: string; name: string; enabled: boolean } | null = $state(null);
	let editForm = $state({ name: '', enabled: true });
	let editSubmitting: boolean = $state(false);
	let pluginTypeNames: Map<string, string> = $state(new Map());
	let updateModalItem: SoftwareItemResponse | null = $state(null);
	let updateModalDetail: SoftwareItemDetailResponse | null = $state(null);
	let updateModalLoading: boolean = $state(false);
	let selectedHostIds: Set<string> = $state(new Set());
	let triggeringUpdate: boolean = $state(false);
	let confirmIgnore: { id: string; name: string } | null = $state(null);
	let ignoreSubmitting: boolean = $state(false);
	let batchSelectedIds = new SvelteSet<string>();
	let batchConfirmAction: string | null = $state(null);
	let batchResult: BatchActionResponse | null = $state(null);

	const canView = $derived(getUser()?.permissions.includes(Permission.ViewSoftware) ?? false);
	const canManage = $derived(getUser()?.permissions.includes(Permission.ManageSoftware) ?? false);

	const batchActions = $derived.by(() => {
		const acts: { id: string; label: string; destructive?: boolean }[] = [];
		if (activeTab === 'pending') {
			acts.push({ id: 'approve', label: 'Approve' });
			acts.push({ id: 'ignore', label: 'Ignore', destructive: true });
		}
		acts.push({ id: 'delete', label: 'Delete', destructive: true });
		return acts;
	});

	let refreshInterval: ReturnType<typeof setInterval> | null = null;
	let unsubscribers: (() => void)[] = [];

	$effect(() => {
		const parts: string[] = [];
		if (activeTab !== 'all') parts.push(`tab=${activeTab}`);
		if (isItemsTab && currentPage > 1) parts.push(`page=${currentPage}`);
		const search = parts.join('&');
		goto(search ? `${location.pathname}?${search}` : location.pathname, {
			replaceState: true,
			keepFocus: true,
			noScroll: true
		});
	});

	onMount(() => {
		if (canView) {
			loadAll(currentPage);
			unsubscribers.push(
				subscribeToEvent('software_item_updated', () => loadAll(currentPage, true)),
				subscribeToEvent('software_item_created', () => loadAll(currentPage, true)),
				subscribeToEvent('version_check_completed', () => loadAll(currentPage, true))
			);
			refreshInterval = setInterval(() => {
				if (document.visibilityState === 'visible') loadAll(currentPage, true);
			}, 300_000);
		}
		listPluginTypes()
			.then((types) => {
				pluginTypeNames = new Map(types.map((t) => [t.plugin_type, t.display_name]));
			})
			.catch(() => {
				// Non-fatal: raw plugin type keys will show as fallback
			});
	});

	onDestroy(() => {
		for (const unsub of unsubscribers) unsub();
		if (refreshInterval) clearInterval(refreshInterval);
	});

	function discoveryStateFilter(): 'pending' | 'approved' | undefined {
		if (activeTab === 'pending') return 'pending';
		if (activeTab === 'active') return 'approved';
		return undefined;
	}

	async function loadAll(page: number, background = false) {
		if (!background) {
			loading = true;
			error = null;
		}
		try {
			const result = await getSoftwareItems(page, undefined, discoveryStateFilter());
			items = result.items;
			currentPage = result.page;
			totalPages = result.total_pages;
			totalItems = result.total;
			if (background) error = null;
		} catch (e) {
			if (!background) {
				error = e instanceof Error ? e.message : 'Failed to load software items';
			}
		} finally {
			if (!background) loading = false;
		}
	}

	function switchTab(tab: string) {
		if (activeTab === tab) return;
		currentPage = 1;
		activeTab = tab;
		if (tab === 'all' || tab === 'pending' || tab === 'active') {
			loadAll(1);
		}
	}

	function toggleMenu(id: string, button: HTMLElement) {
		if (openMenuId === id) {
			openMenuId = null;
			return;
		}
		const rect = button.getBoundingClientRect();
		menuPos = { top: rect.bottom + 4, left: rect.right - 180 };
		openMenuId = id;
	}

	function closeMenu() {
		openMenuId = null;
	}

	function requestDelete(item: SoftwareItemResponse) {
		closeMenu();
		confirmDelete = { id: item.id, name: item.name };
	}

	function openAssignModal(item: SoftwareItemResponse) {
		closeMenu();
		assignItem = { id: item.id, name: item.name };
	}

	function openEditModal(item: SoftwareItemResponse) {
		closeMenu();
		editItem = { id: item.id, name: item.name, enabled: item.enabled };
		editForm = { name: item.name, enabled: item.enabled };
	}

	async function executeEdit() {
		if (!editItem || editSubmitting) return;
		editSubmitting = true;
		try {
			const updated = await updateSoftwareItem(editItem.id, {
				name: editForm.name || undefined,
				enabled: editForm.enabled
			});
			items = items.map((i) => (i.id === editItem!.id ? updated : i));
			showSuccess(`"${updated.name}" updated.`);
			editItem = null;
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to update software item.');
		} finally {
			editSubmitting = false;
		}
	}

	async function executeDelete() {
		if (!confirmDelete || submitting) return;
		const { id } = confirmDelete;
		confirmDelete = null;
		submitting = true;
		try {
			await deleteSoftwareItem(id);
			items = items.filter((i) => i.id !== id);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to delete software item.');
			loadAll(currentPage);
		} finally {
			submitting = false;
		}
	}

	function requestIgnore(item: SoftwareItemResponse) {
		closeMenu();
		confirmIgnore = { id: item.id, name: item.name };
	}

	async function executeIgnore() {
		if (!confirmIgnore || ignoreSubmitting) return;
		const { id, name } = confirmIgnore;
		confirmIgnore = null;
		ignoreSubmitting = true;
		try {
			const detail = await getSoftwareItem(id);
			await Promise.all(detail.hosts.map((h) => unassignHostFromSoftwareItemWithIgnore(id, h.host_id)));
			await deleteSoftwareItem(id);
			items = items.filter((i) => i.id !== id);
			showSuccess(`"${name}" will be ignored in future autodiscovery runs.`);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to ignore software item.');
			loadAll(currentPage);
		} finally {
			ignoreSubmitting = false;
		}
	}

	async function triggerVersionCheck(item: SoftwareItemResponse) {
		closeMenu();
		checkingVersionsId = item.id;
		try {
			const result = await checkSoftwareItemVersions(item.id);
			if (result.agents_notified > 0) {
				showSuccess(`Version check triggered — ${result.agents_notified} agent(s) notified`);
			} else {
				showSuccess('Version check queued — no agents currently connected');
			}
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to trigger version check.');
		} finally {
			checkingVersionsId = null;
		}
	}

	async function approveItem(item: SoftwareItemResponse) {
		closeMenu();
		approvingId = item.id;
		try {
			const updated = await approveSoftwareItem(item.id);
			showSuccess(`"${item.name}" approved for version tracking`);
			if (activeTab === 'pending') {
				items = items.filter((i) => i.id !== item.id);
			} else {
				items = items.map((i) => (i.id === item.id ? updated : i));
			}
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to approve software item.');
		} finally {
			approvingId = null;
		}
	}

	async function openUpdateModal(item: SoftwareItemResponse) {
		closeMenu();
		updateModalItem = item;
		updateModalDetail = null;
		selectedHostIds = new Set();
		updateModalLoading = true;
		try {
			const detail = await getSoftwareItem(item.id);
			updateModalDetail = detail;
			selectedHostIds = new Set(detail.hosts.filter((h) => h.update_available).map((h) => h.host_id));
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load host details.');
			updateModalItem = null;
		} finally {
			updateModalLoading = false;
		}
	}

	async function executeUpdate() {
		if (!updateModalItem || !updateModalDetail || triggeringUpdate) return;
		triggeringUpdate = true;
		const targets = updateModalDetail.hosts.filter(
			(h) => h.update_available && selectedHostIds.has(h.host_id) && h.latest_version
		);
		const results = await Promise.allSettled(
			targets.map((h) => triggerSoftwareUpdate(updateModalItem!.id, h.host_id, { to_version: h.latest_version! }))
		);
		const succeeded = results.filter((r) => r.status === 'fulfilled').length;
		const failed = results.filter((r) => r.status === 'rejected').length;
		if (succeeded > 0) showSuccess(`Update triggered for ${succeeded} host(s).`);
		if (failed > 0) showError(`Failed to trigger update for ${failed} host(s).`);
		triggeringUpdate = false;
		updateModalItem = null;
		loadAll(currentPage);
	}

	function toggleBatchSelectAll() {
		if (batchSelectedIds.size === items.length) {
			batchSelectedIds.clear();
		} else {
			batchSelectedIds.clear();
			for (const item of items) batchSelectedIds.add(item.id);
		}
	}

	function toggleBatchSelect(id: string) {
		if (batchSelectedIds.has(id)) {
			batchSelectedIds.delete(id);
		} else {
			batchSelectedIds.add(id);
		}
	}

	function requestBatchAction(actionId: string) {
		batchConfirmAction = actionId;
	}

	async function executeBatchAction() {
		if (!batchConfirmAction || submitting) return;
		const action = batchConfirmAction;
		const ids = [...batchSelectedIds];
		batchConfirmAction = null;
		submitting = true;
		try {
			if (action === 'ignore') {
				// Client-side orchestration: fetch details, unassign hosts with ignore, then delete
				const succeeded: { id: string }[] = [];
				const failed: { id: string; error: string }[] = [];
				for (const itemId of ids) {
					try {
						const detail = await getSoftwareItem(itemId);
						await Promise.all(detail.hosts.map((h) => unassignHostFromSoftwareItemWithIgnore(itemId, h.host_id)));
						await deleteSoftwareItem(itemId);
						succeeded.push({ id: itemId });
					} catch (e) {
						failed.push({ id: itemId, error: e instanceof Error ? e.message : 'Unknown error' });
					}
				}
				if (failed.length > 0) {
					batchResult = { succeeded, failed };
				} else {
					showSuccess(`${succeeded.length} item(s) will be ignored in future autodiscovery runs.`);
				}
			} else {
				const response = await batchSoftwareItems(action, ids);
				if (response.failed.length > 0) {
					batchResult = response;
				} else {
					showSuccess(`${response.succeeded.length} item(s) ${action}d successfully.`);
				}
			}
			batchSelectedIds.clear();
			await loadAll(currentPage);
		} catch (e) {
			showError(e instanceof Error ? e.message : `Failed to ${action} software items`);
		} finally {
			submitting = false;
		}
	}

	function handleWindowClick(event: MouseEvent) {
		if (openMenuId && !(event.target as HTMLElement).closest('.actions-menu')) {
			closeMenu();
		}
	}
</script>

<svelte:window onclick={handleWindowClick} />

{#if getUser()}
	<h1 class="h1 mb-4">Software</h1>

	{#if !canView}
		<aside class="rounded-lg p-4 preset-filled-error-500">
			<p>You do not have permission to view software items.</p>
		</aside>
	{:else}
		<div class="mb-4 flex items-center justify-between gap-2 flex-wrap">
			<div class="flex gap-1 flex-wrap">
				<button
					class="btn btn-sm {activeTab === 'all' ? 'preset-filled-primary-500' : 'preset-tonal'}"
					onclick={() => switchTab('all')}
				>
					All
				</button>
				<button
					class="btn btn-sm {activeTab === 'pending' ? 'preset-filled-warning-500' : 'preset-tonal'}"
					onclick={() => switchTab('pending')}
				>
					Pending{activeTab === 'pending' && totalItems > 0 ? ` (${totalItems})` : ''}
				</button>
				<button
					class="btn btn-sm {activeTab === 'active' ? 'preset-filled-success-500' : 'preset-tonal'}"
					onclick={() => switchTab('active')}
				>
					Active
				</button>
				<button
					class="btn btn-sm {activeTab === 'ignores' ? 'preset-filled-primary-500' : 'preset-tonal'}"
					onclick={() => switchTab('ignores')}
				>
					Ignore Rules
				</button>
				{#each tabExtensions as ext (ext.id)}
					<button
						class="btn btn-sm {activeTab === ext.id ? 'preset-filled-primary-500' : 'preset-tonal'}"
						onclick={() => switchTab(ext.id)}
					>
						{ext.label}
					</button>
				{/each}
			</div>
			{#if isItemsTab && canManage}
				<button class="btn preset-filled-primary-500" onclick={() => (showAddModal = true)}>Add Software</button>
			{/if}
		</div>

		{#if isItemsTab}
			{#if error}
				<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
					<p>{error}</p>
					<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadAll(currentPage)}>Retry</button>
				</aside>
			{/if}

			<div class="table-wrap">
				<table class="table">
					<thead>
						<tr>
							{#if canManage}
								<th class="w-10">
									<input
										type="checkbox"
										class="checkbox"
										checked={items.length > 0 && batchSelectedIds.size === items.length}
										indeterminate={batchSelectedIds.size > 0 && batchSelectedIds.size < items.length}
										onchange={toggleBatchSelectAll}
										aria-label="Select all"
									/>
								</th>
							{/if}
							<th>Name</th>
							<th>Plugins</th>
							<th>Status</th>
							<th>Hosts</th>
							<th>Last Checked</th>
							{#if canManage}
								<th class="w-20"></th>
							{/if}
						</tr>
					</thead>
					<tbody>
						{#if loading}
							<tr>
								<td colspan={canManage ? 8 : 5} class="py-6 text-center">Loading...</td>
							</tr>
						{:else}
							{#each items as item (item.id)}
								<tr>
									{#if canManage}
										<td>
											<input
												type="checkbox"
												class="checkbox"
												checked={batchSelectedIds.has(item.id)}
												onchange={() => toggleBatchSelect(item.id)}
												aria-label="Select {item.name}"
											/>
										</td>
									{/if}
									<td>
										<a href="/software/{item.id}" class="hover:underline font-medium">{item.name}</a>
									</td>
									<td class="text-sm text-surface-500">
										{item.plugins.map((p) => pluginTypeNames.get(p) ?? p).join(', ') || '\u2014'}
									</td>
									<td>
										{#if item.discovery_state === 'pending'}
											<span class="badge preset-filled-warning-500">Pending</span>
										{:else}
											{#if item.update_available}
												{#if canManage}
													<button
														class="badge preset-filled-warning-500 cursor-pointer hover:opacity-80"
														onclick={() => openUpdateModal(item)}
													>
														Update Available
													</button>
												{:else}
													<span class="badge preset-filled-warning-500">Update Available</span>
												{/if}
											{/if}
											{#if !item.enabled}
												<span class="badge preset-tonal">Disabled</span>
											{/if}
											{#if !item.update_available && item.enabled}
												<span class="text-surface-400">—</span>
											{/if}
										{/if}
									</td>
									<td>{item.host_count}</td>
									<td>
										{formatDate(item.last_checked_at)}
										{#if item.latest_version}
											<span class="block text-xs text-surface-500" title={item.latest_version}
												>{formatVersion(item.latest_version)} available</span
											>
										{/if}
									</td>
									{#if canManage}
										<td>
											<div class="actions-menu">
												<button
													class="btn btn-sm preset-tonal"
													aria-label="Actions for {item.name}"
													onclick={(e) => {
														e.stopPropagation();
														toggleMenu(item.id, e.currentTarget);
													}}
												>
													&#8943;
												</button>
											</div>
										</td>
									{/if}
								</tr>
							{:else}
								<tr>
									<td colspan={canManage ? 8 : 5} class="py-8 text-center">
										{#if activeTab === 'pending'}
											<p class="text-lg font-medium">No pending items</p>
											<p class="mt-1 text-sm text-surface-500">No discovered software awaiting review.</p>
										{:else if activeTab === 'active'}
											<p class="text-lg font-medium">No active software</p>
											<p class="mt-1 text-sm text-surface-500">Register or approve software to start tracking.</p>
										{:else}
											<p class="text-lg font-medium">No software registered yet</p>
											<p class="mt-1 text-sm text-surface-500">Register a package to start tracking.</p>
										{/if}
									</td>
								</tr>
							{/each}
						{/if}
					</tbody>
				</table>
			</div>

			<Pagination {currentPage} {totalPages} total={totalItems} onPageChange={loadAll} />

			{#if canManage && batchSelectedIds.size > 0}
				<BatchActionBar
					selectedCount={batchSelectedIds.size}
					actions={batchActions}
					onaction={requestBatchAction}
					oncancel={() => batchSelectedIds.clear()}
				/>
			{/if}

			{#if batchConfirmAction}
				<ConfirmDialog
					title="Batch {batchConfirmAction}"
					messagePrefix="Are you sure you want to {batchConfirmAction}"
					entityName="{batchSelectedIds.size} software item(s)"
					confirmLabel={submitting
						? 'Processing...'
						: batchConfirmAction === 'approve'
							? 'Approve'
							: batchConfirmAction === 'ignore'
								? 'Ignore'
								: 'Delete'}
					confirmClass={batchConfirmAction === 'approve'
						? 'preset-filled-success-500'
						: batchConfirmAction === 'ignore'
							? 'preset-filled-warning-500'
							: 'preset-filled-error-500'}
					confirmDisabled={submitting}
					onconfirm={executeBatchAction}
					oncancel={() => (batchConfirmAction = null)}
				/>
			{/if}

			{#if batchResult}
				<BatchResultDialog title="Batch Action Results" response={batchResult} onclose={() => (batchResult = null)} />
			{/if}

			{#if openMenuId}
				{@const item = items.find((i) => i.id === openMenuId)}
				{#if item}
					<ContextMenu top={menuPos.top} left={menuPos.left} onclose={closeMenu}>
						{#if item.discovery_state === 'pending'}
							<li>
								<button
									class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800"
									role="menuitem"
									tabindex="-1"
									onclick={() => openEditModal(item)}
								>
									Edit
								</button>
							</li>
							<li>
								<button
									class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800 disabled:cursor-not-allowed disabled:opacity-50"
									role="menuitem"
									tabindex="-1"
									disabled={approvingId === item.id}
									onclick={() => approveItem(item)}
								>
									{approvingId === item.id ? 'Approving…' : 'Approve'}
								</button>
							</li>
							<li>
								<button
									class="w-full rounded-md px-3 py-2 text-left text-sm text-warning-600 dark:text-warning-400 hover:bg-surface-200 dark:hover:bg-surface-800"
									role="menuitem"
									tabindex="-1"
									onclick={() => requestIgnore(item)}
								>
									Ignore
								</button>
							</li>
							<li>
								<button
									class="w-full rounded-md px-3 py-2 text-left text-sm text-error-500 hover:bg-surface-200 dark:hover:bg-surface-800"
									role="menuitem"
									tabindex="-1"
									onclick={() => requestDelete(item)}
								>
									Delete
								</button>
							</li>
						{:else}
							<li>
								<button
									class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800"
									role="menuitem"
									tabindex="-1"
									onclick={() => openEditModal(item)}
								>
									Edit
								</button>
							</li>
							<li>
								<button
									class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800 disabled:cursor-not-allowed disabled:opacity-50"
									role="menuitem"
									tabindex="-1"
									disabled={checkingVersionsId === item.id}
									onclick={() => triggerVersionCheck(item)}
								>
									{checkingVersionsId === item.id ? 'Checking…' : 'Check Versions'}
								</button>
							</li>
							<li>
								<button
									class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800"
									role="menuitem"
									tabindex="-1"
									onclick={() => openAssignModal(item)}
								>
									Assign to Host
								</button>
							</li>
							<li>
								<button
									class="w-full rounded-md px-3 py-2 text-left text-sm text-error-500 hover:bg-surface-200 dark:hover:bg-surface-800"
									role="menuitem"
									tabindex="-1"
									onclick={() => requestDelete(item)}
								>
									Delete
								</button>
							</li>
						{/if}
					</ContextMenu>
				{/if}
			{/if}

			{#if confirmDelete}
				<ConfirmDialog
					title="Delete Software Item"
					messagePrefix="Are you sure you want to delete"
					entityName={confirmDelete.name}
					confirmLabel={submitting ? 'Deleting...' : 'Delete'}
					confirmClass="preset-filled-error-500"
					confirmDisabled={submitting}
					onconfirm={executeDelete}
					oncancel={() => (confirmDelete = null)}
				/>
			{/if}

			{#if confirmIgnore}
				<Modal title="Ignore Software Item" onclose={() => (confirmIgnore = null)}>
					<p class="text-sm">
						Permanently ignore <strong>{confirmIgnore.name}</strong> from future autodiscovery runs?
					</p>
					<p class="mt-2 text-sm text-surface-500">
						An ignore rule will be created so this package is not re-discovered. You can manage ignore rules under <a
							href="/software?tab=ignores"
							class="underline">Software → Ignore Rules</a
						>.
					</p>
					{#snippet footer()}
						<button class="btn preset-tonal-surface" onclick={() => (confirmIgnore = null)}>Cancel</button>
						<button class="btn preset-filled-warning-500" disabled={ignoreSubmitting} onclick={executeIgnore}>
							{ignoreSubmitting ? 'Ignoring…' : 'Ignore'}
						</button>
					{/snippet}
				</Modal>
			{/if}

			{#if assignItem}
				<AssignToHostModal
					softwareItemId={assignItem.id}
					softwareItemName={assignItem.name}
					onclose={() => (assignItem = null)}
					onsuccess={() => {
						assignItem = null;
						loadAll(currentPage);
					}}
				/>
			{/if}

			{#if showAddModal}
				<AddSoftwareModal
					onclose={() => (showAddModal = false)}
					onsuccess={() => {
						showAddModal = false;
						loadAll(1);
					}}
				/>
			{/if}
		{:else if activeTab === 'ignores'}
			<IgnoreRulesTab />
		{:else}
			{#each tabExtensions as ext (ext.id)}
				{#if activeTab === ext.id}
					<ExtensionTabContent extension={ext} />
				{/if}
			{/each}
		{/if}
	{/if}
{/if}

{#if updateModalItem}
	<Modal title="Trigger Update — {updateModalItem.name}" onclose={() => (updateModalItem = null)} maxWidth="max-w-lg">
		{#if updateModalLoading}
			<p class="text-sm text-surface-500">Loading hosts…</p>
		{:else if updateModalDetail}
			<p class="text-sm text-surface-500 mb-2">
				Select the hosts to update. Hosts that are already up to date cannot be selected.
			</p>
			<ul class="space-y-2">
				{#each updateModalDetail.hosts as host (host.host_id)}
					{@const upToDate = !host.update_available}
					<li class="flex items-start gap-3 {upToDate ? 'opacity-50' : ''}">
						<input
							type="checkbox"
							class="checkbox mt-0.5"
							disabled={upToDate}
							checked={selectedHostIds.has(host.host_id)}
							onchange={(e) => {
								const next = new Set(selectedHostIds);
								if ((e.target as HTMLInputElement).checked) {
									next.add(host.host_id);
								} else {
									next.delete(host.host_id);
								}
								selectedHostIds = next;
							}}
						/>
						<div class="flex-1 min-w-0">
							<p class="text-sm font-medium truncate">
								{host.friendly_name || host.hostname}
							</p>
							{#if upToDate}
								<p class="text-xs text-surface-400">Already up to date</p>
							{:else}
								<p class="text-xs text-surface-500">
									{host.installed_version ?? 'unknown'} → {host.latest_version}
								</p>
							{/if}
						</div>
					</li>
				{/each}
			</ul>
		{/if}
		{#snippet footer()}
			<button class="btn preset-tonal-surface" onclick={() => (updateModalItem = null)}> Cancel </button>
			<button
				class="btn preset-filled-primary-500"
				disabled={selectedHostIds.size === 0 || triggeringUpdate}
				onclick={executeUpdate}
			>
				{triggeringUpdate ? 'Triggering…' : `Update ${selectedHostIds.size} host(s)`}
			</button>
		{/snippet}
	</Modal>
{/if}

{#if editItem}
	<Modal title="Edit Software Item" onclose={() => (editItem = null)}>
		<label class="label">
			<span>Name</span>
			<input class="input" type="text" bind:value={editForm.name} />
		</label>

		<label class="flex items-center gap-3">
			<input class="checkbox" type="checkbox" bind:checked={editForm.enabled} />
			<span>Enabled</span>
		</label>

		{#snippet footer()}
			<button class="btn preset-tonal-surface" onclick={() => (editItem = null)}>Cancel</button>
			<button class="btn preset-filled-primary-500" onclick={executeEdit} disabled={editSubmitting}>
				{editSubmitting ? 'Saving...' : 'Save'}
			</button>
		{/snippet}
	</Modal>
{/if}
