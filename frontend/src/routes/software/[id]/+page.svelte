<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { page } from '$app/stores';
	import { getUser } from '$lib/auth.svelte';
	import {
		getSoftwareItem,
		checkSoftwareItemVersions,
		checkSoftwareItemVersionsHost,
		triggerSoftwareUpdate
	} from '$lib/api';
	import { formatDate } from '$lib/utils';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import ModalBackdrop from '$lib/components/ModalBackdrop.svelte';
	import { Permission } from '$lib/types';
	import type { SoftwareItemDetailResponse, SoftwareItemHostSummary } from '$lib/types';

	const id = $derived($page.params.id as string);

	let item: SoftwareItemDetailResponse | null = $state(null);
	let loading: boolean = $state(true);
	let error: string | null = $state(null);
	let checkingAll: boolean = $state(false);
	let checkingHostId: string | null = $state(null);

	// Update confirm modal state
	let updateModal: { host: SoftwareItemHostSummary; toVersion: string } | null = $state(null);
	let updateTriggering: boolean = $state(false);

	const canView = $derived(getUser()?.permissions.includes(Permission.ViewSoftware) ?? false);
	const canManage = $derived(getUser()?.permissions.includes(Permission.ManageSoftware) ?? false);

	let refreshInterval: ReturnType<typeof setInterval> | null = null;

	onMount(() => {
		if (canView) {
			loadItem();
			refreshInterval = setInterval(() => {
				if (document.visibilityState === 'visible') loadItem(true);
			}, 30_000);
		}
	});

	onDestroy(() => {
		if (refreshInterval) clearInterval(refreshInterval);
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
			// Refresh data after a short delay to pick up results.
			setTimeout(() => loadItem(true), 2000);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to trigger version check');
		} finally {
			checkingHostId = null;
		}
	}

	function openUpdateModal(host: SoftwareItemHostSummary) {
		const toVersion = host.latest_version ?? item?.latest_version ?? '';
		updateModal = { host, toVersion };
	}

	async function executeUpdate() {
		if (!item || !updateModal || updateTriggering) return;
		updateTriggering = true;
		try {
			const res = await triggerSoftwareUpdate(item.id, updateModal.host.host_id, {
				to_version: updateModal.toVersion
			});
			showSuccess(`Update triggered — history ID: ${res.update_history_id}`);
			updateModal = null;
			setTimeout(() => loadItem(true), 1000);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to trigger update');
		} finally {
			updateTriggering = false;
		}
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

<svelte:window
	onkeydown={(e) => {
		if (e.key === 'Escape' && updateModal) updateModal = null;
	}}
/>

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
					{#if item.enabled}
						<span class="badge preset-filled-success-500">Enabled</span>
					{:else}
						<span class="badge preset-tonal">Disabled</span>
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
							Latest version: <span class="font-medium text-surface-700 dark:text-surface-300"
								>{item.latest_version}</span
							>
						</p>
					{/if}
					<p>Last checked: {formatDate(item.last_checked_at)}</p>
					<p>{item.host_count} host{item.host_count !== 1 ? 's' : ''} assigned</p>
				</div>
			</div>
			{#if canManage}
				<button class="btn preset-tonal-surface" onclick={checkAllVersions} disabled={checkingAll}>
					{checkingAll ? 'Checking…' : 'Check All Versions'}
				</button>
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
							<th>Actions</th>
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
						{#each item.hosts as host (host.host_id)}
							<tr>
								<td>
									<a href="/hosts/{host.host_id}" class="hover:underline font-medium">{host.hostname}</a>
									{#if host.friendly_name && host.friendly_name !== host.hostname}
										<span class="block text-xs text-surface-500">{host.friendly_name}</span>
									{/if}
								</td>
								<td>{host.installed_version ?? '—'}</td>
								<td>{host.latest_version ?? item.latest_version ?? '—'}</td>
								<td>
									<span class="badge {versionStatusClass(host)}">{versionStatusLabel(host)}</span>
								</td>
								<td>{formatDate(host.installed_version_detected_at)}</td>
								{#if canManage}
									<td class="space-x-2 whitespace-nowrap">
										{#if host.update_available || item.latest_version}
											<button class="btn btn-sm preset-filled-warning-500" onclick={() => openUpdateModal(host)}>
												Update to {host.latest_version ?? item?.latest_version}
											</button>
										{/if}
										<button
											class="btn btn-sm preset-tonal"
											onclick={() => checkHostVersions(host.host_id)}
											disabled={checkingHostId === host.host_id}
										>
											{checkingHostId === host.host_id ? 'Checking…' : 'Check Versions'}
										</button>
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

{#if updateModal}
	<ModalBackdrop onclose={() => (updateModal = null)}>
		<div
			class="card bg-surface-50 dark:bg-surface-900 w-full max-w-md space-y-4 p-6 shadow-xl"
			role="dialog"
			aria-modal="true"
		>
			<h3 class="h3">Confirm Update</h3>

			<p class="text-sm">
				Update <strong>{item?.name}</strong> on <strong>{updateModal.host.hostname}</strong>?
			</p>
			<div class="grid grid-cols-2 gap-4 text-sm">
				<div>
					<p class="text-surface-500">From</p>
					<p class="font-medium">{updateModal.host.installed_version ?? 'unknown'}</p>
				</div>
				<div>
					<p class="text-surface-500">To</p>
					<p class="font-medium">{updateModal.toVersion}</p>
				</div>
			</div>

			<div class="flex justify-end gap-2">
				<button class="btn preset-tonal-surface" onclick={() => (updateModal = null)}>Cancel</button>
				<button class="btn preset-filled-warning-500" onclick={executeUpdate} disabled={updateTriggering}>
					{updateTriggering ? 'Triggering…' : 'Trigger Update'}
				</button>
			</div>
		</div>
	</ModalBackdrop>
{/if}
