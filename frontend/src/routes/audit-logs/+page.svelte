<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import { listAuditLogs, listSystemAuditLogs } from '$lib/api';
	import { formatDate, parseUrlPage } from '$lib/utils';
	import { Permission } from '$lib/types';
	import type { AuditLogEntry } from '$lib/types';
	import { Callout, DataTable, PageShell, SectionCard, StatusBadge, TableFooterBar } from '$lib/components/ui';

	type TabKey = 'tenant' | 'system';

	const user = $derived(getUser());

	const canViewTenant = $derived(user?.permissions.includes(Permission.ViewAuditLogs) ?? false);
	const canViewSystem = $derived(user?.permissions.includes(Permission.ViewSystemAuditLogs) ?? false);
	const hasBoth = $derived(canViewTenant && canViewSystem);

	// Active tab: prefer URL param; default to whichever tab is available
	function resolveTab(url: URL): TabKey {
		const raw = url.searchParams.get('tab');
		if (raw === 'system') return 'system';
		if (raw === 'tenant') return 'tenant';
		// If only system is available, default to system
		return 'tenant';
	}

	let activeTab: TabKey = $state(resolveTab(page.url));
	let currentPage: number = $state(parseUrlPage(page.url));

	// Filters
	let filterActorType: string = $state(page.url.searchParams.get('actor_type') ?? '');
	let filterMethod: string = $state(page.url.searchParams.get('method') ?? '');
	let filterStatus: string = $state(page.url.searchParams.get('status') ?? '');
	let filterFrom: string = $state(page.url.searchParams.get('from') ?? '');
	let filterTo: string = $state(page.url.searchParams.get('to') ?? '');

	let items: AuditLogEntry[] = $state([]);
	let totalPages: number = $state(1);
	let totalItems: number = $state(0);
	let loading: boolean = $state(false);
	let error: string | null = $state(null);

	// Sync URL with filter state
	$effect(() => {
		const parts: string[] = [];
		if (hasBoth) parts.push(`tab=${encodeURIComponent(activeTab)}`);
		if (filterActorType) parts.push(`actor_type=${encodeURIComponent(filterActorType)}`);
		if (filterMethod) parts.push(`method=${encodeURIComponent(filterMethod)}`);
		if (filterStatus) parts.push(`status=${encodeURIComponent(filterStatus)}`);
		if (filterFrom) parts.push(`from=${encodeURIComponent(filterFrom)}`);
		if (filterTo) parts.push(`to=${encodeURIComponent(filterTo)}`);
		if (currentPage > 1) parts.push(`page=${currentPage}`);
		const qs = parts.join('&');
		goto(qs ? `${location.pathname}?${qs}` : location.pathname, {
			replaceState: true,
			keepFocus: true,
			noScroll: true
		});
	});

	onMount(() => {
		if (!canViewTenant && canViewSystem) {
			activeTab = 'system';
		}
		if (canViewTenant || canViewSystem) {
			load(currentPage);
		}
	});

	async function load(p: number) {
		loading = true;
		error = null;
		const params = {
			page: p,
			actor_type: filterActorType || undefined,
			method: filterMethod || undefined,
			status: filterStatus ? Number(filterStatus) : undefined,
			from: filterFrom || undefined,
			to: filterTo || undefined
		};
		try {
			const fn = activeTab === 'system' ? listSystemAuditLogs : listAuditLogs;
			const res = await fn(params);
			items = res.items;
			currentPage = res.page;
			totalPages = res.total_pages;
			totalItems = res.total;
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load audit logs';
		} finally {
			loading = false;
		}
	}

	function applyFilters() {
		currentPage = 1;
		load(1);
	}

	function clearFilters() {
		filterActorType = '';
		filterMethod = '';
		filterStatus = '';
		filterFrom = '';
		filterTo = '';
		currentPage = 1;
		load(1);
	}

	function switchTab(tab: TabKey) {
		activeTab = tab;
		currentPage = 1;
		// Reset filter fields so old query parameters don't carry over
		filterActorType = '';
		filterMethod = '';
		filterStatus = '';
		filterFrom = '';
		filterTo = '';
		load(1);
	}

	function statusBadgeTone(status: number): 'danger' | 'warning' | 'success' | 'neutral' {
		if (status >= 500) return 'danger';
		if (status >= 400) return 'warning';
		if (status >= 200 && status < 300) return 'success';
		return 'neutral';
	}

	const HTTP_METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE'];
	const ACTOR_TYPES = ['user', 'api_token', 'oidc'];
</script>

