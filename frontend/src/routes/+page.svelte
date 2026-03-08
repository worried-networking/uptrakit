<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { getUser } from '$lib/auth.svelte';
	import { getHosts, getServices, getSoftwareItems, listUpdateHistory } from '$lib/api';
	import { formatDate } from '$lib/utils';
	import { subscribeToEvent } from '$lib/stores/events.svelte';
	import { Permission } from '$lib/types';
	import type {
		HostResponse,
		ServiceResponse,
		SoftwareItemResponse,
		UpdateHistoryResponse,
		PaginatedResponse
	} from '$lib/types';

	// --- Dashboard state ---
	let loading = $state(true);
	let error: string | null = $state(null);

	let hosts: HostResponse[] = $state([]);
	let totalHosts = $state(0);
	let services: ServiceResponse[] = $state([]);
	let totalServices = $state(0);
	let softwareItems: SoftwareItemResponse[] = $state([]);
	let totalSoftwareItems = $state(0);
	let pendingSoftwareCount = $state(0);
	let recentUpdates: UpdateHistoryResponse[] = $state([]);
	let totalRecentUpdates = $state(0);

	// --- Derived stats ---
	const hostsWithUpdates = $derived(hosts.filter((h) => h.update_summary.available_updates_count > 0).length);
	const hostsWithSecurityUpdates = $derived(hosts.filter((h) => h.update_summary.security_updates_count > 0).length);
	const pendingServices = $derived(services.filter((s) => s.status === 'pending').length);
	const failedUpdates = $derived(recentUpdates.filter((u) => u.status === 'failed').length);
	const itemsWithUpdates = $derived(softwareItems.filter((i) => i.update_available).length);

	const hasAttentionItems = $derived(
		pendingServices > 0 ||
			pendingSoftwareCount > 0 ||
			hostsWithUpdates > 0 ||
			hostsWithSecurityUpdates > 0 ||
			failedUpdates > 0
	);

	// Hosts that need attention (have updates or security updates)
	const hostsNeedingAttention = $derived(
		hosts
			.filter((h) => h.update_summary.available_updates_count > 0 || h.update_summary.security_updates_count > 0)
			.slice(0, 5)
	);

	// --- Permissions ---
	const canViewHosts = $derived(getUser()?.permissions.includes(Permission.ViewHosts) ?? false);
	const canViewAgents = $derived(getUser()?.permissions.includes(Permission.ViewAgents) ?? false);
	const canViewSoftware = $derived(getUser()?.permissions.includes(Permission.ViewSoftware) ?? false);

	let refreshInterval: ReturnType<typeof setInterval> | null = null;
	let unsubscribers: (() => void)[] = [];

	onMount(() => {
		loadDashboard();

		unsubscribers.push(
			subscribeToEvent('host_updated', () => loadDashboard(true)),
			subscribeToEvent('host_created', () => loadDashboard(true)),
			subscribeToEvent('service_status_changed', () => loadDashboard(true)),
			subscribeToEvent('software_item_updated', () => loadDashboard(true)),
			subscribeToEvent('software_item_created', () => loadDashboard(true)),
			subscribeToEvent('version_check_completed', () => loadDashboard(true)),
			subscribeToEvent('update_completed', () => loadDashboard(true)),
			subscribeToEvent('host_packages_changed', () => loadDashboard(true))
		);

		refreshInterval = setInterval(() => {
			if (document.visibilityState === 'visible') loadDashboard(true);
		}, 300_000);
	});

	onDestroy(() => {
		for (const unsub of unsubscribers) unsub();
		if (refreshInterval) clearInterval(refreshInterval);
	});

	async function loadDashboard(background = false) {
		if (!background) {
			loading = true;
			error = null;
		}

		// Build promises based on permissions, using large perPage to get good counts
		const promises: Promise<void>[] = [];

		if (canViewHosts) {
			promises.push(
				getHosts(1, 100)
					.then((result: PaginatedResponse<HostResponse>) => {
						hosts = result.items;
						totalHosts = result.total;
					})
					.catch(() => {
						/* non-fatal */
					})
			);
		}

		if (canViewAgents) {
			promises.push(
				getServices({ page: 1, perPage: 100 })
					.then((result: PaginatedResponse<ServiceResponse>) => {
						services = result.items;
						totalServices = result.total;
					})
					.catch(() => {
						/* non-fatal */
					})
			);
		}

		if (canViewSoftware) {
			promises.push(
				getSoftwareItems(1, 100, 'approved')
					.then((result: PaginatedResponse<SoftwareItemResponse>) => {
						softwareItems = result.items;
						totalSoftwareItems = result.total;
					})
					.catch(() => {
						/* non-fatal */
					})
			);

			promises.push(
				getSoftwareItems(1, 1, 'pending')
					.then((result: PaginatedResponse<SoftwareItemResponse>) => {
						pendingSoftwareCount = result.total;
					})
					.catch(() => {
						/* non-fatal */
					})
			);

			promises.push(
				listUpdateHistory({ page: 1, per_page: 5 })
					.then((result: PaginatedResponse<UpdateHistoryResponse>) => {
						recentUpdates = result.items;
						totalRecentUpdates = result.total;
					})
					.catch(() => {
						/* non-fatal */
					})
			);
		}

		try {
			await Promise.all(promises);
			if (background) error = null;
		} catch (e) {
			if (!background) {
				error = e instanceof Error ? e.message : 'Failed to load dashboard data';
			}
		} finally {
			if (!background) loading = false;
		}
	}

	function statusBadgeClass(status: string): string {
		switch (status) {
			case 'completed':
				return 'preset-filled-success-500';
			case 'failed':
				return 'preset-filled-error-500';
			case 'in_progress':
				return 'preset-filled-primary-500';
			case 'pending':
			case 'queued':
				return 'preset-filled-warning-500';
			default:
				return 'preset-tonal';
		}
	}
