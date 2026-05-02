<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import {
		getSoftwareItem,
		getSoftwareItems,
		checkSoftwareItemVersions,
		checkSoftwareItemVersionsHost,
		triggerSoftwareUpdate,
		updateSoftwareItem,
		deleteSoftwareItem,
		unassignHostFromSoftwareItem,
		getUpdateHistoryEntry,
		previewSoftwareItemMerge,
		executeSoftwareItemMerge
	} from '$lib/api';
	import { formatDate, formatVersion, isValidExternalUrl, isValidLogoUrl, resolveDisplayVersion } from '$lib/utils';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import { subscribeToEvent } from '$lib/stores/events.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import TerminalOutput from '$lib/components/TerminalOutput.svelte';
	import EditHostAssignmentModal from '$lib/components/EditHostAssignmentModal.svelte';
	import AssignToHostModal from '$lib/components/AssignToHostModal.svelte';
	import { CheckboxList } from '$lib/components/forms';
	import type { CheckboxListItem } from '$lib/components/forms';
	import SurfaceReadPanel from '$lib/components/surfaces/SurfaceReadPanel.svelte';
	import { connectInteractiveSession } from '$lib/interactive';
	import type { InteractiveConnectionState } from '$lib/interactive';
	import { Permission, hasAnyPermission, hasPermissionValue } from '$lib/types';
	import type {
		AttestationStatus,
		MergeSoftwareItemSummary,
		SoftwareItemDetailResponse,
		SoftwareItemHostSummary
	} from '$lib/types';
	import type { SurfaceResponse } from '$lib/surfaces/contract';
	import { getSurfaceReadModel, getSurfacesBySlot, loadSurfaceReadModels } from '$lib/surfaces/registry.svelte';
	import { filterSurfacesByPermission, shouldUseSurfaceRoute } from '$lib/surfaces/read-model';
	import {
		ActionBadge,
		Callout,
		ContextMenuItem,
		ContextMenuShell,
		DataTable,
		ModalShell,
		PageShell,
		ReleaseNotes,
		SectionCard,
		StatusBadge
	} from '$lib/components/ui';
	import { FormFieldRow, Input, Checkbox } from '$lib/components/forms';
	import SoftwareMergeWizard from '$lib/components/SoftwareMergeWizard.svelte';
	import Button from '$lib/components/Button.svelte';

	const id = $derived(page.params.id as string);

	let item: SoftwareItemDetailResponse | null = $state(null);
	let loading: boolean = $state(true);
	let error: string | null = $state(null);
	let checkingAll: boolean = $state(false);
	let checkingHostId: string | null = $state(null);

	// Context menu state (per host row)
	let openMenuHostId: string | null = $state(null);
	let menuPos: { top: number; left: number } = $state({ top: 0, left: 0 });

	// Edit software item state
	let editItem: boolean = $state(false);
	let editForm = $state({ name: '', featured: true, icon_url: '' });
	let editSubmitting: boolean = $state(false);

	// Delete software item state
	let confirmDelete: boolean = $state(false);
	let deleteSubmitting: boolean = $state(false);

	// Unassign host state
	let confirmUnassign: SoftwareItemHostSummary | null = $state(null);
	let unassignSubmitting: boolean = $state(false);

	// Update confirm modal state
	let updateModal: { host: SoftwareItemHostSummary; toVersion: string } | null = $state(null);
	let updateTriggering: boolean = $state(false);

	// Update All (multi-host) modal state
	let updateAllModal: boolean = $state(false);
	let updateAllDetail: SoftwareItemDetailResponse | null = $state(null);
	let updateAllLoading: boolean = $state(false);
	const updateAllSelectedHostIds = new SvelteSet<string>();
	let updateAllTriggering: boolean = $state(false);
	const updateAllHostItems = $derived.by<CheckboxListItem[]>(() => {
		const hosts: SoftwareItemHostSummary[] = updateAllDetail?.hosts ?? [];
		return hosts.map((h) => {
			const upToDate = !h.update_available;
			return {
				value: h.host_id,
				label: h.friendly_name || h.hostname,
				sublabel: upToDate ? 'Already up to date' : `${h.installed_version ?? 'unknown'} → ${h.latest_version}`,
				disabled: upToDate
			};
		});
	});

	// Assign to Host modal state
	let showAssignModal: boolean = $state(false);

	// Configure plugins modal state
	let configureModal: SoftwareItemHostSummary | null = $state(null);
	let mergeModalOpen = $state(false);
	let mergeInitialCandidates: MergeSoftwareItemSummary[] = $state([]);
	let mergeSeedItemId: string | null = $state(null);
	let mergeInitialSearchQuery = $state('');

	// Surface-backed host context operations modal state
	let hostContextSurfaceModal: {
		host: SoftwareItemHostSummary;
		surface: SurfaceResponse;
	} | null = $state(null);

	// Release notes modal state
	interface ReleaseMeta {
		release_url?: string;
		release_notes?: string;
		tag?: string;
		published_at?: string;
		attestation_status?: AttestationStatus;
		display_version?: string;
	}
	let releaseNotesModal: {
		softwareName: string;
		hostName: string;
		meta: ReleaseMeta;
	} | null = $state(null);

	// Live terminal modal state
	let liveModal: { updateHistoryId: string; hostName: string } | null = $state(null);
	let liveStartedAt: number | null = $state(null);
	let liveWsState: InteractiveConnectionState = $state('disconnected');
	let liveWsHandle: ReturnType<typeof connectInteractiveSession> | null = null;
	let liveStdinAttention: boolean = $state(false);
	let liveTerminalRef: TerminalOutput | undefined = $state(undefined);
	let pendingLiveHistoryId: string | null = $state(null);
	let pendingLiveHostName: string = $state('');

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
	const canTriggerUpdates = $derived(getUser()?.permissions.includes(Permission.TriggerUpdates) ?? false);
	const canMergeSoftware = $derived(
		(getUser()?.permissions.includes(Permission.UpdateSoftware) ?? false) &&
			(getUser()?.permissions.includes(Permission.DeleteSoftware) ?? false)
	);
	const softwareItemTabSurfaces = $derived(
		filterSurfacesByPermission(getSurfacesBySlot('software_item.tabs'), (requiredPermission) =>
			hasPermissionValue(getUser(), requiredPermission)
		)
	);
	const softwareItemTabReads = $derived.by(() => {
		const result: Record<string, NonNullable<ReturnType<typeof getSurfaceReadModel>>> = {};
		for (const surface of softwareItemTabSurfaces) {
			const read = getSurfaceReadModel(surface.surface_id);
			if (read) {
				result[surface.surface_id] = read;
			}
		}
		return result;
	});
	const softwareItemTabBaseParams = $derived.by<Record<string, string | undefined>>(() => ({
		software_item_id: id
	}));
	let softwareItemTabsReloadToken = $state(0);
	const hostContextSurfaces = $derived(
		filterSurfacesByPermission(getSurfacesBySlot('software_item.host_context_menu'), (requiredPermission) =>
			hasPermissionValue(getUser(), requiredPermission)
		)
	);
	const hostContextSurfaceReads = $derived.by(() => {
		const result: Record<string, NonNullable<ReturnType<typeof getSurfaceReadModel>>> = {};
		for (const surface of hostContextSurfaces) {
			const read = getSurfaceReadModel(surface.surface_id);
			if (read) {
				result[surface.surface_id] = read;
			}
		}
		return result;
	});
	const useSurfaceHostContext = $derived(shouldUseSurfaceRoute(hostContextSurfaces, hostContextSurfaceReads));
	const hostContextSurface = $derived(useSurfaceHostContext ? hostContextSurfaces[0] : undefined);

	const ROLE_SHORT: Record<string, string> = {
		detect_version: 'Detect',
		fetch_releases: 'Fetch',
		execute_update: 'Update',
		pre_update_hook: 'Pre-Hook',
		post_update_hook: 'Post-Hook'
	};

	function groupHostPlugins(plugins: SoftwareItemHostSummary['plugins']): Array<{ name: string; roles: string[] }> {
		const groups: Record<string, string[]> = {};
		for (const p of plugins) {
			const key = p.plugin_config_name ?? p.plugin_type;
			const role = ROLE_SHORT[p.role] ?? p.role;
			if (groups[key]) {
				if (!groups[key].includes(role)) groups[key].push(role);
			} else {
				groups[key] = [role];
			}
		}
		return Object.entries(groups).map(([name, roles]) => ({ name, roles }));
	}

	function toMergeSummary(softwareItem: { id: string; name: string; host_count: number; plugins: string[] }) {
		return {
			id: softwareItem.id,
			name: softwareItem.name,
			host_count: softwareItem.host_count,
			plugins: softwareItem.plugins
		} satisfies MergeSoftwareItemSummary;
	}

	let refreshInterval: ReturnType<typeof setInterval> | null = null;
	let unsubscribers: (() => void)[] = [];

	onMount(() => {
		if (canView) {
			loadItem();
			unsubscribers.push(
				subscribeToEvent('software_item_updated', (data) => {
					if (data.id === id) loadItem(true);
				}),
				subscribeToEvent('version_check_completed', (data) => {
					if (data.software_item_id === id) loadItem(true);
				}),
				subscribeToEvent('update_completed', (data) => {
					if (data.software_item_id === id) loadItem(true);
				}),
				subscribeToEvent('update_triggered', (data) => {
					if (data.software_item_id === id) loadItem(true);
				}),
				subscribeToEvent('update_protection_started', (data) => {
					if (data.update_history_id === pendingLiveHistoryId && pendingLiveHistoryId) {
						const histId = pendingLiveHistoryId;
						const hostName = pendingLiveHostName;
						pendingLiveHistoryId = null;
						pendingLiveHostName = '';
						openLiveModal(histId, hostName);
					}
					if (data.software_item_id === id) loadItem(true);
				}),
				subscribeToEvent('update_started', (data) => {
					if (data.update_history_id === pendingLiveHistoryId && pendingLiveHistoryId) {
						const histId = pendingLiveHistoryId;
						const hostName = pendingLiveHostName;
						pendingLiveHistoryId = null;
						pendingLiveHostName = '';
						openLiveModal(histId, hostName);
					}
					if (data.software_item_id === id) loadItem(true);
				})
			);
			refreshInterval = setInterval(() => {
				if (document.visibilityState === 'visible') loadItem(true);
			}, 300_000);
		}
	});

	$effect(() => {
		if (softwareItemTabSurfaces.length === 0) {
			return;
		}
		void loadSurfaceReadModels(softwareItemTabSurfaces.map((surface) => surface.surface_id));
	});

	$effect(() => {
		if (hostContextSurfaces.length === 0) {
			return;
		}
		void loadSurfaceReadModels(hostContextSurfaces.map((surface) => surface.surface_id));
	});

	onDestroy(() => {
		for (const unsub of unsubscribers) unsub();
		if (refreshInterval) clearInterval(refreshInterval);
		closeLiveModal();
	});

	async function loadItem(background = false) {
		if (!background) {
			loading = true;
			error = null;
		}
		try {
			item = await getSoftwareItem(id);
			softwareItemTabsReloadToken += 1;
		} catch (e) {
			if (!background) {
				error = e instanceof Error ? e.message : 'Failed to load software item';
			}
		} finally {
			if (!background) loading = false;
		}
	}

	function toggleMenu(hostId: string, button: HTMLElement) {
		if (openMenuHostId === hostId) {
			openMenuHostId = null;
			return;
		}
		const rect = button.getBoundingClientRect();
		menuPos = { top: rect.bottom + 4, left: rect.right - 180 };
		openMenuHostId = hostId;
	}

	function closeMenu() {
		openMenuHostId = null;
	}

	function handleWindowClick(event: MouseEvent) {
		if (openMenuHostId && !(event.target as HTMLElement).closest('.actions-menu')) {
			closeMenu();
		}
	}

	function openEditModal() {
		if (!item) return;
		editForm = { name: item.name, featured: item.featured, icon_url: item.icon_url ?? '' };
		editItem = true;
	}

	function openMergeModal() {
		if (!item || !canMergeSoftware) return;
		mergeInitialCandidates = [toMergeSummary(item)];
		mergeSeedItemId = item.id;
		mergeInitialSearchQuery = item.name;
		mergeModalOpen = true;
	}

	async function searchMergeCandidates(query: string): Promise<MergeSoftwareItemSummary[]> {
		const result = await getSoftwareItems(1, 25, undefined, undefined, undefined, undefined, query);
		return result.items.map(toMergeSummary);
	}

	async function toggleFeatured() {
		if (!item) return;
		try {
			await updateSoftwareItem(item.id, { featured: !item.featured });
			item = { ...item, featured: !item.featured };
			showSuccess(`"${item.name}" ${item.featured ? 'featured' : 'unfeatured'}.`);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to update item');
		}
	}

	async function executeEdit() {
		if (!item || editSubmitting) return;
		if (editForm.icon_url.trim() && !isValidLogoUrl(editForm.icon_url.trim())) return;
		editSubmitting = true;
		try {
			const trimmedIcon = editForm.icon_url.trim();
			const icon_url = trimmedIcon === '' ? (item.icon_url ? null : undefined) : trimmedIcon;
			await updateSoftwareItem(item.id, {
				name: editForm.name || undefined,
				featured: editForm.featured,
				icon_url
			});
			showSuccess('Software item updated.');
			editItem = false;
			await loadItem(true);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to update software item.');
		} finally {
			editSubmitting = false;
		}
	}

	async function executeDelete() {
		if (!item || deleteSubmitting) return;
		const name = item.name;
		deleteSubmitting = true;
		confirmDelete = false;
		try {
			await deleteSoftwareItem(item.id);
			showSuccess(`"${name}" deleted.`);
			goto('/software');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to delete software item.');
			deleteSubmitting = false;
		}
	}

	async function executeUnassign() {
		if (!item || !confirmUnassign || unassignSubmitting) return;
		const host = confirmUnassign;
		confirmUnassign = null;
		unassignSubmitting = true;
		try {
			await unassignHostFromSoftwareItem(item.id, host.host_id);
			showSuccess(`"${host.hostname}" unassigned from "${item.name}".`);
			await loadItem(true);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to unassign host.');
		} finally {
			unassignSubmitting = false;
		}
	}

	async function checkAllVersions() {
		if (!item || checkingAll) return;
		checkingAll = true;
		try {
			const result = await checkSoftwareItemVersions(item.id);
			if (result.agents_notified > 0) {
				showSuccess(`Version check triggered — ${result.agents_notified} agent(s) notified`);
			} else {
				showSuccess('Version check queued — no agents currently connected');
			}
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to trigger version check');
		} finally {
			checkingAll = false;
		}
	}

	function openConfigurePlugins() {
		const host = resolveMenuHost();
		closeMenu();
		if (host) configureModal = host;
	}

	function openUnassignConfirm() {
		const host = resolveMenuHost();
		closeMenu();
		if (host) confirmUnassign = host;
	}

	function openHostContextSurface(host: SoftwareItemHostSummary) {
		closeMenu();
		if (!hostContextSurface) {
			showError('Host context surface is not available.');
			return;
		}
		hostContextSurfaceModal = { host, surface: hostContextSurface };
	}

	function menuCheckHostVersions() {
		const host = resolveMenuHost();
		closeMenu();
		if (host) checkHostVersions(host.host_id);
	}

	async function checkHostVersions(hostId: string) {
		if (!item || checkingHostId) return;
		checkingHostId = hostId;
		try {
			const result = await checkSoftwareItemVersionsHost(item.id, hostId);
			if (result.agents_notified > 0) {
				showSuccess('Version check triggered for host');
			} else {
				showSuccess('Version check queued — no agents currently connected');
			}
			setTimeout(() => loadItem(true), 2000);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to trigger version check');
		} finally {
			checkingHostId = null;
		}
	}

	function resolveMenuHost(): SoftwareItemHostSummary | null {
		return openMenuHostId && item ? (item.hosts.find((h) => h.id === openMenuHostId) ?? null) : null;
	}

	function openUpdateModal(host?: SoftwareItemHostSummary) {
		if (!canTriggerUpdates) return;
		const resolved = host ?? resolveMenuHost();
		closeMenu();
		if (!resolved) return;
		const toVersion = resolved.latest_version ?? item?.latest_version ?? '';
		updateModal = { host: resolved, toVersion };
	}

	async function executeUpdate() {
		if (!item || !updateModal || updateTriggering || !canTriggerUpdates) return;
		updateTriggering = true;
		try {
			const hostName = updateModal.host.hostname;
			const res = await triggerSoftwareUpdate(item.id, updateModal.host.host_id, {
				to_version: updateModal.toVersion
			});
			updateModal = null;

			if (res.status === 'failed') {
				showError(`Update failed before dispatch — history ID: ${res.update_history_id}`);
				await loadItem(true);
				return;
			}

			showSuccess(`Update triggered — history ID: ${res.update_history_id}`);
			pendingLiveHistoryId = res.update_history_id;
			pendingLiveHostName = hostName;
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to trigger update');
		} finally {
			updateTriggering = false;
		}
	}

	function openLiveModal(updateHistoryId: string, hostName: string) {
		liveModal = { updateHistoryId, hostName };
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
					loadItem(true);
				},
				onStdinAttention: () => {
					liveStdinAttention = true;
				},
				onStateChange: (state) => {
					liveWsState = state;
				},
				onError: (err) => {
					showError(`Interactive session error: ${err}`);
					// The WS failed before any output arrived.
					// Fetch stored output from the history entry as a fallback
					// so the terminal still shows what happened.
					const historyId = liveModal?.updateHistoryId;
					if (!historyId) return;
					liveWsState = 'connecting';
					void (async () => {
						try {
							const entry = await getUpdateHistoryEntry(historyId);
							// Guard: modal may have been closed while awaiting.
							if (liveModal?.updateHistoryId !== historyId) return;
							if (entry.output) {
								liveTerminalRef?.write(entry.output);
							}
							if (entry.status === 'completed' || entry.status === 'failed') {
								liveWsState = 'completed';
								loadItem(true);
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
		if (liveWsHandle) {
			liveWsHandle.disconnect();
			liveWsHandle = null;
		}
		liveModal = null;
		liveStartedAt = null;
		liveWsState = 'disconnected';
		liveStdinAttention = false;
	}

	async function openUpdateAllModal() {
		if (!item || !canTriggerUpdates) return;
		updateAllModal = true;
		updateAllDetail = null;
		updateAllSelectedHostIds.clear();
		updateAllLoading = true;
		try {
			const detail = await getSoftwareItem(item.id);
			updateAllDetail = detail;
			updateAllSelectedHostIds.clear();
			for (const h of detail.hosts.filter((h) => h.update_available)) {
				updateAllSelectedHostIds.add(h.host_id);
			}
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load host details.');
			updateAllModal = false;
		} finally {
			updateAllLoading = false;
		}
	}

	async function executeUpdateAll() {
		if (!item || !updateAllDetail || updateAllTriggering || !canTriggerUpdates) return;
		updateAllTriggering = true;
		const targets = updateAllDetail.hosts.filter(
			(h) => h.update_available && updateAllSelectedHostIds.has(h.host_id) && h.latest_version
		);
		const results = await Promise.allSettled(
			targets.map((h) => triggerSoftwareUpdate(item!.id, h.host_id, { to_version: h.latest_version! }))
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
		updateAllTriggering = false;
		updateAllModal = false;
		await loadItem(true);
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
			tag: typeof meta.tag === 'string' ? meta.tag : undefined,
			published_at: typeof meta.published_at === 'string' ? meta.published_at : undefined,
			attestation_status,
			display_version: typeof meta.display_version === 'string' ? meta.display_version : undefined
		};
	}

	function openReleaseNotesModal(host: SoftwareItemHostSummary) {
		const meta = getReleaseMeta(host);
		if (!meta) return;
		releaseNotesModal = { softwareName: item?.name ?? '', hostName: host.hostname, meta };
	}

	function versionStatusLabel(host: SoftwareItemHostSummary): string {
		if (host.active_update_history_id) return 'In Progress';
		if (!host.installed_version) return 'Unknown';
		if (!host.latest_version) return 'Unknown latest';
		if (host.update_available) return 'Update Available';
		return 'Up-to-date';
	}

	function versionStatusTone(host: SoftwareItemHostSummary): 'info' | 'neutral' | 'warning' | 'success' {
		if (host.active_update_history_id) return 'info';
		if (!host.installed_version) return 'neutral';
		if (host.update_available) return 'warning';
		return 'success';
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
</script>

<svelte:window onclick={handleWindowClick} />

{#if getUser()}
	<PageShell title={item?.name ?? 'Software Item'} description="Inspect host assignments, status, and update controls.">
		<div class="mb-2">
			<a href="/software" class="text-sm text-[var(--text-muted)] hover:underline">← Back to Software</a>
		</div>

		{#if !canView}
			<Callout tone="danger" title="Access denied" message="You do not have permission to view software items." />
		{:else if loading}
			<SectionCard title="Software Item">
				<p>Loading...</p>
			</SectionCard>
		{:else if error}
			<Callout tone="danger" title="Unable to load software item" message={error}>
				<Button variant="primary" size="sm" class="mt-2" onclick={() => loadItem()}>Retry</Button>
			</Callout>
		{:else if item}
			<!-- Header -->
			<SectionCard>
				<div class="mb-6 flex flex-wrap items-start justify-between gap-4">
					<div>
						<h2 class="text-section-title font-semibold text-[var(--text-primary)]">
							{#if isValidLogoUrl(item.icon_url)}
								<img
									src={item.icon_url}
									alt=""
									class="h-8 w-8 inline-block mr-2 rounded-card object-contain align-middle"
									referrerpolicy="no-referrer"
								/>
							{/if}{item.name}
						</h2>
						<div class="mt-2 flex flex-wrap items-center gap-2">
							{#if canManage}
								<button
									class="cursor-pointer text-[1.25rem] leading-none transition-[background,border-color,color] duration-fast"
									class:text-[var(--color-warning)]={item.featured}
									class:text-[var(--text-muted)]={!item.featured}
									title={item.featured ? 'Unfeature' : 'Feature'}
									onclick={toggleFeatured}
									aria-label="{item.featured ? 'Unfeature' : 'Feature'} {item.name}"
								>
									{item.featured ? '★' : '☆'}
								</button>
							{:else}
								<span
									class="text-[1.25rem] {item.featured ? 'text-[var(--color-warning)]' : 'text-[var(--text-muted)]'}"
									>{item.featured ? '★' : '☆'}</span
								>
							{/if}
							{#if item.plugins.length > 0}
								<span class="text-sm text-[var(--text-muted)]">{item.plugins.join(', ')}</span>
							{/if}
						</div>
						<div class="mt-2 space-y-1 text-sm text-[var(--text-muted)]">
							{#if item.latest_version}
								<p>
									Latest version: <span class="font-medium text-[var(--text-primary)]" title={item.latest_version}
										>{formatVersion(item.latest_version)}</span
									>
								</p>
							{/if}
							<p>Last checked: {formatDate(item.last_checked_at)}</p>
							<p>{item.host_count} host{item.host_count !== 1 ? 's' : ''} assigned</p>
						</div>
					</div>
					{#if canManage}
						<div class="flex flex-wrap items-center gap-2">
							{#if canTriggerUpdates && item.update_available}
								<Button variant="primary" onclick={openUpdateAllModal}>Update All</Button>
							{/if}
							<Button variant="secondary" onclick={() => (showAssignModal = true)}>Assign to Host</Button>
							<Button variant="secondary" loading={checkingAll} onclick={checkAllVersions}>Check All Versions</Button>
							{#if canMergeSoftware}
								<Button variant="secondary" onclick={openMergeModal}>Merge...</Button>
							{/if}
							<Button variant="secondary" onclick={openEditModal}>Edit</Button>
							<Button variant="danger" loading={deleteSubmitting} onclick={() => (confirmDelete = true)}>Delete</Button>
						</div>
					{/if}
				</div>

				{#if softwareItemTabSurfaces.length > 0}
					<div class="mb-6 space-y-4">
						{#each softwareItemTabSurfaces as surface (surface.surface_id)}
							<SectionCard title={surface.label}>
								<SurfaceReadPanel
									{surface}
									read={softwareItemTabReads[surface.surface_id]}
									baseParams={softwareItemTabBaseParams}
									reloadToken={softwareItemTabsReloadToken}
								/>
							</SectionCard>
						{/each}
					</div>
				{/if}

				<!-- Hosts table -->
				<DataTable
					columns={[]}
					rows={item.hosts as unknown as Record<string, unknown>[]}
					emptyTitle="No hosts assigned"
					emptyDescription="Assign hosts to this software item to start tracking."
					rowKey={(row) => (row as unknown as SoftwareItemHostSummary).id}
				>
					{#snippet header()}
						<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
							<th
								class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								scope="col">Hostname</th
							>
							<th
								class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								scope="col"
							>
								Installed Version
							</th>
							<th
								class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								scope="col"
							>
								Latest Version
							</th>
							<th
								class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								scope="col">Status</th
							>
							<th
								class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								scope="col"
							>
								Detected At
							</th>
							{#if canManage}
								<th
									class="w-20 table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
									scope="col"
								></th>
							{/if}
						</tr>
					{/snippet}
					{#snippet row(rowValue, _index)}
						{@const host = rowValue as unknown as SoftwareItemHostSummary}
						<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
							<td class="table-cell-pad text-[var(--text-primary)]">
								<a href="/hosts/{host.host_id}" class="hover:underline font-medium">{host.hostname}</a
								>{#if host.qualifier}<StatusBadge tone="info" label={host.qualifier} />{/if}
								{#if host.friendly_name && host.friendly_name !== host.hostname}
									<span class="block text-xs text-[var(--text-muted)]">{host.friendly_name}</span>
								{/if}
								{#if host.plugins.length > 0}
									<div class="mt-1 space-y-0.5">
										{#each groupHostPlugins(host.plugins) as group (group.name)}
											<div class="text-xs text-[var(--text-muted)]">
												<span class="font-medium">{group.name}</span><span class="opacity-60">
													· {group.roles.join(' · ')}</span
												>
											</div>
										{/each}
									</div>
								{:else}
									<span class="mt-1 block text-xs italic text-[var(--text-muted)]">No plugins configured</span>
								{/if}
							</td>
							<td
								class="table-cell-pad whitespace-nowrap text-[var(--text-primary)]"
								title={host.installed_version ?? undefined}
								>{formatVersion(resolveDisplayVersion(host.installed_version, host.installed_display_version))}</td
							>
							<td class="table-cell-pad whitespace-nowrap text-[var(--text-primary)]">
								<span title={host.latest_version ?? item?.latest_version ?? undefined}
									>{formatVersion(
										resolveDisplayVersion(
											host.latest_version ?? item?.latest_version,
											getReleaseMeta(host)?.display_version
										)
									)}</span
								>
								{#if getReleaseMeta(host)}
									<button
										class="mt-0.5 block text-xs text-[var(--accent)] hover:underline"
										onclick={() => openReleaseNotesModal(host)}>Release notes ↗</button
									>
								{/if}
								{#if getReleaseMeta(host)?.attestation_status === 'Verified'}
									<span class="mt-0.5 block" title="GitHub Actions attestation verified">
										<StatusBadge tone="success" label="Attested" />
									</span>
								{:else if getReleaseMeta(host)?.attestation_status === 'NotFound'}
									<span class="mt-0.5 block" title="No GitHub Actions attestation found">
										<StatusBadge tone="danger" label="Not Attested" />
									</span>
								{/if}
							</td>
							<td class="table-cell-pad text-[var(--text-primary)]">
								{#if canView && host.active_update_history_id}
									<span class="inline-flex" title="View update progress">
										<ActionBadge
											variant="navigation"
											tone="info"
											idleLabel="In Progress"
											hoverLabel="→ Log"
											onclick={() => openLiveModal(host.active_update_history_id!, host.hostname)}
										/>
									</span>
								{:else if canTriggerUpdates && host.update_available}
									<span
										class="inline-flex"
										title={`Update to ${formatVersion(resolveDisplayVersion(host.latest_version ?? item?.latest_version, getReleaseMeta(host)?.display_version))}`}
									>
										<ActionBadge
											variant="navigation"
											tone="accent"
											idleLabel="Update"
											hoverLabel="Update"
											onclick={() => openUpdateModal(host)}
										/>
									</span>
								{:else}
									<StatusBadge tone={versionStatusTone(host)} label={versionStatusLabel(host)} />
								{/if}
							</td>
							<td class="table-cell-pad whitespace-nowrap text-sm text-[var(--text-muted)]"
								>{formatDate(host.installed_version_detected_at)}</td
							>
							{#if canManage}
								<td class="table-cell-pad">
									<div class="actions-menu">
										<Button
											variant="ghost"
											size="sm"
											ariaLabel="Actions for {host.hostname}"
											onclick={(e) => {
												e.stopPropagation();
												toggleMenu(host.id, e.currentTarget);
											}}>&#8943;</Button
										>
									</div>
								</td>
							{/if}
						</tr>
					{/snippet}
				</DataTable>
			</SectionCard>
		{/if}
	</PageShell>
{/if}

{#if openMenuHostId && item}
	{@const host = item.hosts.find((h) => h.id === openMenuHostId)}
	{#if host}
		<ContextMenuShell top={menuPos.top} left={menuPos.left} onclose={closeMenu}>
			{#if canTriggerUpdates && (host.update_available || item.latest_version)}
				{@const updateToVer = host.latest_version ?? item.latest_version ?? null}
				<li>
					<ContextMenuItem
						label={`Update to ${formatVersion(resolveDisplayVersion(updateToVer, getReleaseMeta(host)?.display_version))}`}
						onclick={() => openUpdateModal()}
					/>
				</li>
			{/if}
			<li>
				<ContextMenuItem
					label={checkingHostId === host.host_id ? 'Checking...' : 'Check Versions'}
					disabled={checkingHostId === host.host_id}
					onclick={menuCheckHostVersions}
				/>
			</li>
			<li>
				<ContextMenuItem label="Configure Plugins" onclick={openConfigurePlugins} />
			</li>
			<li>
				<ContextMenuItem label="Unassign" destructive onclick={openUnassignConfirm} />
			</li>
			{#if useSurfaceHostContext && hostContextSurface}
				<li>
					<ContextMenuItem label={hostContextSurface.label} onclick={() => openHostContextSurface(host)} />
				</li>
			{/if}
		</ContextMenuShell>
	{/if}
{/if}

{#if updateModal}
	<ModalShell title="Confirm Update" onclose={() => (updateModal = null)} maxWidth="max-w-3xl">
		<p class="text-sm">
			Update <strong>{item?.name}</strong> on <strong>{updateModal.host.hostname}</strong>?
		</p>
		<div class="grid grid-cols-2 gap-4 text-sm">
			<div>
				<p class="text-[var(--text-muted)]">From</p>
				<p class="font-medium" title={updateModal.host.installed_version ?? undefined}>
					{formatVersion(
						resolveDisplayVersion(updateModal.host.installed_version, updateModal.host.installed_display_version),
						'unknown'
					)}
				</p>
			</div>
			<div>
				<p class="text-[var(--text-muted)]">To</p>
				<p class="font-medium" title={updateModal.toVersion}>
					{formatVersion(
						resolveDisplayVersion(updateModal.toVersion, getReleaseMeta(updateModal.host)?.display_version)
					)}
				</p>
			</div>
		</div>

		{#if updateModal}
			{@const meta = getReleaseMeta(updateModal.host)}
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
					<summary class="cursor-pointer text-[var(--text-muted)] hover:text-[var(--text-primary)]"
						>Release notes</summary
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
		{/if}

		{#snippet footer()}
			<Button variant="secondary" onclick={() => (updateModal = null)}>Cancel</Button>
			<Button variant="primary" loading={updateTriggering} onclick={executeUpdate}>Trigger Update</Button>
		{/snippet}
	</ModalShell>
{/if}

{#if releaseNotesModal}
	<ModalShell onclose={() => (releaseNotesModal = null)} maxWidth="max-w-3xl">
		<div class="flex items-start justify-between gap-4">
			<div>
				<h3 class="text-subsection-title font-bold text-[var(--text-primary)]">{releaseNotesModal.softwareName}</h3>
				<p class="text-sm text-[var(--text-muted)]">
					{releaseNotesModal.meta.tag ?? ''} on {releaseNotesModal.hostName}
					{#if releaseNotesModal.meta.published_at}
						· {formatDate(releaseNotesModal.meta.published_at)}
					{/if}
				</p>
			</div>
			{#if releaseNotesModal.meta.release_url && isValidExternalUrl(releaseNotesModal.meta.release_url)}
				<Button
					variant="ghost"
					size="sm"
					href={releaseNotesModal.meta.release_url}
					target="_blank"
					rel="noopener noreferrer"
					class="shrink-0">View on GitHub ↗</Button
				>
			{/if}
		</div>

		{#if releaseNotesModal.meta.release_notes}
			<div class="overflow-y-auto max-h-96">
				<ReleaseNotes content={releaseNotesModal.meta.release_notes} />
			</div>
		{:else}
			<p class="text-[var(--text-muted)] text-sm">No release notes available.</p>
		{/if}

		{#snippet footer()}
			<Button variant="secondary" onclick={() => (releaseNotesModal = null)}>Close</Button>
		{/snippet}
	</ModalShell>
{/if}

{#if liveModal}
	<TerminalOutput
		bind:this={liveTerminalRef}
		open={true}
		title={`${item?.name ?? 'Software item'} on ${liveModal.hostName}`}
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

{#if configureModal && item}
	<EditHostAssignmentModal
		softwareItemId={item.id}
		hostId={configureModal.host_id}
		hostName={configureModal.hostname}
		softwareItemName={item.name}
		existingPlugins={configureModal.plugins}
		onclose={() => (configureModal = null)}
		onsuccess={(result) => {
			item = result;
			configureModal = null;
		}}
	/>
{/if}

{#if editItem && item}
	<ModalShell title="Edit Software Item" onclose={() => (editItem = false)}>
		<FormFieldRow label="Name" inputId="software-detail-edit-name">
			<Input id="software-detail-edit-name" type="text" bind:value={editForm.name} />
		</FormFieldRow>
		<FormFieldRow
			label="Icon URL"
			inputId="software-detail-edit-icon-url"
			hint="Optional. Must be HTTPS when provided."
			error={editForm.icon_url.trim() && !isValidLogoUrl(editForm.icon_url.trim())
				? 'Icon URL must be a valid HTTPS URL.'
				: undefined}
		>
			<Input
				id="software-detail-edit-icon-url"
				type="url"
				bind:value={editForm.icon_url}
				placeholder="https://example.com/icon.png"
				error={editForm.icon_url.trim() && !isValidLogoUrl(editForm.icon_url.trim())
					? 'Icon URL must be a valid HTTPS URL.'
					: undefined}
			/>
		</FormFieldRow>
		<FormFieldRow label="Featured" inputId="software-detail-edit-featured">
			<Checkbox id="software-detail-edit-featured" bind:checked={editForm.featured} />
		</FormFieldRow>
		{#snippet footer()}
			<Button variant="secondary" onclick={() => (editItem = false)}>Cancel</Button>
			<Button variant="primary" loading={editSubmitting} onclick={executeEdit}>Save</Button>
		{/snippet}
	</ModalShell>
{/if}

{#if confirmDelete && item}
	<ConfirmDialog
		title="Delete Software Item"
		messagePrefix="Are you sure you want to delete"
		entityName={item.name}
		confirmLabel={deleteSubmitting ? 'Deleting...' : 'Delete'}
		confirmDisabled={deleteSubmitting}
		onconfirm={executeDelete}
		oncancel={() => (confirmDelete = false)}
	/>
{/if}

{#if confirmUnassign}
	<ConfirmDialog
		title="Unassign Host"
		messagePrefix="Remove assignment of"
		entityName="{confirmUnassign.hostname} from this software item"
		confirmLabel={unassignSubmitting ? 'Removing...' : 'Unassign'}
		confirmDisabled={unassignSubmitting}
		onconfirm={executeUnassign}
		oncancel={() => (confirmUnassign = null)}
	/>
{/if}

{#if updateAllModal && item}
	<ModalShell title="Trigger Update — {item.name}" onclose={() => (updateAllModal = false)} maxWidth="max-w-lg">
		{#if updateAllLoading}
			<p class="text-sm text-[var(--text-muted)]">Loading hosts...</p>
		{:else if updateAllDetail}
			<p class="text-sm text-[var(--text-muted)] mb-2">
				Select the hosts to update. Hosts that are already up to date cannot be selected.
			</p>
			<CheckboxList items={updateAllHostItems} selected={updateAllSelectedHostIds} />
		{/if}
		{#snippet footer()}
			<Button variant="secondary" onclick={() => (updateAllModal = false)}>Cancel</Button>
			<Button
				variant="primary"
				loading={updateAllTriggering}
				disabled={updateAllSelectedHostIds.size === 0}
				onclick={executeUpdateAll}>Update {updateAllSelectedHostIds.size} host(s)</Button
			>
		{/snippet}
	</ModalShell>
{/if}

{#if showAssignModal && item}
	<AssignToHostModal
		softwareItemId={item.id}
		softwareItemName={item.name}
		onclose={() => (showAssignModal = false)}
		onsuccess={() => {
			showAssignModal = false;
			loadItem(true);
		}}
	/>
{/if}

{#if mergeModalOpen && item}
	<SoftwareMergeWizard
		candidates={mergeInitialCandidates}
		seedItemId={mergeSeedItemId}
		searchCandidates={searchMergeCandidates}
		initialSearchQuery={mergeInitialSearchQuery}
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
			await goto('/software?merge_success=1');
		}}
	/>
{/if}

{#if hostContextSurfaceModal && item}
	<ModalShell
		title="{hostContextSurfaceModal.surface.label} — {hostContextSurfaceModal.host.hostname}"
		onclose={() => (hostContextSurfaceModal = null)}
	>
		<SurfaceReadPanel
			surface={hostContextSurfaceModal.surface}
			read={hostContextSurfaceReads[hostContextSurfaceModal.surface.surface_id]}
			baseParams={{ software_item_id: item.id, host_id: hostContextSurfaceModal.host.host_id }}
		/>
	</ModalShell>
{/if}
