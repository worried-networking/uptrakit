<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { page } from '$app/stores';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import {
		getSoftwareItem,
		checkSoftwareItemVersions,
		checkSoftwareItemVersionsHost,
		triggerSoftwareUpdate,
		updateSoftwareItem,
		deleteSoftwareItem,
		unassignHostFromSoftwareItem
	} from '$lib/api';
	import { formatDate, formatVersion } from '$lib/utils';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import { subscribeToEvent } from '$lib/stores/events.svelte';
	import Modal from '$lib/components/Modal.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import TerminalOutput from '$lib/components/TerminalOutput.svelte';
	import EditHostAssignmentModal from '$lib/components/EditHostAssignmentModal.svelte';
	import AssignToHostModal from '$lib/components/AssignToHostModal.svelte';
	import { connectInteractiveSession } from '$lib/interactive';
	import type { InteractiveConnectionState } from '$lib/interactive';
	import { Permission, hasAnyPermission } from '$lib/types';
	import type { AttestationStatus, SoftwareItemDetailResponse, SoftwareItemHostSummary } from '$lib/types';

	const id = $derived($page.params.id as string);

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
	let editForm = $state({ name: '', featured: true });
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
	let updateAllSelectedHostIds: Set<string> = $state(new Set());
	let updateAllTriggering: boolean = $state(false);

	// Assign to Host modal state
	let showAssignModal: boolean = $state(false);

	// Configure plugins modal state
	let configureModal: SoftwareItemHostSummary | null = $state(null);

	// Release notes modal state
	interface ReleaseMeta {
		release_url?: string;
		release_notes?: string;
		tag?: string;
		published_at?: string;
		attestation_status?: AttestationStatus;
	}
	let releaseNotesModal: {
		softwareName: string;
		hostName: string;
		meta: ReleaseMeta;
	} | null = $state(null);

	// Live terminal modal state
	let liveModal: { updateHistoryId: string; hostName: string } | null = $state(null);
	let liveWsState: InteractiveConnectionState = $state('disconnected');
	let liveWsHandle: ReturnType<typeof connectInteractiveSession> | null = null;
	let liveStdinAttention: boolean = $state(false);
	let liveTerminalRef: TerminalOutput | undefined = $state(undefined);

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

	const ROLE_SHORT: Record<string, string> = {
		detect_version: 'Detect',
		fetch_releases: 'Fetch',
		execute_update: 'Update'
	};

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
				})
			);
			refreshInterval = setInterval(() => {
				if (document.visibilityState === 'visible') loadItem(true);
			}, 300_000);
		}
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
		editForm = { name: item.name, featured: item.featured };
		editItem = true;
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
		editSubmitting = true;
		try {
			await updateSoftwareItem(item.id, {
				name: editForm.name || undefined,
				featured: editForm.featured
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
		const resolved = host ?? resolveMenuHost();
		closeMenu();
		if (!resolved) return;
		const toVersion = resolved.latest_version ?? item?.latest_version ?? '';
		updateModal = { host: resolved, toVersion };
	}

	async function executeUpdate() {
		if (!item || !updateModal || updateTriggering) return;
		updateTriggering = true;
		try {
			const hostName = updateModal.host.hostname;
			const res = await triggerSoftwareUpdate(item.id, updateModal.host.host_id, {
				to_version: updateModal.toVersion
			});
			showSuccess(`Update triggered — history ID: ${res.update_history_id}`);
			updateModal = null;
			openLiveModal(res.update_history_id, hostName);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to trigger update');
		} finally {
			updateTriggering = false;
		}
	}

	function openLiveModal(updateHistoryId: string, hostName: string) {
		liveModal = { updateHistoryId, hostName };
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
		liveWsState = 'disconnected';
		liveStdinAttention = false;
	}

	async function openUpdateAllModal() {
		if (!item) return;
		updateAllModal = true;
		updateAllDetail = null;
		updateAllSelectedHostIds = new Set();
		updateAllLoading = true;
		try {
			const detail = await getSoftwareItem(item.id);
			updateAllDetail = detail;
			updateAllSelectedHostIds = new Set(detail.hosts.filter((h) => h.update_available).map((h) => h.host_id));
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load host details.');
			updateAllModal = false;
		} finally {
			updateAllLoading = false;
		}
	}

	async function executeUpdateAll() {
		if (!item || !updateAllDetail || updateAllTriggering) return;
		updateAllTriggering = true;
		const targets = updateAllDetail.hosts.filter(
			(h) => h.update_available && updateAllSelectedHostIds.has(h.host_id) && h.latest_version
		);
		const results = await Promise.allSettled(
			targets.map((h) => triggerSoftwareUpdate(item!.id, h.host_id, { to_version: h.latest_version! }))
		);
		const succeeded = results.filter((r) => r.status === 'fulfilled').length;
		const failed = results.filter((r) => r.status === 'rejected').length;
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
			attestation_status
		};
	}

	function openReleaseNotesModal(host: SoftwareItemHostSummary) {
		const meta = getReleaseMeta(host);
		if (!meta) return;
		releaseNotesModal = { softwareName: item?.name ?? '', hostName: host.hostname, meta };
	}

	function versionStatusLabel(host: SoftwareItemHostSummary): string {
		if (!host.installed_version) return 'Unknown';
		if (!host.latest_version) return 'Unknown latest';
		if (host.update_available) return 'Update Available';
		return 'Up-to-date';
	}

	function versionStatusClass(host: SoftwareItemHostSummary): string {
		if (!host.installed_version) return 'preset-tonal';
		if (host.update_available) return 'preset-filled-warning-500';
		return 'preset-filled-success-500';
	}
</script>

<svelte:window onclick={handleWindowClick} />

{#if getUser()}
	<div class="mb-4">
		<a href="/software" class="text-sm text-surface-500 hover:underline">← Back to Software</a>
	</div>

	{#if !canView}
		<aside class="rounded-lg p-4 preset-filled-error-500">
			<p>You do not have permission to view software items.</p>
		</aside>
	{:else if loading}
		<p>Loading...</p>
	{:else if error}
		<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
			<p>{error}</p>
			<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadItem()}>Retry</button>
		</aside>
	{:else if item}
		<!-- Header -->
		<div class="mb-6 flex flex-wrap items-start justify-between gap-4">
			<div>
				<h1 class="h1">{item.name}</h1>
				<div class="mt-2 flex flex-wrap items-center gap-2">
					{#if canManage}
						<button
							class="cursor-pointer text-xl leading-none transition-opacity hover:opacity-70"
							class:text-warning-500={item.featured}
							class:text-surface-400={!item.featured}
							title={item.featured ? 'Unfeature' : 'Feature'}
							onclick={toggleFeatured}
							aria-label="{item.featured ? 'Unfeature' : 'Feature'} {item.name}"
						>
							{item.featured ? '★' : '☆'}
						</button>
					{:else}
						<span class="text-xl {item.featured ? 'text-warning-500' : 'text-surface-400'}"
							>{item.featured ? '★' : '☆'}</span
						>
					{/if}
					{#if item.update_available}
						<span class="badge preset-filled-warning-500">Update Available</span>
					{/if}
					{#if item.plugins.length > 0}
						<span class="text-sm text-surface-500">{item.plugins.join(', ')}</span>
					{/if}
				</div>
				<div class="mt-2 space-y-1 text-sm text-surface-500">
					{#if item.latest_version}
						<p>
							Latest version: <span
								class="font-medium text-surface-700 dark:text-surface-300"
								title={item.latest_version}>{formatVersion(item.latest_version)}</span
							>
						</p>
					{/if}
					<p>Last checked: {formatDate(item.last_checked_at)}</p>
					<p>{item.host_count} host{item.host_count !== 1 ? 's' : ''} assigned</p>
				</div>
			</div>
			{#if canManage}
				<div class="flex flex-wrap items-center gap-2">
					{#if item.update_available}
						<button class="btn preset-filled-warning-500" onclick={openUpdateAllModal}> Update All </button>
					{/if}
					<button class="btn preset-tonal-surface" onclick={() => (showAssignModal = true)}> Assign to Host </button>
					<button class="btn preset-tonal-surface" onclick={checkAllVersions} disabled={checkingAll}>
						{checkingAll ? 'Checking...' : 'Check All Versions'}
					</button>
					<button class="btn preset-tonal-surface" onclick={openEditModal}>Edit</button>
					<button
						class="btn preset-filled-error-500"
						onclick={() => (confirmDelete = true)}
						disabled={deleteSubmitting}
					>
						Delete
					</button>
				</div>
			{/if}
		</div>

		<!-- Hosts table -->
		<div class="table-wrap">
			<table class="table">
				<thead>
					<tr>
						<th>Hostname</th>
						<th>Installed Version</th>
						<th>Latest Version</th>
						<th>Status</th>
						<th>Detected At</th>
						{#if canManage}
							<th class="w-20"></th>
						{/if}
					</tr>
				</thead>
				<tbody>
					{#if item.hosts.length === 0}
						<tr>
							<td colspan={canManage ? 6 : 5} class="py-8 text-center">
								<p class="text-lg font-medium">No hosts assigned</p>
								<p class="mt-1 text-sm text-surface-500">Assign hosts to this software item to start tracking.</p>
							</td>
						</tr>
					{:else}
						{#each item.hosts as host (host.id)}
							<tr>
								<td>
									<a href="/hosts/{host.host_id}" class="hover:underline font-medium">{host.hostname}</a>
									{#if host.friendly_name && host.friendly_name !== host.hostname}
										<span class="block text-xs text-surface-500">{host.friendly_name}</span>
									{/if}
									{#if host.qualifier}
										<span class="block text-xs text-surface-500 font-mono">{host.qualifier}</span>
									{/if}
									{#if host.plugins.length > 0}
										<div class="mt-1 space-y-0.5">
											{#each host.plugins as p (`${p.plugin_config_id}_${p.role}`)}
												<div class="flex items-baseline gap-1 text-xs text-surface-500">
													<span class="shrink-0 font-semibold">{ROLE_SHORT[p.role] ?? p.role}:</span>
													<span class="truncate">{p.plugin_config_name}</span>
													{#if p.package_identifier}
														<span class="shrink-0 opacity-60">({p.package_identifier})</span>
													{/if}
													{#if p.execution_site && p.execution_site !== 'auto'}
														<span class="badge preset-tonal text-xs shrink-0">{p.execution_site}</span>
													{/if}
												</div>
											{/each}
										</div>
									{:else}
										<span class="mt-1 block text-xs italic text-surface-400">No plugins configured</span>
									{/if}
								</td>
								<td title={host.installed_version ?? undefined}>{formatVersion(host.installed_version)}</td>
								<td>
									<span title={host.latest_version ?? item.latest_version ?? undefined}
										>{formatVersion(host.latest_version ?? item.latest_version)}</span
									>
									{#if getReleaseMeta(host)}
										<button
											class="btn btn-sm preset-tonal ml-1"
											title="View release notes"
											onclick={() => openReleaseNotesModal(host)}>Notes</button
										>
									{/if}
									{#if getReleaseMeta(host)?.attestation_status === 'Verified'}
										<span
											class="badge preset-filled-success-500 ml-1 text-xs"
											title="GitHub Actions attestation verified">Attested</span
										>
									{:else if getReleaseMeta(host)?.attestation_status === 'NotFound'}
										<span class="badge preset-filled-error-500 ml-1 text-xs" title="No GitHub Actions attestation found"
											>Not attested</span
										>
									{/if}
								</td>
								<td>
									<span class="badge {versionStatusClass(host)}">{versionStatusLabel(host)}</span>
								</td>
								<td>{formatDate(host.installed_version_detected_at)}</td>
								{#if canManage}
									<td>
										<div class="actions-menu">
											<button
												class="btn btn-sm preset-tonal"
												aria-label="Actions for {host.hostname}"
												onclick={(e) => {
													e.stopPropagation();
													toggleMenu(host.id, e.currentTarget);
												}}
											>
												&#8943;
											</button>
										</div>
									</td>
								{/if}
							</tr>
						{/each}
					{/if}
				</tbody>
			</table>
		</div>
	{/if}
{/if}

{#if openMenuHostId && item}
	{@const host = item.hosts.find((h) => h.id === openMenuHostId)}
	{#if host}
		<ContextMenu top={menuPos.top} left={menuPos.left} onclose={closeMenu}>
			{#if host.update_available || item.latest_version}
				{@const updateToVer = host.latest_version ?? item.latest_version ?? null}
				<li>
					<button
						class="w-full rounded-md px-3 py-2 text-left text-sm text-warning-600 dark:text-warning-400 hover:bg-surface-200 dark:hover:bg-surface-800"
						role="menuitem"
						tabindex="-1"
						onclick={() => openUpdateModal()}
					>
						Update to {formatVersion(updateToVer)}
					</button>
				</li>
			{/if}
			<li>
				<button
					class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800 disabled:cursor-not-allowed disabled:opacity-50"
					role="menuitem"
					tabindex="-1"
					disabled={checkingHostId === host.host_id}
					onclick={menuCheckHostVersions}
				>
					{checkingHostId === host.host_id ? 'Checking...' : 'Check Versions'}
				</button>
			</li>
			<li>
				<button
					class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800"
					role="menuitem"
					tabindex="-1"
					onclick={openConfigurePlugins}
				>
					Configure Plugins
				</button>
			</li>
			<li>
				<button
					class="w-full rounded-md px-3 py-2 text-left text-sm text-error-500 hover:bg-surface-200 dark:hover:bg-surface-800"
					role="menuitem"
					tabindex="-1"
					onclick={openUnassignConfirm}
				>
					Unassign
				</button>
			</li>
		</ContextMenu>
	{/if}
{/if}

{#if updateModal}
	<Modal title="Confirm Update" onclose={() => (updateModal = null)}>
		<p class="text-sm">
			Update <strong>{item?.name}</strong> on <strong>{updateModal.host.hostname}</strong>?
		</p>
		<div class="grid grid-cols-2 gap-4 text-sm">
			<div>
				<p class="text-surface-500">From</p>
				<p class="font-medium" title={updateModal.host.installed_version ?? undefined}>
					{formatVersion(updateModal.host.installed_version, 'unknown')}
				</p>
			</div>
			<div>
				<p class="text-surface-500">To</p>
				<p class="font-medium" title={updateModal.toVersion}>{formatVersion(updateModal.toVersion)}</p>
			</div>
		</div>

		{#if updateModal}
			{@const meta = getReleaseMeta(updateModal.host)}
			{#if meta?.release_url}
				<p class="text-sm">
					<a href={meta.release_url} target="_blank" rel="noopener noreferrer" class="text-primary-500 hover:underline"
						>View release page ↗</a
					>
				</p>
			{/if}
			{#if meta?.release_notes}
				<details class="text-sm">
					<summary class="cursor-pointer text-surface-500 hover:text-surface-700">Release notes</summary>
					<pre class="mt-2 max-h-48 overflow-y-auto whitespace-pre-wrap text-xs">{meta.release_notes}</pre>
				</details>
			{/if}
			{#if meta?.attestation_status === 'NotFound'}
				<aside class="rounded-lg p-3 preset-filled-warning-500 text-sm" role="alert">
					<strong>Warning:</strong> No GitHub Actions attestation was found for this release. The artifacts may not have been
					produced by the official GitHub Actions workflow.
				</aside>
			{:else if meta?.attestation_status === 'Verified'}
				<p class="text-sm text-success-600 dark:text-success-400">&#10003; GitHub Actions attestation verified</p>
			{/if}
		{/if}

		{#snippet footer()}
			<button class="btn preset-tonal-surface" onclick={() => (updateModal = null)}>Cancel</button>
			<button class="btn preset-filled-warning-500" onclick={executeUpdate} disabled={updateTriggering}>
				{updateTriggering ? 'Triggering...' : 'Trigger Update'}
			</button>
		{/snippet}
	</Modal>
{/if}

{#if releaseNotesModal}
	<Modal onclose={() => (releaseNotesModal = null)} maxWidth="max-w-2xl">
		<div class="flex items-start justify-between gap-4">
			<div>
				<h3 class="h3">{releaseNotesModal.softwareName}</h3>
				<p class="text-sm text-surface-500">
					{releaseNotesModal.meta.tag ?? ''} on {releaseNotesModal.hostName}
					{#if releaseNotesModal.meta.published_at}
						· {formatDate(releaseNotesModal.meta.published_at)}
					{/if}
				</p>
			</div>
			{#if releaseNotesModal.meta.release_url}
				<a
					href={releaseNotesModal.meta.release_url}
					target="_blank"
					rel="noopener noreferrer"
					class="btn btn-sm preset-tonal-surface shrink-0">View on GitHub ↗</a
				>
			{/if}
		</div>

		{#if releaseNotesModal.meta.release_notes}
			<div class="overflow-y-auto max-h-96">
				<pre class="whitespace-pre-wrap text-sm leading-relaxed">{releaseNotesModal.meta.release_notes}</pre>
			</div>
		{:else}
			<p class="text-surface-500 text-sm">No release notes available.</p>
		{/if}

		{#snippet footer()}
			<button class="btn preset-tonal-surface" onclick={() => (releaseNotesModal = null)}>Close</button>
		{/snippet}
	</Modal>
{/if}

{#if liveModal}
	<Modal onclose={closeLiveModal} maxWidth="max-w-3xl">
		<div class="flex items-center justify-between">
			<div class="flex items-center gap-2">
				<h3 class="h3">Update Output</h3>
				{#if liveWsState === 'connected'}
					<span class="badge preset-filled-success-500 text-xs animate-pulse">Live</span>
				{:else if liveWsState === 'connecting'}
					<span class="badge preset-tonal text-xs">Connecting...</span>
				{:else if liveWsState === 'completed'}
					<span class="badge preset-filled-success-500 text-xs">Completed</span>
				{:else if liveWsState === 'error'}
					<span class="badge preset-filled-error-500 text-xs">Error</span>
				{/if}
				{#if liveStdinAttention}
					<span class="badge preset-filled-warning-500 text-xs animate-pulse">Input Required</span>
				{/if}
			</div>
			<div class="flex items-center gap-2">
				{#if liveWsState === 'connected' || liveWsState === 'connecting'}
					<button
						class="btn btn-sm preset-tonal-error text-xs"
						title="Send Ctrl+C (SIGINT)"
						onclick={() => liveWsHandle?.sendSignal(2)}
					>
						Ctrl+C
					</button>
				{/if}
				<p class="text-sm text-surface-500">{liveModal.hostName}</p>
			</div>
		</div>

		<TerminalOutput bind:this={liveTerminalRef} class="h-96" onInput={(data) => liveWsHandle?.sendInput(data)} />

		{#snippet footer()}
			<button class="btn preset-tonal-surface" onclick={closeLiveModal}>
				{liveWsState === 'connected' || liveWsState === 'connecting' ? 'Close (update continues)' : 'Close'}
			</button>
		{/snippet}
	</Modal>
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
	<Modal title="Edit Software Item" onclose={() => (editItem = false)}>
		<label class="label">
			<span>Name</span>
			<input class="input" type="text" bind:value={editForm.name} />
		</label>
		<label class="flex items-center gap-3">
			<input class="checkbox" type="checkbox" bind:checked={editForm.featured} />
			<span>Featured</span>
		</label>
		{#snippet footer()}
			<button class="btn preset-tonal-surface" onclick={() => (editItem = false)}>Cancel</button>
			<button class="btn preset-filled-primary-500" onclick={executeEdit} disabled={editSubmitting}>
				{editSubmitting ? 'Saving...' : 'Save'}
			</button>
		{/snippet}
	</Modal>
{/if}

{#if confirmDelete && item}
	<ConfirmDialog
		title="Delete Software Item"
		messagePrefix="Are you sure you want to delete"
		entityName={item.name}
		confirmLabel={deleteSubmitting ? 'Deleting...' : 'Delete'}
		confirmClass="preset-filled-error-500"
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
		confirmClass="preset-filled-error-500"
		confirmDisabled={unassignSubmitting}
		onconfirm={executeUnassign}
		oncancel={() => (confirmUnassign = null)}
	/>
{/if}

{#if updateAllModal && item}
	<Modal title="Trigger Update — {item.name}" onclose={() => (updateAllModal = false)} maxWidth="max-w-lg">
		{#if updateAllLoading}
			<p class="text-sm text-surface-500">Loading hosts...</p>
		{:else if updateAllDetail}
			<p class="text-sm text-surface-500 mb-2">
				Select the hosts to update. Hosts that are already up to date cannot be selected.
			</p>
			<ul class="space-y-2">
				{#each updateAllDetail.hosts as host (host.host_id)}
					{@const upToDate = !host.update_available}
					<li class="flex items-start gap-3 {upToDate ? 'opacity-50' : ''}">
						<input
							type="checkbox"
							class="checkbox mt-0.5"
							disabled={upToDate}
							checked={updateAllSelectedHostIds.has(host.host_id)}
							onchange={(e) => {
								const next = new Set(updateAllSelectedHostIds);
								if ((e.target as HTMLInputElement).checked) {
									next.add(host.host_id);
								} else {
									next.delete(host.host_id);
								}
								updateAllSelectedHostIds = next;
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
			<button class="btn preset-tonal-surface" onclick={() => (updateAllModal = false)}> Cancel </button>
			<button
				class="btn preset-filled-primary-500"
				disabled={updateAllSelectedHostIds.size === 0 || updateAllTriggering}
				onclick={executeUpdateAll}
			>
				{updateAllTriggering ? 'Triggering...' : `Update ${updateAllSelectedHostIds.size} host(s)`}
			</button>
		{/snippet}
	</Modal>
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