{#if user}
	<PageShell
		title="Audit Logs"
		description="Search request and actor-level audit trails for tenant and system actions."
	>
		{#if !canViewTenant && !canViewSystem}
			<Callout tone="danger" message="You do not have permission to view audit logs." />
		{:else}
			{#if hasBoth}
				<SectionCard title="Log Scope">
					<div class="flex gap-1 border-b border-surface-200 dark:border-surface-700">
						<button
							class="border-b-2 px-4 py-2 text-sm font-medium {activeTab === 'tenant'
								? 'border-primary-500 text-primary-600 dark:text-primary-400'
								: 'border-transparent hover:text-primary-500'}"
							onclick={() => switchTab('tenant')}
						>
							Tenant Logs
						</button>
						<button
							class="border-b-2 px-4 py-2 text-sm font-medium {activeTab === 'system'
								? 'border-primary-500 text-primary-600 dark:text-primary-400'
								: 'border-transparent hover:text-primary-500'}"
							onclick={() => switchTab('system')}
						>
							System Logs
						</button>
					</div>
				</SectionCard>
			{:else if canViewSystem}
				<SectionCard>
					<p class="text-sm text-surface-500">Showing system-level audit logs.</p>
				</SectionCard>
			{/if}

			<SectionCard title="Filters">
				<div class="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
					<label class="label">
						<span class="text-xs font-medium">Actor Type</span>
						<select class="select" bind:value={filterActorType}>
							<option value="">All</option>
							{#each ACTOR_TYPES as t (t)}
								<option value={t}>{t}</option>
							{/each}
						</select>
					</label>

					<label class="label">
						<span class="text-xs font-medium">HTTP Method</span>
						<select class="select" bind:value={filterMethod}>
							<option value="">All</option>
							{#each HTTP_METHODS as m (m)}
								<option value={m}>{m}</option>
							{/each}
						</select>
					</label>

					<label class="label">
						<span class="text-xs font-medium">Status Code</span>
						<input class="input" type="number" min="100" max="599" placeholder="e.g. 200" bind:value={filterStatus} />
					</label>

					<label class="label">
						<span class="text-xs font-medium">From (RFC 3339)</span>
						<input class="input" type="datetime-local" bind:value={filterFrom} />
					</label>

					<label class="label">
						<span class="text-xs font-medium">To (RFC 3339)</span>
						<input class="input" type="datetime-local" bind:value={filterTo} />
					</label>

					<div class="flex items-end gap-2">
						<button class="btn preset-filled-primary-500 flex-1" onclick={applyFilters}>Apply</button>
						<button class="btn preset-tonal-surface" onclick={clearFilters}>Clear</button>
					</div>
				</div>
			</SectionCard>

			<SectionCard title={activeTab === 'system' ? 'System Audit Entries' : 'Tenant Audit Entries'}>
				<DataTable
					columns={[]}
					rows={items as unknown as Record<string, unknown>[]}
					{loading}
					{error}
					emptyTitle="No audit log entries"
					emptyDescription="No entries match the current filters."
					rowKey={(row) => (row as unknown as AuditLogEntry).id}
				>
					{#snippet header()}
						<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
							<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">
								Occurred At
							</th>
							<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">Method</th>
							<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">Path</th>
							<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">Status</th>
							<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">
								Actor Type
							</th>
							<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">Auth</th>
							<th class="px-4 py-3 text-right text-xs font-semibold uppercase tracking-[0.14em]" scope="col">
								Duration (ms)
							</th>
							<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">IP</th>
						</tr>
					{/snippet}
					{#snippet row(rowValue)}
						{@const entry = rowValue as unknown as AuditLogEntry}
						<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
							<td class="px-4 py-3 whitespace-nowrap text-xs text-[var(--text-primary)]"
								>{formatDate(entry.occurred_at)}</td
							>
							<td class="px-4 py-3 font-mono text-xs text-[var(--text-primary)]">{entry.http_method}</td>
							<td
								class="px-4 py-3 max-w-xs truncate font-mono text-xs text-[var(--text-primary)]"
								title={entry.http_path}
							>
								{entry.http_path}
							</td>
							<td class="px-4 py-3">
								<StatusBadge tone={statusBadgeTone(entry.http_status)} label={`${entry.http_status}`} />
							</td>
							<td class="px-4 py-3 text-xs text-[var(--text-primary)]">{entry.actor_type}</td>
							<td class="px-4 py-3 text-xs text-[var(--text-primary)]">{entry.auth_method}</td>
							<td class="px-4 py-3 text-right text-xs text-[var(--text-primary)]">{entry.duration_ms}</td>
							<td class="px-4 py-3 text-xs text-[var(--text-primary)]">{entry.client_ip ?? '—'}</td>
						</tr>
					{/snippet}
					{#snippet errorActions()}
						<button class="btn preset-filled-primary-500 mt-3" onclick={() => load(currentPage)}>Retry</button>
					{/snippet}
					{#snippet footer()}
						{#if !error}
							<TableFooterBar {currentPage} {totalPages} total={totalItems} onPageChange={load} />
						{/if}
					{/snippet}
				</DataTable>
			</SectionCard>
		{/if}
	</PageShell>
{/if}
