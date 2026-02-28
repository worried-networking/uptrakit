<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import { getHost, listUpdateHistory, updateHost, deactivateHost, triggerHostDiscovery } from '$lib/api';
	import { formatDate } from '$lib/utils';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import ModalBackdrop from '$lib/components/ModalBackdrop.svelte';
	import { Permission } from '$lib/types';
	import type { HostResponse, UpdateHistoryResponse, ServiceStatus, UpdateHistoryStatus } from '$lib/types';

	const id = $derived(page.params.id as string);

	let host: HostResponse | null = $state(null);
	let recentHistory: UpdateHistoryResponse[] = $state([]);
	let loading: boolean = $state(true);
	let error: string | null = $state(null);

	let editHost: { friendlyName: string } | null = $state(null);
	let submitting: boolean = $state(false);
	let confirmDeactivate: boolean = $state(false);
	let discovering: boolean = $state(false);

	const canManage = $derived(getUser()?.permissions.includes(Permission.ManageHosts) ?? false);
	const canManageSoftware = $derived(getUser()?.permissions.includes(Permission.ManageSoftware) ?? false);

	let refreshInterval: ReturnType<typeof setInterval> | null = null;

	onMount(() => {
		loadData();
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

<svelte:window
	onkeydown={(e) => {
		if (e.key === 'Escape') {
			if (editHost) cancelEdit();
			else if (confirmDeactivate) confirmDeactivate = false;
		}
	}}
/>

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
									<td>{entry.from_version ?? '—'}</td>
									<td>{entry.to_version}</td>
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
	<ModalBackdrop onclose={cancelEdit}>
		<div
			class="card bg-surface-50 dark:bg-surface-900 w-full max-w-md space-y-4 p-6 shadow-xl"
			role="dialog"
			aria-modal="true"
		>
			<h3 class="h3">Edit Host Name</h3>
			<label class="label">
				<span>Friendly Name</span>
				<input class="input" type="text" bind:value={editHost.friendlyName} />
			</label>
			<div class="flex justify-end gap-2">
				<button class="btn preset-tonal-surface" onclick={cancelEdit}>Cancel</button>
				<button class="btn preset-filled-primary-500" disabled={submitting} onclick={executeEdit}>
					{submitting ? 'Saving...' : 'Save'}
				</button>
			</div>
		</div>
	</ModalBackdrop>
{/if}
