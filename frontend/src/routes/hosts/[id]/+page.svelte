<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
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
		listPluginTypes
	} from '$lib/api';
	import { formatDate, formatVersion } from '$lib/utils';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import Modal from '$lib/components/Modal.svelte';
	import { Permission, PluginCapability } from '$lib/types';
	import type {
		HostResponse,
		UpdateHistoryResponse,
		ServiceStatus,
		UpdateHistoryStatus,
		HostDiscoveryAllowlistEntry,
		PluginTypeInfo
	} from '$lib/types';

	const id = $derived(page.params.id as string);

	let host: HostResponse | null = $state(null);
	let recentHistory: UpdateHistoryResponse[] = $state([]);
	let loading: boolean = $state(true);
	let error: string | null = $state(null);

	let editHost: { friendlyName: string } | null = $state(null);
	let submitting: boolean = $state(false);
	let confirmDeactivate: boolean = $state(false);
	let discovering: boolean = $state(false);

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

	const canManage = $derived(getUser()?.permissions.includes(Permission.ManageHosts) ?? false);
	const canManageSoftware = $derived(getUser()?.permissions.includes(Permission.ManageSoftware) ?? false);
	const canViewSoftware = $derived(getUser()?.permissions.includes(Permission.ViewSoftware) ?? false);

	let refreshInterval: ReturnType<typeof setInterval> | null = null;

	onMount(() => {
		loadData();
		if (canViewSoftware) {
			loadPluginTypes();
			loadHostAllowlist();
		}
		refreshInterval = setInterval(() => {
			if (document.visibilityState === 'visible') loadData(true);
		}, 30_000);
	});

	onDestroy(() => {
		if (refreshInterval) clearInterval(refreshInterval);
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
		} catch (e) {
			if (!background) {
				error = e instanceof Error ? e.message : 'Failed to load host';
			}
		} finally {
			if (!background) loading = false;
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

	function openAddAllowlistEntry() {
		allowlistForm = { plugin_type: discoveryPluginTypes[0]?.plugin_type ?? '' };
		showAllowlistModal = true;
	}

	function closeAllowlistModal() {
		showAllowlistModal = false;
	}

	async function saveAllowlistEntry() {
		try {
			const created = await addHostDiscoveryAllowlistEntry(id, { plugin_type: allowlistForm.plugin_type });
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

	function agentStatusClass(status: ServiceStatus): string {
		switch (status) {
			case 'approved':
				return 'preset-filled-success-500';
			case 'pending':
				return 'preset-tonal';
			case 'rejected':
				return 'preset-filled-error-500';
			case 'deactivated':
				return 'preset-tonal';
		}
	}

	function historyStatusClass(status: UpdateHistoryStatus): string {
		switch (status) {
			case 'completed':
				return 'preset-filled-success-500';
			case 'failed':
				return 'preset-filled-error-500';
			case 'in_progress':
				return 'preset-filled-warning-500';
			case 'pending':
				return 'preset-tonal';
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
			case 'pending':
				return 'Pending';
		}
	}
</script>

{#if getUser()}
	<div class="mb-4">
		<a href="/hosts" class="text-sm text-surface-500 hover:underline">← Back to Hosts</a>
	</div>

	{#if loading}
		<p>Loading...</p>
	{:else if error}
		<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
			<p>{error}</p>
			<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadData()}>Retry</button>
		</aside>
	{:else if host}
		<!-- Header -->
		<div class="mb-6 flex flex-wrap items-start justify-between gap-4">
			<div>
				<h1 class="h1">{host.friendly_name}</h1>
				{#if host.friendly_name !== host.hostname}
					<p class="mt-1 text-sm text-surface-500">{host.hostname}</p>
				{/if}
				<p class="mt-1 text-sm text-surface-500">Last seen: {formatDate(host.last_seen_at)}</p>
			</div>
			<div class="flex flex-wrap items-center gap-2">
				{#if canManage}
					<button class="btn preset-tonal-surface" onclick={openEditDialog}> Edit Name </button>
					<button class="btn preset-filled-error-500" onclick={() => (confirmDeactivate = true)} disabled={submitting}>
						Deactivate
					</button>
				{/if}
				{#if canManageSoftware}
					<button class="btn preset-tonal-surface" onclick={triggerDiscovery} disabled={discovering}>
						{discovering ? 'Triggering…' : 'Trigger Discovery'}
					</button>
				{/if}
			</div>
		</div>

		<!-- Info grid -->
		<div class="card bg-surface-50 dark:bg-surface-900 mb-6 p-4">
			<div class="grid grid-cols-2 gap-x-8 gap-y-3 sm:grid-cols-4 text-sm">
				<div>
					<p class="text-surface-500">OS</p>
					<p class="font-medium">{host.os_version ?? host.os_type ?? '—'}</p>
				</div>
				<div>
					<p class="text-surface-500">Architecture</p>
					<p class="font-medium">{host.architecture ?? '—'}</p>
				</div>
				<div>
					<p class="text-surface-500">IP Address</p>
					<p class="font-medium">{host.ip_address ?? '—'}</p>
				</div>
				<div>
					<p class="text-surface-500">Machine ID</p>
					<p class="font-medium break-all">{host.machine_id}</p>
				</div>
				<div>
					<p class="text-surface-500">Registered</p>
					<p class="font-medium">{formatDate(host.created_at)}</p>
				</div>
				<div>
					<p class="text-surface-500">Updated</p>
					<p class="font-medium">{formatDate(host.updated_at)}</p>
				</div>
			</div>
		</div>

		<!-- Connected Agents -->
		<section class="mb-6">
			<h2 class="h3 mb-3">Connected Agents</h2>
			{#if host.agents.length === 0}
				<p class="text-sm text-surface-500">No agents connected to this host.</p>
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
										<span class="badge {agentStatusClass(agent.status)}">{agent.status}</span>
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		</section>

		<!-- Package Updates Summary -->
		{#if canViewSoftware}
			<section class="mb-6">
				<div class="mb-3 flex items-center justify-between">
					<h2 class="h3">Package Updates</h2>
					<a href="/hosts/{host.id}/packages" class="text-sm text-primary-500 hover:underline">View all packages →</a>
				</div>
				<div class="card bg-surface-50 dark:bg-surface-900 p-4">
					<div class="flex flex-wrap gap-6 text-sm">
						<div>
							<p class="text-surface-500">Available Updates</p>
							<p class="text-2xl font-semibold">
								{#if host.update_summary.available_updates_count > 0}
									<a href="/hosts/{host.id}/packages?has_update=true" class="text-warning-500 hover:underline">
										{host.update_summary.available_updates_count}
									</a>
								{:else}
									<span class="text-surface-400">0</span>
								{/if}
							</p>
						</div>
						<div>
							<p class="text-surface-500">Security Updates</p>
							<p class="text-2xl font-semibold">
								{#if host.update_summary.security_updates_count > 0}
									<a
										href="/hosts/{host.id}/packages?category=security&has_update=true"
										class="text-error-500 hover:underline"
									>
										{host.update_summary.security_updates_count}
									</a>
								{:else}
									<span class="text-surface-400">0</span>
								{/if}
							</p>
						</div>
					</div>
				</div>
			</section>
		{/if}

		<!-- Discovery Allowlist -->
		{#if canViewSoftware}
			<section class="mb-6">
				<div class="mb-3 flex flex-wrap items-center justify-between gap-2">
					<div>
						<h2 class="h3">Discovery Allowlist</h2>
						<p class="mt-1 text-sm text-surface-500">
							{#if hostAllowlist.length === 0}
								No host-specific restrictions — tenant-wide allowlist applies, or all plugins run if none is configured.
							{:else}
								Host-specific entries are active. Only the listed plugin types will run discovery on this host,
								overriding the tenant-wide allowlist entirely.
							{/if}
						</p>
					</div>
					{#if canManageSoftware}
						<button class="btn btn-sm preset-filled-primary-500" onclick={openAddAllowlistEntry}>
							Add Plugin Type
						</button>
					{/if}
				</div>

				{#if hostAllowlistLoading}
					<p class="text-sm text-center py-4">Loading...</p>
				{:else if hostAllowlist.length === 0}
					<aside class="rounded-lg p-4 preset-tonal-surface text-sm">
						<p>
							No host-specific allowlist configured. Add an entry to restrict which discovery plugins run on this host —
							any host-specific entries will override the tenant-wide allowlist completely.
						</p>
					</aside>
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
										<td><span class="badge preset-tonal">{entry.plugin_type}</span></td>
										<td>{formatDate(entry.created_at)}</td>
										{#if canManageSoftware}
											<td>
												<button
													class="btn btn-sm preset-tonal-error"
													onclick={() => (allowlistDeleteConfirm = { id: entry.id, plugin_type: entry.plugin_type })}
												>
													Remove
												</button>
											</td>
										{/if}
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
				{/if}
			</section>
		{/if}

		<!-- Recent Update History -->
		<section class="mb-6">
			<div class="mb-3 flex items-center justify-between">
				<h2 class="h3">Recent Update History</h2>
				<a href="/history?host_id={host.id}" class="text-sm text-primary-500 hover:underline"> View all → </a>
			</div>
			{#if recentHistory.length === 0}
				<p class="text-sm text-surface-500">No update history for this host.</p>
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
										<span class="badge {historyStatusClass(entry.status)}">
											{historyStatusLabel(entry.status)}
										</span>
									</td>
									<td>{formatDate(entry.created_at)}</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		</section>
	{/if}
{/if}

{#if confirmDeactivate && host}
	<ConfirmDialog
		title="Deactivate Host"
		messagePrefix="Are you sure you want to deactivate"
		entityName={host.friendly_name}
		confirmLabel={submitting ? 'Processing...' : 'Deactivate'}
		confirmDisabled={submitting}
		onconfirm={executeDeactivate}
		oncancel={() => (confirmDeactivate = false)}
	/>
{/if}

{#if editHost}
	<Modal title="Edit Host Name" onclose={cancelEdit}>
		<label class="label">
			<span>Friendly Name</span>
			<input class="input" type="text" bind:value={editHost.friendlyName} />
		</label>
		{#snippet footer()}
			<button class="btn preset-tonal-surface" onclick={cancelEdit}>Cancel</button>
			<button class="btn preset-filled-primary-500" disabled={submitting} onclick={executeEdit}>
				{submitting ? 'Saving...' : 'Save'}
			</button>
		{/snippet}
	</Modal>
{/if}

<!-- Discovery allowlist modal -->
{#if showAllowlistModal}
	<Modal title="Add Discovery Plugin Type" onclose={closeAllowlistModal}>
		<p class="text-sm text-surface-500">
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
			<button class="btn preset-tonal-surface" onclick={closeAllowlistModal}>Cancel</button>
			<button class="btn preset-filled-primary-500" onclick={saveAllowlistEntry}>Add</button>
		{/snippet}
	</Modal>
{/if}

{#if allowlistDeleteConfirm}
	<ConfirmDialog
		title="Remove Allowlist Entry"
		messagePrefix="Remove discovery plugin type"
		entityName={allowlistDeleteConfirm.plugin_type}
		confirmLabel="Remove"
		confirmClass="preset-filled-error-500"
		onconfirm={executeDeleteAllowlistEntry}
		oncancel={() => (allowlistDeleteConfirm = null)}
	/>
{/if}
