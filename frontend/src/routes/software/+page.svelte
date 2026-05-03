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
		getUpdateHistoryEntry,
		batchSoftwareItems,
		executeBatchChunked,
		previewSoftwareItemMerge,
		executeSoftwareItemMerge
	} from '$lib/api';
	import {
		parseUrlPage,
		isValidLogoUrl,
		nextValidPage,
		formatVersion,
		resolveDisplayVersion,
		isValidExternalUrl,
		formatDate
	} from '$lib/utils';
	import { connectInteractiveSession } from '$lib/interactive';
	import type { InteractiveConnectionState } from '$lib/interactive';
	import TerminalOutput from '$lib/components/TerminalOutput.svelte';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import { subscribeToEvent } from '$lib/stores/events.svelte';
	import { AdminEventType } from '$lib/sse';
	import AddSoftwareModal from '$lib/components/AddSoftwareModal.svelte';
	import AssignToHostModal from '$lib/components/AssignToHostModal.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import BatchActionBar from '$lib/components/BatchActionBar.svelte';
	import BatchResultDialog from '$lib/components/BatchResultDialog.svelte';
	import SoftwareMergeWizard from '$lib/components/SoftwareMergeWizard.svelte';
	import type {
		SoftwareItemResponse,
		SoftwareItemDetailResponse,
		SoftwareItemHostSummary,
		AttestationStatus,
		BatchActionResponse,
		MergeSoftwareItemSummary
	} from '$lib/types';
	import SurfaceReadPanel from '$lib/components/surfaces/SurfaceReadPanel.svelte';
	import { Permission, hasAnyPermission, hasPermissionValue } from '$lib/types';
	import {
		getSurfaceReadLoading,
		getSurfaceReadModel,
		getSurfaceReadRequested,
		getSurfacesBySlot,
		loadSurfaceReadModels
	} from '$lib/surfaces/registry.svelte';
	import { filterSurfacesByPermission, isSurfaceTabPending } from '$lib/surfaces/read-model';
	import {
		Callout,
		ContextMenuItem,
		ContextMenuShell,
		EmptyState,
		ModalShell,
		PageShell,
		ReleaseNotes,
		SectionCard,
		SoftwareGroupList,
		StatusBadge,
		TabStrip,
		type TabStripItem
	} from '$lib/components/ui';
	import { FormFieldRow, Input, Checkbox, Select } from '$lib/components/forms';
	import IgnoreRulesTab from './IgnoreRulesTab.svelte';
	import Button from '$lib/components/Button.svelte';

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

	const slotTabSurfaces = $derived(
		filterSurfacesByPermission(getSurfacesBySlot('software.tabs'), (requiredPermission) =>
			hasPermissionValue(getUser(), requiredPermission)
		)
	);
	const slotTabReads = $derived.by(() => {
		const result: Record<string, NonNullable<ReturnType<typeof getSurfaceReadModel>>> = {};
		for (const surface of slotTabSurfaces) {
			const read = getSurfaceReadModel(surface.surface_id);
			if (read) {
				result[surface.surface_id] = read;
			}
		}
		return result;
	});
	const showSurfaceSoftwareTabs = $derived(slotTabSurfaces.length > 0);
	const isItemsTab = $derived(activeTab === 'all' || activeTab === 'featured' || activeTab === 'unfeatured');
	const tabItems = $derived.by<TabStripItem[]>(() => {
		const items: TabStripItem[] = [
			{ id: 'all', label: 'All' },
			{ id: 'featured', label: 'Featured' },
			{ id: 'unfeatured', label: 'Unfeatured' },
			{ id: 'ignores', label: 'Ignore Rules' }
		];
		if (showSurfaceSoftwareTabs) {
			for (const surface of slotTabSurfaces) {
				items.push({ id: surface.surface_id, label: surface.label });
			}
		}
		return items;
	});
	let editItem: { id: string; name: string; featured: boolean; icon_url?: string | null } | null = $state(null);
	let editForm = $state({ name: '', featured: true, icon_url: '' });
	let editSubmitting: boolean = $state(false);
	let pluginTypeNames: Map<string, string> = $state(new Map());
	let itemDetailsById = new SvelteMap<string, SoftwareItemDetailResponse>();
	let itemDetailLoadingIds = new SvelteSet<string>();
	let collapsedGroupIds = new SvelteSet<string>();
	let expandedOverflowGroupIds = new SvelteSet<string>();
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
	let mergeModalOpen = $state(false);
	let mergeInitialCandidates: MergeSoftwareItemSummary[] = $state([]);
	let mergeSeedItemId: string | null = $state(null);
	let mergeInitialSearchQuery = $state('');
	let pendingMergeSuccessToast = $state(page.url.searchParams.get('merge_success') === '1');

	// Single-host update modal (confirmation + live terminal)
	let singleHostUpdateModal: {
		host: SoftwareItemHostSummary;
		toVersion: string;
		itemId: string;
		itemName: string;
	} | null = $state(null);
	let singleHostUpdateTriggering: boolean = $state(false);
	let pendingLiveHistoryId: string | null = $state(null);
	let pendingLiveHostName: string = $state('');
	let pendingLiveItemName: string = $state('');
	let liveModal: { updateHistoryId: string; hostName: string; itemName: string } | null = $state(null);
	let liveStartedAt: number | null = $state(null);
	let liveWsState: InteractiveConnectionState = $state('disconnected');
	let liveWsHandle: ReturnType<typeof connectInteractiveSession> | null = null;
	let liveStdinAttention: boolean = $state(false);
	let liveTerminalRef: TerminalOutput | undefined = $state(undefined);

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
	const canTriggerUpdates = $derived(getUser()?.permissions.includes(Permission.TriggerUpdates) ?? false);
	const canMergeSoftware = $derived(
		(getUser()?.permissions.includes(Permission.UpdateSoftware) ?? false) &&
			(getUser()?.permissions.includes(Permission.DeleteSoftware) ?? false)
	);
	const itemsEmptyState = $derived.by(() => {
		if (showUpdatableOnly) {
			return { title: 'No updates available', description: 'All software in this view is up to date.' };
		}
		if (pluginTypeFilter) {
			return { title: 'No matching software', description: 'No items are tracked using the selected plugin.' };
		}
		if (activeTab === 'featured') {
			return {
				title: 'No featured software',
				description: 'Feature software items to highlight them on the dashboard.'
			};
		}
		if (activeTab === 'unfeatured') {
			return {
				title: 'No unfeatured software',
				description: 'All software items are currently featured.'
			};
		}
		return { title: 'No software registered yet', description: 'Register a package to start tracking.' };
	});

	const batchActions = $derived.by(() => {
		const selected = [...batchSelectedItemsMap.values()];
		const acts: { id: string; label: string; destructive?: boolean }[] = [];
		if (selected.some((i) => !i.featured)) {
			acts.push({ id: 'feature', label: 'Feature' });
		}
		if (selected.some((i) => i.featured)) {
			acts.push({ id: 'unfeature', label: 'Unfeature' });
		}
		if (canTriggerUpdates && selected.some((i) => i.update_available)) {
			acts.push({ id: 'update-all', label: 'Update all' });
		}
		if (canTriggerChecks) {
			acts.push({ id: 'check-version', label: 'Check Version' });
		}
		if (canMergeSoftware && selected.length >= 2) {
			acts.push({ id: 'merge', label: 'Merge' });
		}
		acts.push({ id: 'delete', label: 'Delete', destructive: true });
		return acts;
	});

	let refreshInterval: ReturnType<typeof setInterval> | null = null;
	let unsubscribers: (() => void)[] = [];

	$effect(() => {
		const parts: string[] = [];
		if (pendingMergeSuccessToast) parts.push('merge_success=1');
		parts.push(`tab=${activeTab}`);
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

	$effect(() => {
		if (!pendingMergeSuccessToast) return;
		showSuccess('Software items merged.');
		pendingMergeSuccessToast = false;
	});

	$effect(() => {
		if (slotTabSurfaces.length === 0) {
			return;
		}
		void loadSurfaceReadModels(slotTabSurfaces.map((surface) => surface.surface_id));
	});

	$effect(() => {
		if (isItemsTab || activeTab === 'ignores') {
			return;
		}
		const isSurfaceTab = showSurfaceSoftwareTabs && slotTabSurfaces.some((surface) => surface.surface_id === activeTab);
		const isPendingSurfaceTab = isSurfaceTabPending({
			activeTab,
			slotSurfaces: slotTabSurfaces,
			readBySurface: slotTabReads,
			isReadRequested: getSurfaceReadRequested(activeTab),
			isReadLoading: getSurfaceReadLoading(activeTab)
		});
		if (!isSurfaceTab && !isPendingSurfaceTab) {
			activeTab = 'featured';
		}
	});

	onMount(() => {
		if (canView) {
			loadAll(currentPage);
			unsubscribers.push(
				subscribeToEvent(AdminEventType.SoftwareItemUpdated, () => loadAll(currentPage, true)),
				subscribeToEvent(AdminEventType.SoftwareItemCreated, () => loadAll(currentPage, true)),
				subscribeToEvent(AdminEventType.VersionCheckCompleted, () => loadAll(currentPage, true)),
				subscribeToEvent(AdminEventType.UpdateCompleted, () => loadAll(currentPage, true)),
				subscribeToEvent(AdminEventType.UpdateProtectionStarted, (data) => {
					if (data.update_history_id === pendingLiveHistoryId && pendingLiveHistoryId) {
						const histId = pendingLiveHistoryId;
						const hostName = pendingLiveHostName;
						const itemName = pendingLiveItemName;
						pendingLiveHistoryId = null;
						pendingLiveHostName = '';
						pendingLiveItemName = '';
						openLiveModal(histId, hostName, itemName);
					}
				}),
				subscribeToEvent(AdminEventType.UpdateStarted, (data) => {
					if (data.update_history_id === pendingLiveHistoryId && pendingLiveHistoryId) {
						const histId = pendingLiveHistoryId;
						const hostName = pendingLiveHostName;
						const itemName = pendingLiveItemName;
						pendingLiveHistoryId = null;
						pendingLiveHostName = '';
						pendingLiveItemName = '';
						openLiveModal(histId, hostName, itemName);
					}
				})
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
		liveWsHandle?.disconnect();
	});

	function featuredFilter(): boolean | undefined {
		if (activeTab === 'featured') return true;
		if (activeTab === 'unfeatured') return false;
		return undefined;
	}

	function softwareItemDetailIsStale(previous: SoftwareItemResponse | undefined, next: SoftwareItemResponse): boolean {
		if (!previous) {
			return false;
		}
		return (
			previous.updated_at !== next.updated_at ||
			previous.host_count !== next.host_count ||
			previous.latest_version !== next.latest_version ||
			previous.update_available !== next.update_available
		);
	}

	async function loadAll(page: number, background = false) {
		if (!background) {
			loading = true;
			error = null;
		}
		try {
			const previousById = new Map(items.map((item) => [item.id, item]));
			const result = await getSoftwareItems(
				page,
				undefined,
				featuredFilter(),
				undefined,
				showUpdatableOnly ? true : undefined,
				pluginTypeFilter || undefined
			);
			items = result.items;
			const visibleIds = new Set(result.items.map((item) => item.id));
			for (const detailId of itemDetailsById.keys()) {
				if (!visibleIds.has(detailId)) {
					itemDetailsById.delete(detailId);
					collapsedGroupIds.delete(detailId);
					expandedOverflowGroupIds.delete(detailId);
				}
			}
			const staleDetailIds = new Set(
				result.items
					.filter((item) => itemDetailsById.has(item.id) && softwareItemDetailIsStale(previousById.get(item.id), item))
					.map((item) => item.id)
			);
			for (const item of result.items) {
				if (batchSelectedIds.has(item.id)) batchSelectedItemsMap.set(item.id, item);
			}
			currentPage = result.page;
			totalPages = result.total_pages;
			totalItems = result.total;
			const detailLoad = primeVisibleItemDetails(result.items, staleDetailIds);
			if (background) {
				void detailLoad;
			} else {
				await detailLoad;
			}
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

	function toggleGroupCollapsed(itemId: string): void {
		if (collapsedGroupIds.has(itemId)) {
			collapsedGroupIds.delete(itemId);
		} else {
			collapsedGroupIds.add(itemId);
		}
	}

	function toggleGroupOverflow(itemId: string): void {
		if (expandedOverflowGroupIds.has(itemId)) {
			expandedOverflowGroupIds.delete(itemId);
		} else {
			expandedOverflowGroupIds.add(itemId);
		}
	}

	function cacheItemDetail(detail: SoftwareItemDetailResponse): void {
		itemDetailsById.set(detail.id, detail);
	}

	async function loadSoftwareItemDetail(
		itemId: string,
		{
			force = false,
			silent = false
		}: {
			force?: boolean;
			silent?: boolean;
		} = {}
	): Promise<SoftwareItemDetailResponse | undefined> {
		if (!force) {
			const existing = itemDetailsById.get(itemId);
			if (existing) {
				return existing;
			}
		}
		if (itemDetailLoadingIds.has(itemId)) {
			return itemDetailsById.get(itemId);
		}
		itemDetailLoadingIds.add(itemId);
		try {
			const detail = await getSoftwareItem(itemId);
			cacheItemDetail(detail);
			return detail;
		} catch (e) {
			if (!silent) {
				showError(e instanceof Error ? e.message : 'Failed to load software item details.');
			}
			return undefined;
		} finally {
			itemDetailLoadingIds.delete(itemId);
		}
	}

	async function primeVisibleItemDetails(
		rows: SoftwareItemResponse[],
		forceIds: Set<string> = new Set()
	): Promise<void> {
		await Promise.all(
			rows.map(async (item) => {
				const force = forceIds.has(item.id);
				if (!force && (itemDetailsById.has(item.id) || itemDetailLoadingIds.has(item.id))) {
					return;
				}
				await loadSoftwareItemDetail(item.id, { force, silent: true });
			})
		);
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
		if (editForm.icon_url.trim() && !isValidLogoUrl(editForm.icon_url.trim())) return;
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
		updateModalItem = null;
		updateModalDetail = null;
		selectedHostIds = new Set();
		updateModalLoading = true;
		try {
			const detail = (await loadSoftwareItemDetail(item.id, { force: true })) ?? (await getSoftwareItem(item.id));
			cacheItemDetail(detail);
			if (detail.hosts.length === 1) {
				const host = detail.hosts[0];
				const toVersion = host.latest_version ?? item.latest_version ?? '';
				singleHostUpdateModal = { host, toVersion, itemId: item.id, itemName: item.name };
			} else {
				updateModalItem = item;
				updateModalDetail = detail;
				selectedHostIds = new Set(detail.hosts.filter((h) => h.update_available).map((h) => h.host_id));
			}
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load host details.');
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
		let succeeded = 0;
		let failed = 0;
		for (const result of results) {
			if (result.status === 'rejected') {
				failed += 1;
				continue;
			}
			if (result.value.status === 'failed') {
				failed += 1;
				continue;
			}
			succeeded += 1;
		}
		if (succeeded > 0) showSuccess(`Update triggered for ${succeeded} host(s).`);
		if (failed > 0) showError(`Failed to trigger update for ${failed} host(s).`);
		triggeringUpdate = false;
		updateModalItem = null;
		loadAll(currentPage);
	}

	async function executeSingleHostUpdate() {
		if (!singleHostUpdateModal || singleHostUpdateTriggering || !canTriggerUpdates) return;
		singleHostUpdateTriggering = true;
		try {
			const { host, toVersion, itemId, itemName } = singleHostUpdateModal;
			const hostName = host.hostname;
			const res = await triggerSoftwareUpdate(itemId, host.host_id, { to_version: toVersion });
			singleHostUpdateModal = null;
			if (res.status === 'failed') {
				showError(`Update failed before dispatch — history ID: ${res.update_history_id}`);
				loadAll(currentPage);
				return;
			}
			showSuccess(`Update triggered — history ID: ${res.update_history_id}`);
			pendingLiveHistoryId = res.update_history_id;
			pendingLiveHostName = hostName;
			pendingLiveItemName = itemName;
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to trigger update');
		} finally {
			singleHostUpdateTriggering = false;
		}
	}

	function openLiveModal(updateHistoryId: string, hostName: string, itemName: string) {
		liveModal = { updateHistoryId, hostName, itemName };
		liveStartedAt = Date.now();
		liveWsState = 'connecting';
		liveStdinAttention = false;
		setTimeout(() => {
			liveWsHandle = connectInteractiveSession(updateHistoryId, {
				onOutput: (line) => {
					liveTerminalRef?.write(line.text);
				},
				onCompleted: () => {
					liveStdinAttention = false;
					loadAll(currentPage, true);
				},
				onStdinAttention: () => {
					liveStdinAttention = true;
				},
				onStateChange: (state) => {
					liveWsState = state;
				},
				onError: (err) => {
					showError(`Interactive session error: ${err}`);
					const historyId = liveModal?.updateHistoryId;
					if (!historyId) return;
					liveWsState = 'connecting';
					void (async () => {
						try {
							const entry = await getUpdateHistoryEntry(historyId);
							if (liveModal?.updateHistoryId !== historyId) return;
							if (entry.output) {
								liveTerminalRef?.write(entry.output);
							}
							if (entry.status === 'completed' || entry.status === 'failed') {
								liveWsState = 'completed';
								loadAll(currentPage, true);
							} else {
								liveWsState = 'error';
							}
						} catch {
							if (liveModal?.updateHistoryId === historyId) {
								liveWsState = 'error';
							}
						}
					})();
				}
			});
		}, 0);
	}

	function closeLiveModal() {
		liveWsHandle?.disconnect();
		liveWsHandle = null;
		liveModal = null;
		liveStartedAt = null;
		liveWsState = 'disconnected';
		liveStdinAttention = false;
	}

	function liveModalStatusLabel(): string {
		if (liveWsState === 'connected') return 'Live';
		if (liveWsState === 'connecting') return 'Connecting';
		if (liveWsState === 'completed') return 'Completed';
		if (liveWsState === 'error') return 'Error';
		return 'Captured';
	}

	function liveModalStatusTone(): 'neutral' | 'info' | 'success' | 'warning' | 'danger' {
		if (liveWsState === 'connected') return 'success';
		if (liveWsState === 'connecting') return 'warning';
		if (liveWsState === 'completed') return 'success';
		if (liveWsState === 'error') return 'danger';
		return 'neutral';
	}

	function liveDurationLabel(startedAt: number | null): string {
		if (!startedAt) return '0m';
		const elapsedSeconds = Math.max(0, Math.floor((Date.now() - startedAt) / 1000));
		if (elapsedSeconds < 60) return '<1m';
		const minutes = Math.floor(elapsedSeconds / 60);
		if (minutes < 60) return `${minutes}m`;
		const hours = Math.floor(minutes / 60);
		const remainingMinutes = minutes % 60;
		return remainingMinutes === 0 ? `${hours}h` : `${hours}h ${remainingMinutes}m`;
	}

	function relativeStartedLabel(startedAt: number | null): string {
		if (!startedAt) return 'unknown';
		const deltaMs = Date.now() - startedAt;
		if (!Number.isFinite(deltaMs) || deltaMs < 0) return formatDate(new Date(startedAt).toISOString());
		const deltaSeconds = Math.floor(deltaMs / 1000);
		if (deltaSeconds < 60) return 'just now';
		const deltaMinutes = Math.floor(deltaSeconds / 60);
		if (deltaMinutes < 60) return `${deltaMinutes}m ago`;
		const deltaHours = Math.floor(deltaMinutes / 60);
		if (deltaHours < 24) return `${deltaHours}h ago`;
		const deltaDays = Math.floor(deltaHours / 24);
		if (deltaDays < 7) return `${deltaDays}d ago`;
		return formatDate(new Date(startedAt).toISOString());
	}

	function liveMetadata(hostName: string): string {
		const startedLabel = relativeStartedLabel(liveStartedAt);
		return `${hostName} · started ${startedLabel} · ${liveDurationLabel(liveStartedAt)}`;
	}

	function liveInlineBadges(): Array<{ id: string; tone: 'warning' | 'info'; label: string }> {
		if (!liveModal) return [];
		return [
			{
				id: 'interactive',
				tone: liveStdinAttention ? 'warning' : 'info',
				label: 'Interactive terminal'
			}
		];
	}

	function liveTerminalActions(): Array<{
		id: string;
		label: string;
		title: string;
		tone: 'danger';
		onclick: () => void;
	}> {
		if (liveWsState === 'connected' || liveWsState === 'connecting') {
			return [
				{
					id: 'sigint',
					label: 'Ctrl+C',
					title: 'Send Ctrl+C (SIGINT)',
					tone: 'danger',
					onclick: () => liveWsHandle?.sendSignal(2)
				}
			];
		}
		return [];
	}

	interface ReleaseMeta {
		release_url?: string;
		release_notes?: string;
		attestation_status?: AttestationStatus;
		display_version?: string;
	}

	function getReleaseMeta(host: SoftwareItemHostSummary): ReleaseMeta | null {
		const meta = host.latest_release_metadata;
		if (!meta) return null;
		const knownStatuses: AttestationStatus[] = ['Verified', 'NotFound', 'Unverified'];
		const rawStatus = meta.attestation_status;
		const attestation_status: AttestationStatus | undefined =
			typeof rawStatus === 'string' && knownStatuses.includes(rawStatus as AttestationStatus)
				? (rawStatus as AttestationStatus)
				: undefined;
		return {
			release_url: typeof meta.release_url === 'string' ? meta.release_url : undefined,
			release_notes: typeof meta.release_notes === 'string' ? meta.release_notes : undefined,
			attestation_status,
			display_version: typeof meta.display_version === 'string' ? meta.display_version : undefined
		};
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

	function toMergeSummary(item: SoftwareItemResponse): MergeSoftwareItemSummary {
		return {
			id: item.id,
			name: item.name,
			host_count: item.host_count,
			plugins: item.plugins
		};
	}

	async function searchMergeCandidates(query: string): Promise<MergeSoftwareItemSummary[]> {
		const result = await getSoftwareItems(1, 25, undefined, undefined, undefined, undefined, query);
		return result.items.map(toMergeSummary);
	}

	function openBatchMerge() {
		if (!canMergeSoftware) return;
		const selected = [...batchSelectedItemsMap.values()];
		if (selected.length < 2) return;
		mergeInitialCandidates = selected.map(toMergeSummary);
		mergeSeedItemId = null;
		mergeInitialSearchQuery = '';
		mergeModalOpen = true;
	}

	function openSingleItemMerge(item: SoftwareItemResponse) {
		if (!canMergeSoftware) return;
		closeMenu();
		mergeInitialCandidates = [toMergeSummary(item)];
		mergeSeedItemId = item.id;
		mergeInitialSearchQuery = item.name;
		mergeModalOpen = true;
	}

	function requestBatchAction(actionId: string) {
		if (actionId === 'merge') {
			openBatchMerge();
		} else if (actionId === 'check-version') {
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
						for (const result of results) {
							if (result.status === 'rejected') {
								totalFailed += 1;
								continue;
							}
							if (result.value.status === 'failed') {
								totalFailed += 1;
								continue;
							}
							totalTriggered += 1;
						}
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
	<PageShell title="Software" description="Track software versions and trigger controlled updates across hosts.">
		{#if !canView}
			<Callout tone="danger" title="Access denied" message="You do not have permission to view software items." />
		{:else}
			<TabStrip
				items={tabItems}
				activeId={activeTab}
				ariaLabel="Software tabs"
				idBase="software"
				onSelect={switchTab}
			/>

			{#if isItemsTab}
				<div
					class="rounded-card border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm"
					data-ui="software-route-groups"
				>
					<header
						class="flex flex-col gap-3 border-b border-[var(--border-subtle)] card-padding md:flex-row md:items-center md:justify-between"
					>
						<div class="flex flex-wrap items-center gap-3">
							{#if canManage}
								<div class="flex cursor-pointer select-none items-center gap-2 text-sm">
									<Checkbox
										id="software-batch-select-all"
										checked={allBatchPageSelected}
										indeterminate={!allBatchPageSelected && batchSelectedIds.size > 0}
										onchange={toggleBatchSelectAll}
									/>
									<label for="software-batch-select-all" class="cursor-pointer select-none">Select all</label>
								</div>
								<span class="h-4 w-px bg-[var(--border-subtle)]" aria-hidden="true"></span>
							{/if}
							<label class="flex cursor-pointer select-none items-center gap-2 text-sm">
								<Checkbox
									id="software-filter-updatable-only"
									bind:checked={showUpdatableOnly}
									onchange={() => {
										currentPage = 1;
										loadAll(1);
									}}
								/>
								Updates available
							</label>
							{#if pluginTypeOptions.length > 0}
								<Select
									id="software-plugin-filter"
									width="auto"
									bind:value={pluginTypeFilter}
									aria-label="Filter by plugin"
									options={[
										{ value: '', label: 'All plugins' },
										...pluginTypeOptions.map((opt) => ({
											value: opt.plugin_type,
											label: opt.display_name
										}))
									]}
									onchange={() => {
										currentPage = 1;
										loadAll(1);
									}}
								/>
							{/if}
						</div>
						{#if canManage}
							<div class="shrink-0">
								<Button variant="primary" size="sm" onclick={() => (showAddModal = true)}>Add Software</Button>
							</div>
						{/if}
					</header>
					{#if error}
						<div class="content-padding">
							<Callout tone="danger" title="Unable to load software items" message={error}>
								<Button variant="primary" size="sm" class="mt-3" onclick={() => loadAll(currentPage)}>Retry</Button>
							</Callout>
						</div>
					{:else if loading}
						<p class="px-4 py-8 text-center text-sm text-[var(--text-secondary)]">Loading software items...</p>
					{:else if items.length === 0}
						<div class="px-4 py-8 text-center">
							<EmptyState title={itemsEmptyState.title} description={itemsEmptyState.description} />
						</div>
					{:else}
						<SoftwareGroupList
							{items}
							{itemDetailsById}
							{itemDetailLoadingIds}
							{collapsedGroupIds}
							{expandedOverflowGroupIds}
							{batchSelectedIds}
							{canManage}
							{canTriggerUpdates}
							{pluginTypeNames}
							{totalItems}
							{currentPage}
							{totalPages}
							onToggleGroup={toggleGroupCollapsed}
							onToggleOverflow={toggleGroupOverflow}
							onToggleBatch={toggleBatchSelect}
							onOpenMenu={toggleMenu}
							onOpenUpdateModal={openUpdateModal}
							onPageChange={loadAll}
							onToggleFeatured={toggleFeatured}
						/>
					{/if}
				</div>

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
						confirmVariant={batchConfirmAction === 'update-all' ||
						batchConfirmAction === 'feature' ||
						batchConfirmAction === 'unfeature'
							? 'primary'
							: 'danger'}
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
						<ContextMenuShell top={menuPos.top} left={menuPos.left} onclose={closeMenu}>
							<li>
								<ContextMenuItem
									label={item.featured ? 'Unfeature' : 'Feature'}
									onclick={() => {
										toggleFeatured(item);
										closeMenu();
									}}
								/>
							</li>
							<li>
								<ContextMenuItem label="Edit" onclick={() => openEditModal(item)} />
							</li>
							<li>
								<ContextMenuItem
									label={checkingVersionsId === item.id ? 'Checking...' : 'Check Versions'}
									disabled={checkingVersionsId === item.id}
									onclick={() => triggerVersionCheck(item)}
								/>
							</li>
							<li>
								<ContextMenuItem label="Assign to Host" onclick={() => openAssignModal(item)} />
							</li>
							{#if item.update_available && canTriggerUpdates}
								<li>
									<ContextMenuItem label="Trigger Update" onclick={() => openUpdateModal(item)} />
								</li>
							{/if}
							{#if canMergeSoftware}
								<li>
									<ContextMenuItem label="Merge..." onclick={() => openSingleItemMerge(item)} />
								</li>
							{/if}
							<li>
								<ContextMenuItem label="Delete" destructive onclick={() => requestDelete(item)} />
							</li>
						</ContextMenuShell>
					{/if}
				{/if}

				{#if confirmDelete}
					<ConfirmDialog
						title="Delete Software Item"
						messagePrefix="Are you sure you want to delete"
						entityName={confirmDelete.name}
						confirmLabel={submitting ? 'Deleting...' : 'Delete'}
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

				{#if mergeModalOpen}
					<SoftwareMergeWizard
						candidates={mergeInitialCandidates}
						seedItemId={mergeSeedItemId}
						searchCandidates={searchMergeCandidates}
						initialSearchQuery={mergeSeedItemId ? mergeInitialSearchQuery : undefined}
						previewMerge={previewSoftwareItemMerge}
						executeMerge={executeSoftwareItemMerge}
						onclose={() => {
							mergeModalOpen = false;
							mergeInitialCandidates = [];
							mergeSeedItemId = null;
							mergeInitialSearchQuery = '';
						}}
						onsuccess={async () => {
							mergeModalOpen = false;
							mergeInitialCandidates = [];
							mergeSeedItemId = null;
							mergeInitialSearchQuery = '';
							batchSelectedIds.clear();
							batchSelectedItemsMap.clear();
							showSuccess('Software items merged.');
							await loadAll(currentPage);
							const p = nextValidPage(currentPage, totalPages);
							if (p !== null) await loadAll(p);
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
			{:else if showSurfaceSoftwareTabs}
				{#each slotTabSurfaces as surface (surface.surface_id)}
					{#if activeTab === surface.surface_id}
						<SectionCard title={surface.label}>
							<SurfaceReadPanel {surface} read={slotTabReads[surface.surface_id]} />
						</SectionCard>
					{/if}
				{/each}
			{/if}
		{/if}
	</PageShell>
{/if}

{#if updateModalItem}
	<ModalShell
		title="Trigger Update -- {updateModalItem.name}"
		onclose={() => (updateModalItem = null)}
		maxWidth="max-w-lg"
	>
		{#if updateModalLoading}
			<p class="text-sm text-[var(--text-muted)]">Loading hosts...</p>
		{:else if updateModalDetail}
			<p class="text-sm text-[var(--text-muted)] mb-2">
				Select the hosts to update. Hosts that are already up to date cannot be selected.
			</p>
			<ul class="space-y-2">
				{#each updateModalDetail.hosts as host (host.host_id)}
					{@const upToDate = !host.update_available}
					<li class="flex items-start gap-3 {upToDate ? 'opacity-50' : ''}">
						<Checkbox
							id="software-host-select-{host.host_id}"
							class="mt-0.5"
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
								<p class="text-table-header text-[var(--text-muted)]">Already up to date</p>
							{:else}
								<p class="text-table-header text-[var(--text-muted)]">
									{host.installed_version ?? 'unknown'} -> {host.latest_version}
								</p>
							{/if}
						</div>
					</li>
				{/each}
			</ul>
		{/if}
		{#snippet footer()}
			<Button variant="secondary" onclick={() => (updateModalItem = null)}>Cancel</Button>
			<Button variant="primary" loading={triggeringUpdate} disabled={selectedHostIds.size === 0} onclick={executeUpdate}
				>Update {selectedHostIds.size} host(s)</Button
			>
		{/snippet}
	</ModalShell>
{/if}

{#if singleHostUpdateModal}
	<ModalShell title="Confirm Update" onclose={() => (singleHostUpdateModal = null)} maxWidth="max-w-3xl">
		<p class="text-sm">
			Update <strong>{singleHostUpdateModal.itemName}</strong> on
			<strong>{singleHostUpdateModal.host.hostname}</strong>?
		</p>
		<div class="grid grid-cols-2 gap-4 text-sm">
			<div>
				<p class="text-[var(--text-muted)]">From</p>
				<p class="font-medium" title={singleHostUpdateModal.host.installed_version ?? undefined}>
					{formatVersion(
						resolveDisplayVersion(
							singleHostUpdateModal.host.installed_version,
							singleHostUpdateModal.host.installed_display_version
						),
						'unknown'
					)}
				</p>
			</div>
			<div>
				<p class="text-[var(--text-muted)]">To</p>
				<p class="font-medium" title={singleHostUpdateModal.toVersion}>
					{formatVersion(
						resolveDisplayVersion(
							singleHostUpdateModal.toVersion,
							getReleaseMeta(singleHostUpdateModal.host)?.display_version
						)
					)}
				</p>
			</div>
		</div>
		{@const meta = getReleaseMeta(singleHostUpdateModal.host)}
		{#if meta?.release_url && isValidExternalUrl(meta.release_url)}
			<p class="text-sm">
				<a
					href={meta.release_url}
					target="_blank"
					rel="noopener noreferrer"
					class="text-[var(--accent)] hover:underline">View release page ↗</a
				>
			</p>
		{/if}
		{#if meta?.release_notes}
			<details class="text-sm">
				<summary class="cursor-pointer text-[var(--text-muted)] hover:text-[var(--text-primary)]">Release notes</summary
				>
				<div class="mt-2 max-h-48 overflow-y-auto">
					<ReleaseNotes content={meta.release_notes} compact />
				</div>
			</details>
		{/if}
		{#if meta?.attestation_status === 'NotFound'}
			<Callout
				tone="warning"
				title="Attestation warning"
				message="No GitHub Actions attestation was found for this release. The artifacts may not be from the official workflow."
			/>
		{:else if meta?.attestation_status === 'Verified'}
			<div class="text-sm">
				<StatusBadge tone="success" label="GitHub Attestation Verified" />
			</div>
		{/if}
		{#snippet footer()}
			<Button variant="secondary" onclick={() => (singleHostUpdateModal = null)}>Cancel</Button>
			<Button variant="primary" loading={singleHostUpdateTriggering} onclick={executeSingleHostUpdate}
				>Trigger Update</Button
			>
		{/snippet}
	</ModalShell>
{/if}

{#if liveModal}
	<TerminalOutput
		bind:this={liveTerminalRef}
		open={true}
		title={`${liveModal.itemName} on ${liveModal.hostName}`}
		statusLabel={liveModalStatusLabel()}
		statusTone={liveModalStatusTone()}
		inlineBadges={liveInlineBadges()}
		metadata={liveMetadata(liveModal.hostName)}
		actions={liveTerminalActions()}
		onclose={closeLiveModal}
		onInput={liveWsState === 'connected' || liveWsState === 'connecting'
			? (data) => liveWsHandle?.sendInput(data)
			: undefined}
	/>
{/if}

{#if editItem}
	<ModalShell title="Edit Software Item" onclose={() => (editItem = null)}>
		<FormFieldRow label="Name" inputId="software-edit-name">
			<Input id="software-edit-name" type="text" bind:value={editForm.name} />
		</FormFieldRow>

		<FormFieldRow
			label="Icon URL"
			hint="Optional. Must be HTTPS when provided."
			inputId="software-edit-icon-url"
			error={editForm.icon_url.trim() && !isValidLogoUrl(editForm.icon_url.trim())
				? 'Icon URL must be a valid HTTPS URL.'
				: undefined}
		>
			<Input
				id="software-edit-icon-url"
				type="url"
				bind:value={editForm.icon_url}
				placeholder="https://example.com/icon.png"
				error={editForm.icon_url.trim() && !isValidLogoUrl(editForm.icon_url.trim())
					? 'Icon URL must be a valid HTTPS URL.'
					: undefined}
			/>
		</FormFieldRow>

		<FormFieldRow label="Featured">
			<Checkbox id="software-edit-featured" bind:checked={editForm.featured} />
		</FormFieldRow>

		{#snippet footer()}
			<Button variant="secondary" onclick={() => (editItem = null)}>Cancel</Button>
			<Button variant="primary" loading={editSubmitting} onclick={executeEdit}>Save</Button>
		{/snippet}
	</ModalShell>
{/if}
