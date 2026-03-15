<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { SvelteSet, SvelteMap } from 'svelte/reactivity';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import {
		getSoftwareItems,
		deleteSoftwareItem,
		checkSoftwareItemVersions,
		updateSoftwareItem,
		listPluginTypes,
		getSoftwareItem,
		triggerSoftwareUpdate,
		batchSoftwareItems,
		executeBatchChunked
	} from '$lib/api';
	import {
		formatDate,
		formatVersion,
		parseUrlPage,
		isValidLogoUrl,
		resolveDisplayVersion,
		nextValidPage
	} from '$lib/utils';
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
	import { Permission, hasAnyPermission } from '$lib/types';
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
	let activeTab: string = $state(page.url.searchParams.get('tab') ?? 'featured');
	let showUpdatableOnly: boolean = $state(page.url.searchParams.get('updatable') === 'true');
	let pluginTypeFilter: string = $state(page.url.searchParams.get('plugin_type') ?? '');
	let pluginTypeOptions: { plugin_type: string; display_name: string }[] = $state([]);

	const tabExtensions = $derived(getTabExtensions('software'));
	const isItemsTab = $derived(activeTab === 'all' || activeTab === 'featured' || activeTab === 'unfeatured');
	let editItem: { id: string; name: string; featured: boolean; icon_url?: string | null } | null = $state(null);
	let editForm = $state({ name: '', featured: true, icon_url: '' });
	let editSubmitting: boolean = $state(false);
	let pluginTypeNames: Map<string, string> = $state(new Map());
	let updateModalItem: SoftwareItemResponse | null = $state(null);
	let updateModalDetail: SoftwareItemDetailResponse | null = $state(null);
	let updateModalLoading: boolean = $state(false);
	let selectedHostIds: Set<string> = $state(new Set());
	let triggeringUpdate: boolean = $state(false);
	let batchSelectedIds = new SvelteSet<string>();
	const batchSelectedItemsMap = new SvelteMap<string, SoftwareItemResponse>();
	let batchConfirmAction: string | null = $state(null);
	let batchResult: BatchActionResponse | null = $state(null);
	let selectingAllPages = $state(false);

	const allBatchPageSelected = $derived(items.length > 0 && items.every((i) => batchSelectedIds.has(i.id)));

	const selectAllPagesInfo = $derived(
		isItemsTab && allBatchPageSelected && totalItems > items.length && batchSelectedIds.size < totalItems
			? { total: totalItems, loading: selectingAllPages, onSelect: selectAllSoftwarePages }
			: undefined
	);

	const canView = $derived(getUser()?.permissions.includes(Permission.ViewSoftware) ?? false);
	const canManage = $derived(
		hasAnyPermission(
			getUser(),
			Permission.CreateSoftware,
			Permission.UpdateSoftware,
			Permission.DeleteSoftware,
			Permission.TriggerChecks,
			Permission.TriggerUpdates
		)
	);
	const canTriggerChecks = $derived(getUser()?.permissions.includes(Permission.TriggerChecks) ?? false);

	const batchActions = $derived.by(() => {
		const selected = [...batchSelectedItemsMap.values()];
		const acts: { id: string; label: string; destructive?: boolean }[] = [];
		if (selected.some((i) => !i.featured)) {
			acts.push({ id: 'feature', label: 'Feature' });
		}
		if (selected.some((i) => i.featured)) {
			acts.push({ id: 'unfeature', label: 'Unfeature' });
		}
		if (selected.some((i) => i.update_available)) {
			acts.push({ id: 'update-all', label: 'Update All' });
		}
		if (canTriggerChecks) {
			acts.push({ id: 'check-version', label: 'Check Version' });
		}
		acts.push({ id: 'delete', label: 'Delete', destructive: true });
		return acts;
	});

	let refreshInterval: ReturnType<typeof setInterval> | null = null;
	let unsubscribers: (() => void)[] = [];

	$effect(() => {
		const parts: string[] = [];
		if (activeTab !== 'all') parts.push(`tab=${activeTab}`);
		if (isItemsTab && showUpdatableOnly) parts.push('updatable=true');
		if (isItemsTab && pluginTypeFilter) parts.push(`plugin_type=${encodeURIComponent(pluginTypeFilter)}`);
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
				subscribeToEvent('version_check_completed', () => loadAll(currentPage, true)),
				subscribeToEvent('update_completed', () => loadAll(currentPage, true))
			);
			refreshInterval = setInterval(() => {
				if (document.visibilityState === 'visible') loadAll(currentPage, true);
			}, 300_000);
		}
		listPluginTypes()
			.then((types) => {
				pluginTypeNames = new Map(types.map((t) => [t.plugin_type, t.display_name]));
				pluginTypeOptions = [...types].sort((a, b) => a.display_name.localeCompare(b.display_name));
			})
			.catch(() => {
				// Non-fatal: raw plugin type keys will show as fallback
			});
	});

	onDestroy(() => {
		for (const unsub of unsubscribers) unsub();
		if (refreshInterval) clearInterval(refreshInterval);
	});

	function featuredFilter(): boolean | undefined {
		if (activeTab === 'featured') return true;
		if (activeTab === 'unfeatured') return false;
		return undefined;
	}

	async function loadAll(page: number, background = false) {
		if (!background) {
			loading = true;
			error = null;
		}
		try {
			const result = await getSoftwareItems(
				page,
				undefined,
				featuredFilter(),
				undefined,
				showUpdatableOnly ? true : undefined,
				pluginTypeFilter || undefined
			);
			items = result.items;
			for (const item of result.items) {
				if (batchSelectedIds.has(item.id)) batchSelectedItemsMap.set(item.id, item);
			}
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
		batchSelectedIds.clear();
		batchSelectedItemsMap.clear();
		currentPage = 1;
		activeTab = tab;
		if (tab === 'all' || tab === 'featured' || tab === 'unfeatured') {
			loadAll(1);
		} else {
			showUpdatableOnly = false;
			pluginTypeFilter = '';
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
		editItem = { id: item.id, name: item.name, featured: item.featured, icon_url: item.icon_url };
		editForm = { name: item.name, featured: item.featured, icon_url: item.icon_url ?? '' };
	}

	async function toggleFeatured(item: SoftwareItemResponse) {
		try {
			await updateSoftwareItem(item.id, { featured: !item.featured });
			items = items.map((i) => (i.id === item.id ? { ...i, featured: !i.featured } : i));
			showSuccess(`"${item.name}" ${item.featured ? 'unfeatured' : 'featured'}.`);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to update item');
		}
	}

	async function executeEdit() {
		if (!editItem || editSubmitting) return;
		editSubmitting = true;
		try {
			const trimmedIcon = editForm.icon_url.trim();
			// null clears, string sets, undefined keeps existing
			const icon_url = trimmedIcon === '' ? (editItem.icon_url ? null : undefined) : trimmedIcon;
			const updated = await updateSoftwareItem(editItem.id, {
				name: editForm.name || undefined,
				featured: editForm.featured,
				icon_url
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
			await loadAll(currentPage);
			const p = nextValidPage(currentPage, totalPages);
			if (p !== null) await loadAll(p);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to delete software item.');
			loadAll(currentPage);
		} finally {
			submitting = false;
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
		if (allBatchPageSelected) {
			for (const item of items) {
				batchSelectedIds.delete(item.id);
				batchSelectedItemsMap.delete(item.id);
			}
		} else {
			for (const item of items) {
				batchSelectedIds.add(item.id);
				batchSelectedItemsMap.set(item.id, item);
			}
		}
	}

	function toggleBatchSelect(id: string) {
		if (batchSelectedIds.has(id)) {
			batchSelectedIds.delete(id);
			batchSelectedItemsMap.delete(id);
		} else {
			batchSelectedIds.add(id);
			const item = items.find((i) => i.id === id);
			if (item) batchSelectedItemsMap.set(id, item);
		}
	}

	function requestBatchAction(actionId: string) {
		if (actionId === 'check-version') {
			executeBatchCheckVersions();
		} else {
			batchConfirmAction = actionId;
		}
	}

	async function selectAllSoftwarePages() {
		selectingAllPages = true;
		try {
			let p = 1;
			while (true) {
				const result = await getSoftwareItems(
					p,
					100,
					featuredFilter(),
					undefined,
					showUpdatableOnly ? true : undefined,
					pluginTypeFilter || undefined
				);
				for (const item of result.items) {
					batchSelectedIds.add(item.id);
					batchSelectedItemsMap.set(item.id, item);
				}
				if (p >= result.total_pages) break;
				p++;
			}
		} catch {
			showError('Failed to select all items');
		} finally {
			selectingAllPages = false;
		}
	}

	async function executeBatchCheckVersions() {
		if (submitting) return;
		submitting = true;
		const ids = [...batchSelectedIds];
		let succeeded = 0;
		let failed = 0;
		await Promise.allSettled(
			ids.map(async (id) => {
				try {
					await checkSoftwareItemVersions(id);
					succeeded++;
				} catch {
					failed++;
				}
			})
		);
		if (succeeded > 0) showSuccess(`Version check triggered for ${succeeded} item(s).`);
		if (failed > 0) showError(`Failed to trigger version check for ${failed} item(s).`);
		batchSelectedIds.clear();
		batchSelectedItemsMap.clear();
		submitting = false;
	}

	async function executeBatchAction() {
		if (!batchConfirmAction || submitting) return;
		const action = batchConfirmAction;
		const ids = [...batchSelectedIds];
		batchConfirmAction = null;
		submitting = true;
		try {
			if (action === 'update-all') {
				const itemsWithUpdates = [...batchSelectedItemsMap.values()].filter((i) => i.update_available);
				if (itemsWithUpdates.length === 0) {
					showSuccess('None of the selected items have updates available.');
					batchSelectedIds.clear();
					submitting = false;
					return;
				}
				let totalTriggered = 0;
				let totalFailed = 0;
				for (const softwareItem of itemsWithUpdates) {
					try {
						const detail = await getSoftwareItem(softwareItem.id);
						const targets = detail.hosts.filter((h) => h.update_available && h.latest_version);
						const results = await Promise.allSettled(
							targets.map((h) => triggerSoftwareUpdate(softwareItem.id, h.host_id, { to_version: h.latest_version! }))
						);
						totalTriggered += results.filter((r) => r.status === 'fulfilled').length;
						totalFailed += results.filter((r) => r.status === 'rejected').length;
					} catch {
						totalFailed++;
					}
				}
				if (totalTriggered > 0)
					showSuccess(`Update triggered for ${totalTriggered} host(s) across ${itemsWithUpdates.length} item(s).`);
				if (totalFailed > 0) showError(`Failed to trigger update for ${totalFailed} host(s).`);
			} else {
				const response = await executeBatchChunked(action, ids, batchSoftwareItems);
				if (response.failed.length > 0) {
					batchResult = response;
				} else {
					showSuccess(`${response.succeeded.length} item(s) ${action}d successfully.`);
				}
			}
			batchSelectedIds.clear();
			batchSelectedItemsMap.clear();
			await loadAll(currentPage);
			const p = nextValidPage(currentPage, totalPages);
			if (p !== null) await loadAll(p);
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
					class="btn btn-sm {activeTab === 'featured' ? 'preset-filled-success-500' : 'preset-tonal'}"
					onclick={() => switchTab('featured')}
				>
					Featured
				</button>
				<button
					class="btn btn-sm {activeTab === 'unfeatured' ? 'preset-filled-primary-500' : 'preset-tonal'}"
					onclick={() => switchTab('unfeatured')}
				>
					Unfeatured
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
			{#if isItemsTab}
				<label class="flex items-center gap-2 text-sm cursor-pointer select-none">
					<input
						class="checkbox"
						type="checkbox"
						bind:checked={showUpdatableOnly}
						onchange={() => {
							currentPage = 1;
							loadAll(1);
						}}
					/>
					Updates available
				</label>
			{/if}
			{#if isItemsTab && pluginTypeOptions.length > 0}
				<select
					class="select text-sm"
					bind:value={pluginTypeFilter}
					onchange={() => {
						currentPage = 1;
						loadAll(1);
					}}
					aria-label="Filter by plugin"
				>
					<option value="">All plugins</option>
					{#each pluginTypeOptions as opt (opt.plugin_type)}
						<option value={opt.plugin_type}>{opt.display_name}</option>
					{/each}
				</select>
			{/if}
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
										checked={allBatchPageSelected}
										indeterminate={!allBatchPageSelected && batchSelectedIds.size > 0}
										onchange={toggleBatchSelectAll}
										aria-label="Select all"
									/>
								</th>
							{/if}
							<th>Name</th>
							<th>Plugins</th>
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
								<td colspan={canManage ? 7 : 4} class="py-6 text-center">Loading...</td>
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
										{#if canManage}
											<button
												class="mr-1 cursor-pointer text-lg leading-none transition-opacity hover:opacity-70"
												class:text-warning-500={item.featured}
												class:text-surface-400={!item.featured}
												title={item.featured ? 'Unfeature' : 'Feature'}
												onclick={(e) => {
													e.stopPropagation();
													toggleFeatured(item);
												}}
												aria-label="{item.featured ? 'Unfeature' : 'Feature'} {item.name}"
											>
												{item.featured ? '★' : '☆'}
											</button>
										{:else}
											<span class="mr-1 {item.featured ? 'text-warning-500' : 'text-surface-400'}"
												>{item.featured ? '★' : '☆'}</span
											>
										{/if}
										{#if isValidLogoUrl(item.icon_url)}
											<img
												src={item.icon_url}
												alt=""
												class="h-5 w-5 inline-block mr-1 rounded object-contain"
												referrerpolicy="no-referrer"
											/>
										{/if}
										<a href="/software/{item.id}" class="hover:underline font-medium">{item.name}</a>
										{#if item.update_available}
											{#if canManage}
												<button
													class="ml-1 badge preset-filled-warning-500 cursor-pointer hover:opacity-80"
													onclick={(e) => {
														e.stopPropagation();
														openUpdateModal(item);
													}}
												>
													Update Available
												</button>
											{:else}
												<span class="ml-1 badge preset-filled-warning-500">Update Available</span>
											{/if}
										{/if}
									</td>
									<td class="text-sm text-surface-500">
										{item.plugins.map((p) => pluginTypeNames.get(p) ?? p).join(', ') || '\u2014'}
									</td>
									<td>{item.host_count}</td>
									<td>
										{formatDate(item.last_checked_at)}
										{#if item.latest_version}
											<span class="block text-xs text-surface-500" title={item.latest_version}
												>{formatVersion(
													resolveDisplayVersion(
														item.latest_version,
														item.latest_release_metadata?.display_version as string | undefined
													)
												)} available</span
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
									<td colspan={canManage ? 7 : 4} class="py-8 text-center">
										{#if showUpdatableOnly}
											<p class="text-lg font-medium">No updates available</p>
											<p class="mt-1 text-sm text-surface-500">All software in this view is up to date.</p>
										{:else if pluginTypeFilter}
											<p class="text-lg font-medium">No matching software</p>
											<p class="mt-1 text-sm text-surface-500">No items are tracked using the selected plugin.</p>
										{:else if activeTab === 'featured'}
											<p class="text-lg font-medium">No featured software</p>
											<p class="mt-1 text-sm text-surface-500">
												Feature software items to highlight them on the dashboard.
											</p>
										{:else if activeTab === 'unfeatured'}
											<p class="text-lg font-medium">No unfeatured software</p>
											<p class="mt-1 text-sm text-surface-500">All software items are currently featured.</p>
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
					oncancel={() => {
						batchSelectedIds.clear();
						batchSelectedItemsMap.clear();
					}}
					selectAllPages={selectAllPagesInfo}
				/>
			{/if}

			{#if batchConfirmAction}
				<ConfirmDialog
					title={batchConfirmAction === 'update-all' ? 'Update All' : `Batch ${batchConfirmAction}`}
					messagePrefix={batchConfirmAction === 'update-all'
						? 'Trigger updates for all available updates across'
						: `Are you sure you want to ${batchConfirmAction}`}
					entityName="{batchSelectedIds.size} software item(s)"
					confirmLabel={submitting
						? 'Processing...'
						: batchConfirmAction === 'update-all'
							? 'Update All'
							: batchConfirmAction === 'feature'
								? 'Feature'
								: batchConfirmAction === 'unfeature'
									? 'Unfeature'
									: 'Delete'}
					confirmClass={batchConfirmAction === 'update-all' ||
					batchConfirmAction === 'feature' ||
					batchConfirmAction === 'unfeature'
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
						<li>
							<button
								class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800"
								role="menuitem"
								tabindex="-1"
								onclick={() => {
									toggleFeatured(item);
									closeMenu();
								}}
							>
								{item.featured ? 'Unfeature' : 'Feature'}
							</button>
						</li>
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
								{checkingVersionsId === item.id ? 'Checking...' : 'Check Versions'}
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
						{#if item.update_available}
							<li>
								<button
									class="w-full rounded-md px-3 py-2 text-left text-sm text-warning-600 dark:text-warning-400 hover:bg-surface-200 dark:hover:bg-surface-800"
									role="menuitem"
									tabindex="-1"
									onclick={() => openUpdateModal(item)}
								>
									Trigger Update
								</button>
							</li>
						{/if}
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
	<Modal title="Trigger Update -- {updateModalItem.name}" onclose={() => (updateModalItem = null)} maxWidth="max-w-lg">
		{#if updateModalLoading}
			<p class="text-sm text-surface-500">Loading hosts...</p>
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
									{host.installed_version ?? 'unknown'} -> {host.latest_version}
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
				{triggeringUpdate ? 'Triggering...' : `Update ${selectedHostIds.size} host(s)`}
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

		<label class="label">
			<span>Icon URL <span class="text-surface-400 font-normal">(optional, HTTPS)</span></span>
			<input class="input" type="text" bind:value={editForm.icon_url} placeholder="https://example.com/icon.png" />
			{#if editForm.icon_url.trim() && !isValidLogoUrl(editForm.icon_url.trim())}
				<p class="text-warning-500 text-xs">Icon URL must be a valid HTTPS URL.</p>
			{/if}
		</label>

		<label class="flex items-center gap-3">
			<input class="checkbox" type="checkbox" bind:checked={editForm.featured} />
			<span>Featured</span>
		</label>

		{#snippet footer()}
			<button class="btn preset-tonal-surface" onclick={() => (editItem = null)}>Cancel</button>
			<button class="btn preset-filled-primary-500" onclick={executeEdit} disabled={editSubmitting}>
				{editSubmitting ? 'Saving...' : 'Save'}
			</button>
		{/snippet}
	</Modal>
{/if}
