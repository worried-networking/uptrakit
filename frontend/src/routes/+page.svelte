<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { getUser } from '$lib/auth.svelte';
	import { getHosts, getServices, getSoftwareItems, listUpdateHistory } from '$lib/api';
	import { formatDate } from '$lib/utils';
	import { subscribeToEvent } from '$lib/stores/events.svelte';
	import { Permission } from '$lib/types';
	import type { ServiceResponse, UpdateHistoryResponse, PaginatedResponse } from '$lib/types';
	import { Callout, DataTable, PageShell, SectionCard, StatusBadge } from '$lib/components/ui';

	// --- Dashboard state ---
	let loading = $state(true);
	let error: string | null = $state(null);

	let totalHosts = $state(0);
	let services: ServiceResponse[] = $state([]);
	let totalServices = $state(0);
	let totalSoftwareItems = $state(0);
	let unfeaturedSoftwareCount = $state(0);
	let recentUpdates: UpdateHistoryResponse[] = $state([]);
	let totalRecentUpdates = $state(0);

	// --- Derived stats ---
	const pendingServices = $derived(services.filter((s) => s.status === 'pending').length);
	const pendingUpdateCount = $derived(
		recentUpdates.filter((u) => u.status === 'queued' || u.status === 'pending' || u.status === 'in_progress').length
	);
	const failedUpdates = $derived(recentUpdates.filter((u) => u.status === 'failed').length);
	const hasAttentionItems = $derived(pendingServices > 0 || failedUpdates > 0);

	// --- Permissions ---
	const canViewHosts = $derived(getUser()?.permissions.includes(Permission.ViewHosts) ?? false);
	const canViewAgents = $derived(getUser()?.permissions.includes(Permission.ViewServices) ?? false);
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
			subscribeToEvent('update_completed', () => loadDashboard(true))
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
				getHosts(1, 1)
					.then((result) => {
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
				getSoftwareItems(1, 1, true)
					.then((result) => {
						totalSoftwareItems = result.total;
					})
					.catch(() => {
						/* non-fatal */
					})
			);

			promises.push(
				getSoftwareItems(1, 1, false)
					.then((result) => {
						unfeaturedSoftwareCount = result.total;
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

	function statusBadgeTone(status: string): 'neutral' | 'info' | 'success' | 'warning' | 'danger' {
		switch (status) {
			case 'completed':
				return 'success';
			case 'failed':
				return 'danger';
			case 'in_progress':
				return 'info';
			case 'pending':
			case 'queued':
				return 'warning';
			default:
				return 'neutral';
		}
	}
</script>

{#if getUser()}
	<PageShell title="Dashboard" description="Overview of hosts, services, and update activity across your environment.">
		{#if error}
			<Callout tone="danger" message={error}>
				<button class="btn preset-filled-primary-500 mt-3" onclick={() => loadDashboard()}>Retry</button>
			</Callout>
		{/if}

		{#if loading}
			<SectionCard>
				<p class="py-12 text-center text-surface-500">Loading dashboard...</p>
			</SectionCard>
		{:else}
			<SectionCard title="Summary">
				<div class="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
					{#if canViewHosts}
						<a
							href="/hosts"
							class="rounded-[3px] border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-4 hover:border-[var(--accent)]"
						>
							<p class="text-[7.5px] font-semibold uppercase tracking-[0.08em] text-[var(--text-secondary)]">Hosts</p>
							<p class="mt-1 text-[14px] font-bold text-[var(--color-success)]">{totalHosts}</p>
							<p class="mt-1 text-[10px] text-[var(--text-secondary)]">registered hosts</p>
						</a>
					{/if}

					{#if canViewAgents}
						<a
							href="/services"
							class="rounded-[3px] border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-4 hover:border-[var(--accent)]"
						>
							<p class="text-[7.5px] font-semibold uppercase tracking-[0.08em] text-[var(--text-secondary)]">
								Services
							</p>
							<p class="mt-1 text-[14px] font-bold text-[var(--text-muted)]">{totalServices}</p>
							<p class="mt-1 text-[10px] text-[var(--text-secondary)]">
								{#if pendingServices > 0}
									{pendingServices} pending approval
								{:else}
									No pending approvals
								{/if}
							</p>
						</a>
					{/if}

					{#if canViewSoftware}
						<a
							href="/software"
							class="rounded-[3px] border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-4 hover:border-[var(--accent)]"
						>
							<p class="text-[7.5px] font-semibold uppercase tracking-[0.08em] text-[var(--text-secondary)]">
								Updates pending
							</p>
							<p class="mt-1 text-[14px] font-bold text-[var(--color-info)]">{pendingUpdateCount}</p>
							<p class="mt-1 text-[10px] text-[var(--text-secondary)]">
								{totalSoftwareItems + unfeaturedSoftwareCount} tracked software items
							</p>
						</a>

						<a
							href="/history?status=failed"
							class="rounded-[3px] border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-4 hover:border-[var(--accent)]"
						>
							<p class="text-[7.5px] font-semibold uppercase tracking-[0.08em] text-[var(--text-secondary)]">Errors</p>
							<p class="mt-1 text-[14px] font-bold text-[var(--color-error)]">{failedUpdates}</p>
							<p class="mt-1 text-[10px] text-[var(--text-secondary)]">
								{#if failedUpdates > 0}
									{failedUpdates} failed updates in recent activity
								{:else}
									No recent update failures
								{/if}
							</p>
						</a>
					{/if}
				</div>
			</SectionCard>

			{#if hasAttentionItems}
				<SectionCard title="Attention Needed">
					<div class="space-y-3">
						{#if pendingServices > 0 && canViewAgents}
							<Callout tone="warning" title="Services awaiting approval">
								<div class="flex items-center justify-between gap-2">
									<div class="flex items-center gap-2">
										<StatusBadge
											tone="warning"
											label={pendingServices === 1 ? '1 pending service' : `${pendingServices} pending services`}
										/>
									</div>
									<a class="btn btn-sm preset-tonal" href="/services?status=pending">Review</a>
								</div>
							</Callout>
						{/if}

						{#if failedUpdates > 0 && canViewSoftware}
							<Callout tone="danger" title="Recent update failures">
								<div class="flex items-center justify-between gap-2">
									<StatusBadge
										tone="danger"
										label={failedUpdates === 1 ? '1 failed update' : `${failedUpdates} failed updates`}
									/>
									<a class="btn btn-sm preset-tonal" href="/history?status=failed">Investigate</a>
								</div>
							</Callout>
						{/if}
					</div>
				</SectionCard>
			{/if}

			{#if canViewSoftware}
				<SectionCard title="Recent Updates">
					{#snippet actions()}
						{#if totalRecentUpdates > 5}
							<a href="/history" class="btn btn-sm preset-tonal">View all</a>
						{/if}
					{/snippet}

					<DataTable
						columns={[]}
						rows={recentUpdates as unknown as Record<string, unknown>[]}
						emptyTitle="No updates yet"
						emptyDescription="Updates will appear here once triggered."
						rowKey={(row) => (row as unknown as UpdateHistoryResponse).id}
					>
						{#snippet header()}
							<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
								<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">
									Software
								</th>
								<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">Host</th>
								<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">Status</th
								>
								<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">Date</th>
							</tr>
						{/snippet}
						{#snippet row(rowValue)}
							{@const entry = rowValue as unknown as UpdateHistoryResponse}
							<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
								<td class="px-4 py-3 text-sm font-medium text-[var(--text-primary)]">{entry.software_item_name}</td>
								<td class="px-4 py-3 text-sm text-[var(--text-secondary)]">{entry.host_name}</td>
								<td class="px-4 py-3 text-[var(--text-primary)]">
									<StatusBadge tone={statusBadgeTone(entry.status)} label={entry.status} />
								</td>
								<td class="px-4 py-3 text-sm text-[var(--text-secondary)]">{formatDate(entry.created_at)}</td>
							</tr>
						{/snippet}
					</DataTable>
				</SectionCard>
			{/if}
		{/if}
	</PageShell>
{/if}
