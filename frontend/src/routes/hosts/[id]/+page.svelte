<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import {
		getHost,
		listUpdateHistory,
		updateHost,
		deactivateHost,
		triggerHostDiscovery,
		listHostDiscoveryAllowlist,
		addHostDiscoveryAllowlistEntry,
		deleteHostDiscoveryAllowlistEntry,
		listPluginTypes,
		getHostTags,
		setHostTags,
		getSoftwareItems
	} from '$lib/api';
	import { formatDate, formatVersion, resolveDisplayVersion } from '$lib/utils';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import { subscribeToEvent } from '$lib/stores/events.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import Pagination from '$lib/components/Pagination.svelte';
	import { Permission, PluginCapability, hasAnyPermission, hasPermissionValue } from '$lib/types';
	import SurfaceReadPanel from '$lib/components/surfaces/SurfaceReadPanel.svelte';
	import { getSurfaceReadModel, getSurfacesBySlot, loadSurfaceReadModels } from '$lib/surfaces/registry.svelte';
	import type {
		HostResponse,
		UpdateHistoryResponse,
		ServiceStatus,
		UpdateHistoryStatus,
		HostDiscoveryAllowlistEntry,
		PluginTypeInfo,
		HostTagResponse,
		SoftwareItemResponse
	} from '$lib/types';
	import Button from '$lib/components/Button.svelte';
	import TagBadge from '$lib/components/TagBadge.svelte';
	import CheckboxList from '$lib/components/CheckboxList.svelte';
	import type { CheckboxListItem } from '$lib/components/CheckboxList.svelte';
	import { Callout, ModalShell, PageShell, SectionCard, StatusBadge, type StatusBadgeTone } from '$lib/components/ui';

	const id = $derived(page.params.id as string);

	let host: HostResponse | null = $state(null);
	let recentHistory: UpdateHistoryResponse[] = $state([]);
	let loading: boolean = $state(true);
	let error: string | null = $state(null);

	let editHost: { friendlyName: string } | null = $state(null);
	let submitting: boolean = $state(false);
	let confirmDeactivate: boolean = $state(false);
	let discovering: boolean = $state(false);
	let isRetrying: boolean = $state(false);

	// Plugin types (loaded lazily when canViewSoftware)
	let pluginTypes: PluginTypeInfo[] = $state([]);
	const discoveryPluginTypes = $derived(
		pluginTypes.filter((t) => t.capabilities.includes(PluginCapability.DiscoverLocalSoftware))
	);

	// Discovery allowlist state
	let hostAllowlist: HostDiscoveryAllowlistEntry[] = $state([]);
	let hostAllowlistLoading: boolean = $state(false);
	let showAllowlistModal: boolean = $state(false);
	let allowlistForm = $state({ plugin_type: '' });
	let allowlistDeleteConfirm: { id: string; plugin_type: string } | null = $state(null);

	// Tags state
	let allTags: HostTagResponse[] = $state([]);
	let showSetTagsModal: boolean = $state(false);
	const selectedTagIds = new SvelteSet<string>();
	const tagItems = $derived<CheckboxListItem[]>(
		allTags.map((t) => ({ value: t.id, label: t.name, sublabel: t.description ?? undefined, color: t.color }))
	);

	// Assigned software state
	let assignedSoftware: SoftwareItemResponse[] = $state([]);
	let assignedSoftwareTotal = $state(0);
	let assignedSoftwarePage = $state(1);
	let assignedSoftwareLoading = $state(false);
	let assignedSoftwareError: string | null = $state(null);
	const assignedSoftwareTotalPages = $derived(Math.ceil(assignedSoftwareTotal / 20));

	const canManage = $derived(hasAnyPermission(getUser(), Permission.UpdateHosts, Permission.DeactivateHosts));
	const canManageSoftware = $derived(
		hasAnyPermission(
			getUser(),
			Permission.CreateSoftware,
			Permission.UpdateSoftware,
			Permission.DeleteSoftware,
			Permission.TriggerChecks,
			Permission.TriggerUpdates
		)
	);
	const canViewSoftware = $derived(getUser()?.permissions.includes(Permission.ViewSoftware) ?? false);
	const hostDetailSlotSurfaces = $derived(getSurfacesBySlot('host_detail.tabs'));
	const hostDetailSlotRenderableSurfaces = $derived(
		hostDetailSlotSurfaces.filter((surface) => hasPermissionValue(getUser(), surface.required_permission))
	);
	const hostDetailSlotReads = $derived.by(() => {
		const result: Record<string, NonNullable<ReturnType<typeof getSurfaceReadModel>>> = {};
		for (const surface of hostDetailSlotRenderableSurfaces) {
			const read = getSurfaceReadModel(surface.surface_id);
			if (read) {
				result[surface.surface_id] = read;
			}
		}
		return result;
	});
	const hostDetailBaseParams = $derived.by<Record<string, string | undefined>>(() =>
		host ? { host_id: host.id } : {}
	);
	let hostDetailReloadToken = $state(0);

	let refreshInterval: ReturnType<typeof setInterval> | null = null;
	let unsubscribers: (() => void)[] = [];

	onMount(() => {
		if (!getUser()) {
			return;
		}
		loadData();
		loadAllTags();
		if (canViewSoftware) {
			loadPluginTypes();
			loadHostAllowlist();
			loadAssignedSoftware();
		}
		unsubscribers.push(
			subscribeToEvent('host_updated', (data) => {
				if (data.id === id) loadData(true);
			}),
			subscribeToEvent('discovery_completed', (data) => {
				if (data.host_id === id) loadData(true);
			}),
			subscribeToEvent('update_completed', (data) => {
				if (data.host_id === id) loadData(true);
			})
		);
		refreshInterval = setInterval(() => {
			if (document.visibilityState === 'visible') loadData(true);
		}, 300_000);
	});

	onDestroy(() => {
		for (const unsub of unsubscribers) unsub();
		if (refreshInterval) clearInterval(refreshInterval);
	});

	$effect(() => {
		if (hostDetailSlotRenderableSurfaces.length === 0) {
			return;
		}
		void loadSurfaceReadModels(hostDetailSlotRenderableSurfaces.map((surface) => surface.surface_id));
	});

	async function loadData(background = false) {
		if (!background) {
			loading = true;
			error = null;
		}
		try {
			const [hostResult, historyResult] = await Promise.all([
				getHost(id),
				listUpdateHistory({ host_id: id, per_page: 5 })
			]);
			host = hostResult;
			recentHistory = historyResult.items;
			hostDetailReloadToken += 1;
		} catch (e) {
			if (!background) {
				error = e instanceof Error ? e.message : 'Failed to load host';
			}
		} finally {
			if (!background) loading = false;
		}
	}

	async function retryLoad() {
		isRetrying = true;
		try {
			await loadData();
		} finally {
			isRetrying = false;
		}
	}

	async function loadPluginTypes() {
		try {
			pluginTypes = await listPluginTypes();
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load plugin types');
		}
	}

	async function loadHostAllowlist() {
		hostAllowlistLoading = true;
		try {
			hostAllowlist = await listHostDiscoveryAllowlist(id);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load host discovery allowlist');
		} finally {
			hostAllowlistLoading = false;
		}
	}

	async function loadAssignedSoftware(page = 1) {
		assignedSoftwareLoading = true;
		assignedSoftwareError = null;
		try {
			const result = await getSoftwareItems(page, 20, undefined, id);
			assignedSoftware = result.items;
			assignedSoftwareTotal = result.total;
			assignedSoftwarePage = page;
		} catch (e) {
			assignedSoftwareError = e instanceof Error ? e.message : 'Failed to load assigned software';
		} finally {
			assignedSoftwareLoading = false;
		}
	}

	async function loadAllTags() {
		try {
			const result = await getHostTags(1, 100);
			allTags = result.items;
		} catch {
			// Tags are non-critical — silently ignore
		}
	}

	function openSetTagsModal() {
		if (!host) return;
		selectedTagIds.clear();
		for (const t of host.tags) selectedTagIds.add(t.id);
		showSetTagsModal = true;
	}

	async function executeSetTags() {
		if (!host || submitting) return;
		submitting = true;
		try {
			const updatedTags = await setHostTags(host.id, { tag_ids: [...selectedTagIds] });
			host = { ...host, tags: updatedTags };
			showSetTagsModal = false;
			showSuccess('Tags updated');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to set tags');
		} finally {
			submitting = false;
		}
	}

	function openAddAllowlistEntry() {
		allowlistForm = { plugin_type: discoveryPluginTypes[0]?.plugin_type ?? '' };
		showAllowlistModal = true;
	}

	function closeAllowlistModal() {
		showAllowlistModal = false;
	}

	async function saveAllowlistEntry() {
		const pluginType = allowlistForm.plugin_type.trim();
		if (!pluginType) {
			showError('Select a plugin type before adding an allowlist entry.');
			return;
		}
		try {
			const created = await addHostDiscoveryAllowlistEntry(id, { plugin_type: pluginType });
			if (!hostAllowlist.some((e) => e.id === created.id)) {
				hostAllowlist = [...hostAllowlist, created];
			}
			showSuccess('Allowlist entry added.');
			closeAllowlistModal();
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to add allowlist entry');
		}
	}

	async function executeDeleteAllowlistEntry() {
		if (!allowlistDeleteConfirm) return;
		const { id: entryId } = allowlistDeleteConfirm;
		allowlistDeleteConfirm = null;
		try {
			await deleteHostDiscoveryAllowlistEntry(id, entryId);
			hostAllowlist = hostAllowlist.filter((e) => e.id !== entryId);
			showSuccess('Allowlist entry removed.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to remove allowlist entry');
		}
	}

	function openEditDialog() {
		if (!host) return;
		editHost = { friendlyName: host.friendly_name };
	}

	function cancelEdit() {
		editHost = null;
	}

	async function executeEdit() {
		if (!editHost || !host || submitting) return;
		submitting = true;
		try {
			const updated = await updateHost(host.id, { friendly_name: editHost.friendlyName });
			host = updated;
			editHost = null;
			showSuccess('Host name updated');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to update host');
		} finally {
			submitting = false;
		}
	}

	async function executeDeactivate() {
		if (!host || submitting) return;
		confirmDeactivate = false;
		submitting = true;
		try {
			await deactivateHost(host.id);
			showSuccess(`Host "${host.friendly_name}" deactivated`);
			goto('/hosts');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to deactivate host');
			submitting = false;
		}
	}

	async function triggerDiscovery() {
		if (!host || discovering) return;
		discovering = true;
		try {
			const result = await triggerHostDiscovery(host.id);
			if (result.plugins_queued > 0) {
				showSuccess(`Discovery triggered — ${result.plugins_queued} plugin(s) queued`);
			} else {
				showSuccess('Discovery triggered — no plugins queued');
			}
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to trigger discovery');
		} finally {
			discovering = false;
		}
	}

	function agentStatusTone(status: ServiceStatus): StatusBadgeTone {
		switch (status) {
			case 'approved':
				return 'success';
			case 'pending':
				return 'neutral';
			case 'rejected':
				return 'danger';
			case 'deactivated':
				return 'neutral';
		}
	}

	function historyStatusTone(status: UpdateHistoryStatus): StatusBadgeTone {
		switch (status) {
			case 'completed':
				return 'success';
			case 'failed':
				return 'danger';
			case 'in_progress':
				return 'warning';
			case 'queued':
			case 'pending':
				return 'neutral';
		}
	}

	function historyStatusLabel(status: UpdateHistoryStatus): string {
		switch (status) {
			case 'completed':
				return 'Done';
			case 'failed':
				return 'Failed';
			case 'in_progress':
				return 'Running';
			case 'queued':
				return 'Queued';
			case 'pending':
				return 'Pending';
		}
	}