</script>

{#if getUser()}
	<h1 class="h1 mb-6">Dashboard</h1>

	{#if error}
		<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
			<p>{error}</p>
			<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadDashboard()}> Retry </button>
		</aside>
	{/if}

	{#if loading}
		<div class="py-12 text-center text-surface-500">Loading dashboard...</div>
	{:else}
		<!-- Summary cards -->
		<div class="mb-8 grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
			{#if canViewHosts}
				<a href="/hosts" class="card p-5 hover:ring-1 hover:ring-primary-500 transition-shadow">
					<p class="text-sm font-medium text-surface-500">Hosts</p>
					<p class="mt-1 text-3xl font-bold">{totalHosts}</p>
					<p class="mt-1 text-sm text-surface-400">
						{#if hostsWithUpdates > 0}
							{hostsWithUpdates} with updates available
						{:else}
							All up to date
						{/if}
					</p>
					{#if hostsWithSecurityUpdates > 0}
						<p class="text-sm text-error-500">
							{hostsWithSecurityUpdates} with security updates
						</p>
					{/if}
				</a>
			{/if}

			{#if canViewAgents}
				<a href="/services" class="card p-5 hover:ring-1 hover:ring-primary-500 transition-shadow">
					<p class="text-sm font-medium text-surface-500">Services</p>
					<p class="mt-1 text-3xl font-bold">{totalServices}</p>
					<p class="mt-1 text-sm text-surface-400">
						{#if pendingServices > 0}
							<span class="text-warning-500">{pendingServices} pending approval</span>
						{:else}
							No pending approvals
						{/if}
					</p>
				</a>
			{/if}

			{#if canViewSoftware}
				<a href="/software" class="card p-5 hover:ring-1 hover:ring-primary-500 transition-shadow">
					<p class="text-sm font-medium text-surface-500">Software Items</p>
					<p class="mt-1 text-3xl font-bold">{totalSoftwareItems}</p>
					<p class="mt-1 text-sm text-surface-400">
						{#if pendingSoftwareCount > 0}
							<span class="text-warning-500">{pendingSoftwareCount} pending review</span>
						{:else if itemsWithUpdates > 0}
							{itemsWithUpdates} with updates available
						{:else}
							All up to date
						{/if}
					</p>
				</a>

				<a href="/history" class="card p-5 hover:ring-1 hover:ring-primary-500 transition-shadow">
					<p class="text-sm font-medium text-surface-500">Update History</p>
					<p class="mt-1 text-3xl font-bold">{totalRecentUpdates}</p>
					<p class="mt-1 text-sm text-surface-400">
						{#if failedUpdates > 0}
							<span class="text-error-500">{failedUpdates} failed recently</span>
						{:else}
							total updates recorded
						{/if}
					</p>
				</a>
			{/if}
		</div>

		<!-- Attention needed -->
		{#if hasAttentionItems}
			<section class="mb-8">
				<h2 class="h2 mb-3">Attention Needed</h2>
				<div class="space-y-2">
					{#if pendingServices > 0 && canViewAgents}
						<a
							href="/services?status=pending"
							class="card flex items-center gap-3 p-4 hover:ring-1 hover:ring-warning-500 transition-shadow"
						>
							<span
								class="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full preset-filled-warning-500 text-sm font-bold"
							>
								{pendingServices}
							</span>
							<span class="text-sm">
								{pendingServices === 1 ? 'service awaiting approval' : 'services awaiting approval'}
							</span>
						</a>
					{/if}

					{#if pendingSoftwareCount > 0 && canViewSoftware}
						<a
							href="/software?tab=pending"
							class="card flex items-center gap-3 p-4 hover:ring-1 hover:ring-warning-500 transition-shadow"
						>
							<span
								class="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full preset-filled-warning-500 text-sm font-bold"
							>
								{pendingSoftwareCount}
							</span>
							<span class="text-sm">
								discovered software {pendingSoftwareCount === 1 ? 'item' : 'items'} pending review
							</span>
						</a>
					{/if}

					{#if hostsWithSecurityUpdates > 0 && canViewHosts}
						<a
							href="/hosts"
							class="card flex items-center gap-3 p-4 hover:ring-1 hover:ring-error-500 transition-shadow"
						>
							<span
								class="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full preset-filled-error-500 text-sm font-bold"
							>
								{hostsWithSecurityUpdates}
							</span>
							<span class="text-sm">
								{hostsWithSecurityUpdates === 1 ? 'host has' : 'hosts have'} security updates available
							</span>
						</a>
					{/if}

					{#if hostsWithUpdates > 0 && canViewHosts}
						<a
							href="/hosts"
							class="card flex items-center gap-3 p-4 hover:ring-1 hover:ring-warning-500 transition-shadow"
						>
							<span
								class="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full preset-filled-warning-500 text-sm font-bold"
							>
								{hostsWithUpdates}
							</span>
							<span class="text-sm">
								{hostsWithUpdates === 1 ? 'host has' : 'hosts have'} package updates available
							</span>
						</a>
					{/if}

					{#if failedUpdates > 0 && canViewSoftware}
						<a
							href="/history?status=failed"
							class="card flex items-center gap-3 p-4 hover:ring-1 hover:ring-error-500 transition-shadow"
						>
							<span
								class="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full preset-filled-error-500 text-sm font-bold"
							>
								{failedUpdates}
							</span>
							<span class="text-sm">
								recent {failedUpdates === 1 ? 'update has' : 'updates have'} failed
							</span>
						</a>
					{/if}
				</div>
			</section>
		{/if}

		<!-- Two-column layout for tables -->
		<div class="grid grid-cols-1 gap-6 lg:grid-cols-2">
			<!-- Recent updates -->
			{#if canViewSoftware}
				<section>
					<div class="mb-3 flex items-center justify-between">
						<h2 class="h2">Recent Updates</h2>
						{#if totalRecentUpdates > 5}
							<a href="/history" class="btn btn-sm preset-tonal">View all</a>
						{/if}
					</div>
					<div class="table-wrap">
						<table class="table">
							<thead>
								<tr>
									<th>Software</th>
									<th>Host</th>
									<th>Status</th>
									<th>Date</th>
								</tr>
							</thead>
							<tbody>
								{#each recentUpdates as entry (entry.id)}
									<tr>
										<td class="text-sm font-medium">
											{entry.software_item_name}
										</td>
										<td class="text-sm text-surface-500">
											{entry.host_name}
										</td>
										<td>
											<span class="badge {statusBadgeClass(entry.status)}">
												{entry.status}
											</span>
										</td>
										<td class="text-sm text-surface-500">
											{formatDate(entry.created_at)}
										</td>
									</tr>
								{:else}
									<tr>
										<td colspan="4" class="py-8 text-center">
											<p class="text-lg font-medium">No updates yet</p>
											<p class="mt-1 text-sm text-surface-500">Updates will appear here once triggered.</p>
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
				</section>
			{/if}

			<!-- Hosts needing attention -->
			{#if canViewHosts}
				<section>
					<div class="mb-3 flex items-center justify-between">
						<h2 class="h2">Hosts with Updates</h2>
						{#if hostsNeedingAttention.length > 0}
							<a href="/hosts" class="btn btn-sm preset-tonal">View all</a>
						{/if}
					</div>
					<div class="table-wrap">
						<table class="table">
							<thead>
								<tr>
									<th>Host</th>
									<th>Updates</th>
									<th>Security</th>
								</tr>
							</thead>
							<tbody>
								{#each hostsNeedingAttention as host (host.id)}
									<tr>
										<td>
											<p class="text-sm font-medium">
												{host.friendly_name || host.hostname}
											</p>
											{#if host.friendly_name && host.friendly_name !== host.hostname}
												<p class="text-xs text-surface-500">
													{host.hostname}
												</p>
											{/if}
										</td>
										<td>
											{#if host.update_summary.available_updates_count > 0}
												<span class="badge preset-filled-warning-500">
													{host.update_summary.available_updates_count}
												</span>
											{:else}
												<span class="text-surface-400">&mdash;</span>
											{/if}
										</td>
										<td>
											{#if host.update_summary.security_updates_count > 0}
												<span class="badge preset-filled-error-500">
													{host.update_summary.security_updates_count}
												</span>
											{:else}
												<span class="text-surface-400">&mdash;</span>
											{/if}
										</td>
									</tr>
								{:else}
									<tr>
										<td colspan="3" class="py-8 text-center">
											<p class="text-lg font-medium">All hosts up to date</p>
											<p class="mt-1 text-sm text-surface-500">No hosts have pending updates.</p>
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
				</section>
			{/if}
		</div>
	{/if}
{/if}
