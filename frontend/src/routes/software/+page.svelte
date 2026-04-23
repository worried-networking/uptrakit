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
		executeBatchChunked,
		previewSoftwareItemMerge,
		executeSoftwareItemMerge
	} from '$lib/api';
	import { formatVersion, parseUrlPage, isValidLogoUrl, resolveDisplayVersion, nextValidPage } from '$lib/utils';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import { subscribeToEvent } from '$lib/stores/events.svelte';
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
		ActionBadge,
		Callout,
		ContextMenuItem,
		ContextMenuShell,
		EmptyState,
		FormFieldRow,
		ModalShell,
		PageShell,
		PillBadge,
		SectionCard,
		StatusBadge,
		TableFooterBar,
		TabStrip,
		type TabStripItem
	} from '$lib/components/ui';
	import IgnoreRulesTab from './IgnoreRulesTab.svelte';
	import UpdateAllButton from '$lib/components/UpdateAllButton.svelte';
	import Button from '$lib/components/Button.svelte';
	import Input from '$lib/components/Input.svelte';
	import Checkbox from '$lib/components/Checkbox.svelte';

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

	function detailHosts(item: SoftwareItemResponse): SoftwareItemDetailResponse['hosts'] {
		return itemDetailsById.get(item.id)?.hosts ?? [];
	}

	function updateableHostCount(item: SoftwareItemResponse): number | null {
		const hosts = detailHosts(item);
		if (hosts.length > 0) {
			return hosts.filter((host) => host.update_available && host.latest_version).length;
		}
		return null;
	}

	function hasAnyUpdateableHosts(item: SoftwareItemResponse): boolean {
		const updateCount = updateableHostCount(item);
		return updateCount === null ? item.update_available : updateCount > 0;
	}

	function softwareUpdateLabel(item: SoftwareItemResponse): string {
		const updateCount = updateableHostCount(item);
		return updateCount === null
			? 'loading updates'
			: updateCount === 0
				? 'up to date'
				: `${updateCount} update${updateCount === 1 ? '' : 's'}`;
	}

	function versionLabel(
		version: string | null | undefined,
		displayVersion?: string | null | undefined,
		fallback = '—'
	): string {
		if (!version) return fallback;
		return formatVersion(resolveDisplayVersion(version, displayVersion ?? undefined));
	}

	function primaryPluginLabel(item: SoftwareItemResponse, host?: SoftwareItemHostSummary): string {
		const plugin = host?.plugins[0];
		if (plugin?.plugin_config_name) {
			return plugin.plugin_config_name;
		}
		if (plugin?.plugin_type) {
			return pluginTypeNames.get(plugin.plugin_type) ?? plugin.plugin_type;
		}
		const itemPlugin = item.plugins[0];
		return itemPlugin ? (pluginTypeNames.get(itemPlugin) ?? itemPlugin) : 'Unknown';
	}

	function visibleHosts(item: SoftwareItemResponse): SoftwareItemDetailResponse['hosts'] {
		const hosts = detailHosts(item);
		if (collapsedGroupIds.has(item.id)) {
			return [];
		}
		if (expandedOverflowGroupIds.has(item.id) || hosts.length <= 3) {
			return hosts;
		}
		return hosts.slice(0, 3);
	}

	function hiddenHostCount(item: SoftwareItemResponse): number {
		const hosts = detailHosts(item);
		if (collapsedGroupIds.has(item.id) || expandedOverflowGroupIds.has(item.id) || hosts.length <= 3) {
			return 0;
		}
		return hosts.length - 3;
	}

	function hiddenHostsSummary(item: SoftwareItemResponse): string {
		const hosts = detailHosts(item);
		if (collapsedGroupIds.has(item.id) || expandedOverflowGroupIds.has(item.id) || hosts.length <= 3) {
			return '';
		}
		const updateCount = hosts.slice(3).filter((host) => host.update_available && host.latest_version).length;
		if (updateCount === 0) {
			return 'all up to date';
		}
		return `${updateCount} with update${updateCount === 1 ? '' : 's'}`;
	}

	function hostDisplayName(host: SoftwareItemHostSummary): string {
		return host.friendly_name || host.hostname;
	}

	function isSingleHostItem(item: SoftwareItemResponse): boolean {
		const hosts = detailHosts(item);
		if (hosts.length > 0) {
			return hosts.length === 1;
		}
		return item.host_count === 1;
	}

	function singleHost(item: SoftwareItemResponse): SoftwareItemHostSummary | null {
		const hosts = detailHosts(item);
		return hosts.length === 1 ? hosts[0] : null;
	}

	function versionTitle(version: string | null | undefined, displayVersion?: string | null | undefined): string {
		return resolveDisplayVersion(version, displayVersion ?? undefined) ?? '—';
	}

	function groupIsOpen(itemId: string): boolean {
		return !collapsedGroupIds.has(itemId);
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
			const detail = (await loadSoftwareItemDetail(item.id, { force: true })) ?? (await getSoftwareItem(item.id));
			cacheItemDetail(detail);
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
								<select
									class="select text-sm w-auto"
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
						</div>
						{#if canManage}
							<div class="shrink-0">
								<Button variant="primary" size="sm" onclick={() => (showAddModal = true)}>Add Software</Button>
							</div>
						{/if}
					</header>
					{#if error}
						<div class="p-5">
							<Callout tone="danger" title="Unable to load software items" message={error}>
								<Button variant="primary" size="sm" class="mt-3" onclick={() => loadAll(currentPage)}>Retry</Button>
							</Callout>
						</div>
					{:else if loading}
						<p class="px-5 py-8 text-center text-sm text-[var(--text-secondary)]">Loading software items...</p>
					{:else if items.length === 0}
						<div class="px-4 py-8 text-center">
							<EmptyState title={itemsEmptyState.title} description={itemsEmptyState.description} />
						</div>
					{:else}
						<div data-ui="software-group-list" role="list" aria-label="Tracked software">
							{#each items as item (item.id)}
								{@const compactSingleHost = singleHost(item)}
								{@const isCompactSingleHost = isSingleHostItem(item)}
								<div
									class="border-b border-[var(--border-subtle)] last:border-b-0"
									data-testid={'software-group-' + item.id}
									role="listitem"
								>
									<div
										class={`grid items-center gap-x-3 bg-[var(--bg-raised)] px-4 py-2.5 ${
											canManage ? 'grid-cols-[24px_minmax(0,1fr)_40px]' : 'grid-cols-[minmax(0,1fr)]'
										}`}
										data-testid={'software-group-header-' + item.id}
									>
										{#if canManage}
											<div>
												<Checkbox
													id="software-row-{item.id}"
													checked={batchSelectedIds.has(item.id)}
													onchange={() => toggleBatchSelect(item.id)}
													aria-label={'Select ' + item.name}
												/>
											</div>
										{/if}
										<div
											class="grid grid-cols-[16px_1fr_120px_88px] items-center gap-x-3"
											data-ui="software-group-grid"
										>
											<div aria-hidden="true"></div>
											<div class="min-w-0">
												<div class="flex items-center gap-2">
													{#if canManage}
														<button
															class="cursor-pointer text-section-title leading-none transition-[background,border-color,color] duration-fast hover:text-[var(--accent-bright)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]"
															class:text-[var(--color-warning)]={item.featured}
															class:star-unfeatured={!item.featured}
															title={item.featured ? 'Unfeature' : 'Feature'}
															onclick={(e) => {
																e.stopPropagation();
																toggleFeatured(item);
															}}
															aria-label={(item.featured ? 'Unfeature ' : 'Feature ') + item.name}
														>
															{item.featured ? '★' : '☆'}
														</button>
													{:else}
														<span class={item.featured ? 'text-[var(--color-warning)]' : 'star-unfeatured'}
															>{item.featured ? '★' : '☆'}</span
														>
													{/if}
													{#if isValidLogoUrl(item.icon_url)}
														<img
															src={item.icon_url}
															alt=""
															class="h-5 w-5 rounded-panel object-contain"
															referrerpolicy="no-referrer"
														/>
													{/if}
													<a
														href={'/software/' + item.id}
														class="truncate text-sm font-semibold text-[var(--text-primary)] hover:underline"
													>
														{item.name}
													</a>
												</div>
												{#if isCompactSingleHost && compactSingleHost}
													<div class="mt-0.5 flex items-center gap-2">
														<p class="truncate text-nav-item text-[var(--text-secondary)]">
															{hostDisplayName(compactSingleHost)}
														</p>
														<PillBadge label={primaryPluginLabel(item, compactSingleHost)} />
													</div>
												{:else}
													<div class="mt-0.5 flex items-center gap-1">
														<button
															type="button"
															class="expand-pill"
															aria-label={groupIsOpen(item.id) ? 'Collapse ' + item.name : 'Expand ' + item.name}
															aria-expanded={groupIsOpen(item.id)}
															aria-controls={'software-group-body-' + item.id}
															onclick={() => toggleGroupCollapsed(item.id)}
														>
															<span
																class={groupIsOpen(item.id)
																	? 'shrink-0 text-subsection-title leading-none'
																	: 'shrink-0 text-table-header leading-none'}
																aria-hidden="true">{groupIsOpen(item.id) ? '▼' : '▶'}</span
															>
															<span>{item.host_count} host{item.host_count === 1 ? '' : 's'}</span>
														</button>
														<span class="text-nav-item text-[var(--text-secondary)]">· {softwareUpdateLabel(item)}</span
														>
													</div>
												{/if}
											</div>
											{#if isCompactSingleHost && compactSingleHost}
												<div class="text-right">
													<p
														class="font-mono text-nav-item text-[var(--text-secondary)]"
														title={versionTitle(
															compactSingleHost.installed_version,
															compactSingleHost.installed_display_version
														)}
													>
														{versionLabel(
															compactSingleHost.installed_version,
															compactSingleHost.installed_display_version
														)}
													</p>
													{#if compactSingleHost.update_available && compactSingleHost.latest_version}
														<p
															class="font-mono text-button text-[var(--accent-bright)]"
															title={versionTitle(
																compactSingleHost.latest_version,
																(compactSingleHost.latest_release_metadata?.display_version as
																	| string
																	| null
																	| undefined) ?? undefined
															)}
														>
															↓ {versionLabel(
																compactSingleHost.latest_version,
																(compactSingleHost.latest_release_metadata?.display_version as
																	| string
																	| null
																	| undefined) ?? undefined
															)}
														</p>
													{/if}
												</div>
											{:else}
												<div aria-hidden="true"></div>
											{/if}
											<div class="flex justify-end">
												{#if canTriggerUpdates}
													{#if isCompactSingleHost}
														<ActionBadge
															variant="navigation"
															tone="accent"
															idleLabel="Update"
															hoverLabel="Update"
															disabled={!(compactSingleHost?.update_available && compactSingleHost?.latest_version)}
															onclick={() => openUpdateModal(item)}
														/>
													{:else}
														{@const updateCount = updateableHostCount(item)}
														<UpdateAllButton
															state={hasAnyUpdateableHosts(item) ? 'idle' : 'dim'}
															count={updateCount !== null && updateCount > 0 ? updateCount : undefined}
															ariaLabel={hasAnyUpdateableHosts(item) ? undefined : 'No updates available'}
															onclick={() => openUpdateModal(item)}
														/>
													{/if}
												{:else if isCompactSingleHost && compactSingleHost?.update_available}
													<StatusBadge tone="info" label="Update avail" />
												{:else if hasAnyUpdateableHosts(item)}
													{@const groupUpdateCount = updateableHostCount(item)}
													<StatusBadge
														tone="info"
														label={groupUpdateCount === null
															? 'Updates avail'
															: `${groupUpdateCount} update${groupUpdateCount === 1 ? '' : 's'}`}
													/>
												{:else}
													<StatusBadge tone="success" label="Up to date" />
												{/if}
											</div>
										</div>
										{#if canManage}
											<div class="actions-menu flex justify-end">
												<Button
													variant="ghost"
													size="sm"
													ariaLabel={'Actions for ' + item.name}
													onclick={(e) => {
														e.stopPropagation();
														toggleMenu(item.id, e.currentTarget);
													}}>&#8943;</Button
												>
											</div>
										{/if}
									</div>
									{#if !isCompactSingleHost && itemDetailLoadingIds.has(item.id)}
										<div
											class={`grid items-center gap-x-3 border-t border-[var(--border-subtle)] px-4 py-3 ${
												canManage ? 'grid-cols-[24px_minmax(0,1fr)_40px]' : 'grid-cols-[minmax(0,1fr)]'
											}`}
											id={'software-group-body-' + item.id}
										>
											{#if canManage}
												<span aria-hidden="true"></span>
											{/if}
											<div class="grid grid-cols-[minmax(0,1fr)_120px_88px] items-center gap-x-3">
												<div class="col-[1/4] text-sm text-[var(--text-secondary)]">Loading hosts...</div>
											</div>
											{#if canManage}
												<span aria-hidden="true"></span>
											{/if}
										</div>
									{:else if !isCompactSingleHost && detailHosts(item).length > 0}
										<div id={'software-group-body-' + item.id}>
											{#each visibleHosts(item) as host (host.id)}
												<div
													class={`grid items-center gap-x-3 border-t border-[var(--border-subtle)] bg-transparent px-4 py-2.5 transition-[background,border-color,color] duration-fast hover:bg-[var(--bg-raised)] ${
														canManage ? 'grid-cols-[24px_minmax(0,1fr)_40px]' : 'grid-cols-[minmax(0,1fr)]'
													}`}
													data-testid={'software-host-row-' + host.id}
												>
													{#if canManage}
														<span aria-hidden="true"></span>
													{/if}
													<div
														class="grid grid-cols-[16px_1fr_120px_88px] items-center gap-x-3"
														data-ui="software-host-grid"
													>
														<div class="min-w-0 pl-[18px]">
															<div class="flex min-w-0 items-center gap-2">
																<span class="shrink-0 text-table-header text-[var(--text-secondary)]" aria-hidden="true"
																	>·</span
																>
																<p class="truncate text-sm text-[var(--text-primary)]">{hostDisplayName(host)}</p>
																<PillBadge label={primaryPluginLabel(item, host)} />
															</div>
															{#if hostDisplayName(host) !== host.hostname}
																<p class="mt-1 truncate text-nav-item text-[var(--text-secondary)]">{host.hostname}</p>
															{/if}
														</div>
														<div class="text-right">
															<p
																class="font-mono text-nav-item text-[var(--text-secondary)]"
																title={versionTitle(host.installed_version, host.installed_display_version)}
															>
																{versionLabel(host.installed_version, host.installed_display_version)}
															</p>
															{#if host.update_available && host.latest_version}
																<p
																	class="font-mono text-button text-[var(--accent-bright)]"
																	title={versionTitle(
																		host.latest_version,
																		(host.latest_release_metadata?.display_version as string | null | undefined) ??
																			undefined
																	)}
																>
																	↓ {versionLabel(
																		host.latest_version,
																		(host.latest_release_metadata?.display_version as string | null | undefined) ??
																			undefined
																	)}
																</p>
															{/if}
														</div>
														<div class="flex justify-end">
															{#if host.update_available && canTriggerUpdates}
																<ActionBadge
																	variant="navigation"
																	tone="accent"
																	idleLabel="Update Avail"
																	hoverLabel="↑ Update"
																	onclick={() => openUpdateModal(item)}
																/>
															{:else if host.update_available}
																<StatusBadge tone="info" label="Update avail" />
															{:else}
																<StatusBadge tone="success" label="Up to date" />
															{/if}
														</div>
													</div>
													{#if canManage}
														<span aria-hidden="true"></span>
													{/if}
												</div>
											{/each}
											{#if hiddenHostCount(item) > 0}
												<div
													class={`grid items-center gap-x-3 border-t border-[var(--border-subtle)] bg-transparent px-4 py-2 ${
														canManage ? 'grid-cols-[24px_minmax(0,1fr)_40px]' : 'grid-cols-[minmax(0,1fr)]'
													}`}
												>
													{#if canManage}
														<span aria-hidden="true"></span>
													{/if}
													<div class="grid grid-cols-[minmax(0,1fr)_120px_88px] items-center gap-x-3">
														<div>
															<button
																type="button"
																class="pl-[49px] text-nav-item text-[var(--text-secondary)] transition-[background,border-color,color] duration-fast hover:text-[var(--text-primary)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]"
																onclick={() => toggleGroupOverflow(item.id)}
															>
																▸ {hiddenHostCount(item)} more — {hiddenHostsSummary(item)}
															</button>
														</div>
														<span aria-hidden="true"></span>
														<span aria-hidden="true"></span>
													</div>
													{#if canManage}
														<span aria-hidden="true"></span>
													{/if}
												</div>
											{/if}
										</div>
									{/if}
								</div>
							{/each}
							<TableFooterBar total={totalItems} {currentPage} {totalPages} onPageChange={loadAll} />
						</div>
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

{#if editItem}
	<ModalShell title="Edit Software Item" onclose={() => (editItem = null)}>
		<FormFieldRow label="Name" inputId="software-edit-name">
			<Input id="software-edit-name" type="text" bind:value={editForm.name} />
		</FormFieldRow>

		<FormFieldRow
			label="Icon URL"
			hint="Optional, HTTPS"
			inputId="software-edit-icon-url"
			error={editForm.icon_url.trim() && !isValidLogoUrl(editForm.icon_url.trim())
				? 'Icon URL must be a valid HTTPS URL.'
				: undefined}
		>
			<Input
				id="software-edit-icon-url"
				type="text"
				bind:value={editForm.icon_url}
				placeholder="https://example.com/icon.png"
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

<style>
	.expand-pill {
		display: inline-flex;
		height: 14px;
		align-items: center;
		overflow: hidden;
		border-radius: var(--radius-badge);
		border: 1px solid rgba(var(--accent-rgb), 0.22);
		background: rgba(var(--accent-rgb), 0.08);
		padding: 0 5px;
		font-size: var(--text-button);
		font-weight: 600;
		text-transform: none;
		gap: 3px;
		color: var(--accent);
		transition:
			background 0.12s,
			border-color 0.12s,
			color 0.12s;
	}
	.expand-pill:hover {
		background: rgba(var(--accent-rgb), 0.18);
		border-color: rgba(var(--accent-rgb), 0.42);
		color: var(--accent-bright);
	}
	.expand-pill:focus-visible {
		outline: none;
		box-shadow: 0 0 0 3px rgba(var(--accent-rgb), 0.25);
	}

	.star-unfeatured {
		color: var(--text-secondary);
	}
</style>