</script>

{#if getUser()}
	<PageShell
		title={host?.friendly_name ?? 'Host'}
		description={host ? `Last seen: ${formatDate(host.last_seen_at)}` : 'Inspect host details and update activity.'}
	>
		<div class="mb-2">
			<a href="/hosts" class="text-sm text-[var(--text-muted)] hover:underline">← Back to Hosts</a>
		</div>

		{#if loading}
			<SectionCard title="Host Details">
				<p>Loading...</p>
			</SectionCard>
		{:else if error}
			<Callout tone="danger" title="Unable to load host" message={error}>
				<Button variant="primary" class="mt-2" loading={isRetrying} onclick={retryLoad}>Retry</Button>
			</Callout>
		{:else if host}
			<!-- Header -->
			<div class="mb-6 flex flex-wrap items-start justify-between gap-4">
				<div>
					{#if host.friendly_name !== host.hostname}
						<p class="mt-1 text-sm text-[var(--text-muted)]">{host.hostname}</p>
					{/if}
				</div>
				<div class="flex flex-wrap items-center gap-2">
					{#if canManage}
						<Button variant="secondary" onclick={openEditDialog}>Edit Name</Button>
						<Button variant="danger" onclick={() => (confirmDeactivate = true)}>Deactivate</Button>
					{/if}
					{#if canManageSoftware}
						<Button variant="secondary" loading={discovering} onclick={triggerDiscovery}>Trigger Discovery</Button>
					{/if}
				</div>
			</div>

			<!-- Info grid -->
			<SectionCard title="Host Metadata">
				<div class="grid grid-cols-2 gap-x-8 gap-y-3 sm:grid-cols-4 text-sm">
					<div>
						<p class="text-[var(--text-muted)]">OS</p>
						<p class="font-medium">{host.os_version ?? host.os_type ?? '—'}</p>
					</div>
					<div>
						<p class="text-[var(--text-muted)]">Architecture</p>
						<p class="font-medium">{host.architecture ?? '—'}</p>
					</div>
					<div>
						<p class="text-[var(--text-muted)]">IP Address</p>
						<p class="font-medium">{host.ip_address ?? '—'}</p>
					</div>
					<div>
						<p class="text-[var(--text-muted)]">Machine ID</p>
						<p class="font-medium break-all">{host.machine_id}</p>
					</div>
					<div>
						<p class="text-[var(--text-muted)]">Registered</p>
						<p class="font-medium">{formatDate(host.created_at)}</p>
					</div>
					<div>
						<p class="text-[var(--text-muted)]">Updated</p>
						<p class="font-medium">{formatDate(host.updated_at)}</p>
					</div>
				</div>
			</SectionCard>

			{#if hostDetailSlotSurfaces.length > 0}
				<section class="mb-6" data-parity-region="host_detail.tabs">
					<div class="space-y-4">
						{#each hostDetailSlotSurfaces as surface (surface.surface_id)}
							<SectionCard title={surface.label}>
								{#if hasPermissionValue(getUser(), surface.required_permission)}
									<SurfaceReadPanel
										{surface}
										read={hostDetailSlotReads[surface.surface_id]}
										baseParams={hostDetailBaseParams}
										reloadToken={hostDetailReloadToken}
									/>
								{:else}
									<Callout
										tone="danger"
										title="Access denied"
										message="You do not have permission to access this surface."
									/>
								{/if}
							</SectionCard>
						{/each}
					</div>
				</section>
			{/if}

			<!-- Tags -->
			<section class="mb-6">
				<SectionCard title="Tags">
					<div class="mb-3 flex flex-wrap items-center justify-end gap-2">
						{#if canManage}
							<Button variant="secondary" size="sm" onclick={openSetTagsModal}>Set Tags</Button>
						{/if}
					</div>
					{#if host.tags.length === 0}
						<p class="text-sm text-[var(--text-muted)]">No tags assigned to this host.</p>
					{:else}
						<div class="flex flex-wrap gap-2">
							{#each host.tags as tag (tag.id)}
								<TagBadge name={tag.name} color={tag.color} />
							{/each}
						</div>
					{/if}
				</SectionCard>
			</section>

			<!-- Connected Agents -->
			<section class="mb-6">
				<SectionCard title="Connected Agents">
					{#if host.agents.length === 0}
						<p class="text-sm text-[var(--text-muted)]">No agents connected to this host.</p>
					{:else}
						<div class="table-wrap">
							<table class="table">
								<thead>
									<tr>
										<th>Name</th>
										<th>Status</th>
									</tr>
								</thead>
								<tbody>
									{#each host.agents as agent (agent.id)}
										<tr>
											<td>{agent.friendly_name}</td>
											<td>
												<StatusBadge tone={agentStatusTone(agent.status)} label={agent.status} />
											</td>
										</tr>
									{/each}
								</tbody>
							</table>
						</div>
					{/if}
				</SectionCard>
			</section>

			<!-- Assigned Software -->
			{#if canViewSoftware}
				<section class="mb-6">
					<SectionCard title="Assigned Software">
						{#if assignedSoftwareLoading}
							<p class="text-sm text-center py-4 text-[var(--text-muted)]">Loading...</p>
						{:else if assignedSoftwareError}
							<Callout tone="danger" title="Unable to load assigned software" message={assignedSoftwareError} />
						{:else if assignedSoftware.length === 0}
							<p class="text-sm text-[var(--text-muted)]">No software assigned to this host yet.</p>
						{:else}
							<div class="table-wrap">
								<table class="table">
									<thead>
										<tr>
											<th>Name</th>
											<th>Installed Version</th>
											<th>Latest Version</th>
											<th>Status</th>
											<th class="w-24">Details</th>
										</tr>
									</thead>
									<tbody>
										{#each assignedSoftware as item (item.id)}
											<tr>
												<td class="font-medium">{item.name}</td>
												<td class="text-sm text-[var(--text-muted)]" title={item.installed_version ?? undefined}
													>{formatVersion(
														resolveDisplayVersion(item.installed_version, item.installed_display_version)
													) ?? '—'}</td
												>
												<td class="text-sm text-[var(--text-muted)]" title={item.latest_version ?? undefined}
													>{formatVersion(
														resolveDisplayVersion(
															item.latest_version,
															item.latest_release_metadata?.display_version as string | undefined
														)
													) ?? '—'}</td
												>
												<td>
													{#if item.update_available}
														<StatusBadge tone="warning" label="Update Available" />
													{:else if item.latest_version}
														<StatusBadge tone="success" label="Up to date" />
													{:else}
														<StatusBadge tone="neutral" label="Unknown" />
													{/if}
												</td>
												<td>
													<Button variant="ghost" size="sm" href="/software/{item.id}">View</Button>
												</td>
											</tr>
										{/each}
									</tbody>
								</table>
							</div>
							<Pagination
								currentPage={assignedSoftwarePage}
								totalPages={assignedSoftwareTotalPages}
								total={assignedSoftwareTotal}
								onPageChange={(p) => loadAssignedSoftware(p)}
							/>
						{/if}
					</SectionCard>
				</section>
			{/if}

			<!-- Discovery Allowlist -->
			{#if canViewSoftware}
				<section class="mb-6">
					<SectionCard title="Discovery Allowlist">
						<div class="mb-3 flex flex-wrap items-center justify-between gap-2">
							<p class="mt-1 text-sm text-[var(--text-muted)]">
								{#if hostAllowlist.length === 0}
									No host-specific restrictions — tenant-wide allowlist applies, or all plugins run if none is
									configured.
								{:else}
									Host-specific entries are active. Only listed plugin types run discovery on this host, overriding
									tenant defaults.
								{/if}
							</p>
							{#if canManageSoftware}
								<Button variant="primary" size="sm" onclick={openAddAllowlistEntry}>Add Plugin Type</Button>
							{/if}
						</div>

						{#if hostAllowlistLoading}
							<p class="text-sm text-center py-4">Loading...</p>
						{:else if hostAllowlist.length === 0}
							<Callout
								tone="info"
								message="No host-specific allowlist configured. Add an entry to restrict which discovery plugins run on this host — any host-specific entries will override the tenant-wide allowlist completely."
							/>
						{:else}
							<div class="table-wrap">
								<table class="table">
									<thead>
										<tr>
											<th>Plugin Type</th>
											<th>Added</th>
											{#if canManageSoftware}<th class="w-24">Actions</th>{/if}
										</tr>
									</thead>
									<tbody>
										{#each hostAllowlist as entry (entry.id)}
											<tr>
												<td><StatusBadge tone="info" label={entry.plugin_type} /></td>
												<td>{formatDate(entry.created_at)}</td>
												{#if canManageSoftware}
													<td>
														<Button
															variant="danger"
															size="sm"
															onclick={() =>
																(allowlistDeleteConfirm = { id: entry.id, plugin_type: entry.plugin_type })}
														>
															Remove
														</Button>
													</td>
												{/if}
											</tr>
										{/each}
									</tbody>
								</table>
							</div>
						{/if}
					</SectionCard>
				</section>
			{/if}

			<!-- Recent Update History -->
			<section class="mb-6">
				<SectionCard title="Recent Update History">
					<div class="mb-3 flex items-center justify-between">
						<a href="/history?host_id={host.id}" class="text-sm text-primary-500 hover:underline"> View all → </a>
					</div>
					{#if recentHistory.length === 0}
						<p class="text-sm text-[var(--text-muted)]">No update history for this host.</p>
					{:else}
						<div class="table-wrap">
							<table class="table">
								<thead>
									<tr>
										<th>Software Item</th>
										<th>From</th>
										<th>To</th>
										<th>Status</th>
										<th>Date</th>
									</tr>
								</thead>
								<tbody>
									{#each recentHistory as entry (entry.id)}
										<tr>
											<td>
												<a href="/software/{entry.software_item_id}" class="hover:underline font-medium">
													{entry.software_item_name}
												</a>
											</td>
											<td title={entry.from_version ?? undefined}>{formatVersion(entry.from_version)}</td>
											<td title={entry.to_version}>{formatVersion(entry.to_version)}</td>
											<td>
												<StatusBadge tone={historyStatusTone(entry.status)} label={historyStatusLabel(entry.status)} />
											</td>
											<td>{formatDate(entry.created_at)}</td>
										</tr>
									{/each}
								</tbody>
							</table>
						</div>
					{/if}
				</SectionCard>
			</section>
		{/if}
	</PageShell>
{/if}

{#if confirmDeactivate && host}
	<ConfirmDialog
		title="Deactivate Host"
		messagePrefix="Are you sure you want to deactivate"
		entityName={host.friendly_name}
		confirmLabel="Deactivate"
		confirmDisabled={submitting}
		onconfirm={executeDeactivate}
		oncancel={() => (confirmDeactivate = false)}
	/>
{/if}

{#if editHost}
	<ModalShell title="Edit Host Name" onclose={cancelEdit}>
		<label class="label">
			<span>Friendly Name</span>
			<input class="input" type="text" bind:value={editHost.friendlyName} />
		</label>
		{#snippet footer()}
			<Button variant="secondary" onclick={cancelEdit}>Cancel</Button>
			<Button variant="primary" loading={submitting} onclick={executeEdit}>Save</Button>
		{/snippet}
	</ModalShell>
{/if}

<!-- Discovery allowlist modal -->
{#if showAllowlistModal}
	<ModalShell title="Add Discovery Plugin Type" onclose={closeAllowlistModal}>
		<p class="text-sm text-[var(--text-muted)]">
			Once any entry exists, only the listed plugin types will run discovery on this host.
		</p>

		<label class="label">
			<span>Plugin Type</span>
			<select class="select" bind:value={allowlistForm.plugin_type}>
				{#each discoveryPluginTypes as t (t.plugin_type)}
					<option value={t.plugin_type}>{t.display_name}</option>
				{/each}
			</select>
		</label>

		{#snippet footer()}
			<Button variant="secondary" onclick={closeAllowlistModal}>Cancel</Button>
			<Button variant="primary" disabled={!allowlistForm.plugin_type.trim()} onclick={saveAllowlistEntry}>Add</Button>
		{/snippet}
	</ModalShell>
{/if}

{#if showSetTagsModal}
	<ModalShell title="Set Tags" onclose={() => (showSetTagsModal = false)}>
		{#if allTags.length === 0}
			<p class="text-sm text-[var(--text-muted)]">
				No tags available. <a href="/host-tags" class="text-primary-500 hover:underline">Create a tag</a> first.
			</p>
		{:else}
			<p class="mb-3 text-sm text-[var(--text-muted)]">Select the tags to assign to this host.</p>
			<CheckboxList items={tagItems} selected={selectedTagIds} maxHeight="max-h-64" showCounter={false} />
		{/if}
		{#snippet footer()}
			<Button variant="secondary" onclick={() => (showSetTagsModal = false)}>Cancel</Button>
			<Button variant="primary" loading={submitting} onclick={executeSetTags}>Save</Button>
		{/snippet}
	</ModalShell>
{/if}

{#if allowlistDeleteConfirm}
	<ConfirmDialog
		title="Remove Allowlist Entry"
		messagePrefix="Remove discovery plugin type"
		entityName={allowlistDeleteConfirm.plugin_type}
		confirmLabel="Remove"
		onconfirm={executeDeleteAllowlistEntry}
		oncancel={() => (allowlistDeleteConfirm = null)}
	/>
{/if}
