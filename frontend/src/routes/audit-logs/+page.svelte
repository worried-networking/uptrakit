<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import { listAuditLogs, listSystemAuditLogs } from '$lib/api';
	import { formatDate, parseUrlPage } from '$lib/utils';
	import { Permission } from '$lib/types';
	import type { AuditLogEntry } from '$lib/types';
	import Button from '$lib/components/Button.svelte';
	import {
		Callout,
		DataTable,
		PageShell,
		SectionCard,
		StatusBadge,
		TableFooterBar,
		TabStrip,
		type TabStripItem
	} from '$lib/components/ui';

	type TabKey = 'tenant' | 'system';

	const user = $derived(getUser());

	const canViewTenant = $derived(user?.permissions.includes(Permission.ViewAuditLogs) ?? false);
	const canViewSystem = $derived(user?.permissions.includes(Permission.ViewSystemAuditLogs) ?? false);
	const hasBoth = $derived(canViewTenant && canViewSystem);

	// Active tab: prefer URL param; default to whichever tab is available
	function resolveTab(url: URL, tenantAllowed: boolean, systemAllowed: boolean): TabKey {
		const raw = url.searchParams.get('tab');
		if (raw === 'system' && systemAllowed) return 'system';
		if (raw === 'tenant' && tenantAllowed) return 'tenant';
		if (tenantAllowed) return 'tenant';
		if (systemAllowed) return 'system';
		return 'tenant';
	}

	function fromRfc3339ToLocalInput(value: string): string {
		if (!value) return '';
		const date = new Date(value);
		if (Number.isNaN(date.getTime())) return '';
		const pad = (part: number) => part.toString().padStart(2, '0');
		return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
	}

	function toRfc3339(value: string): string | undefined {
		if (!value) return undefined;
		const date = new Date(value);
		if (Number.isNaN(date.getTime())) return undefined;
		return date.toISOString();
	}

	let activeTab: TabKey = $state('tenant');
	let currentPage: number = $state(parseUrlPage(page.url));
	let tabInitialized = $state(false);

	// Filters
	let filterActorType: string = $state(page.url.searchParams.get('actor_type') ?? '');
	let filterActionType: string = $state(page.url.searchParams.get('action_type') ?? '');
	let filterOutcome: string = $state(page.url.searchParams.get('outcome') ?? '');
	let filterTargetType: string = $state(page.url.searchParams.get('target_type') ?? '');
	let filterTargetId: string = $state(page.url.searchParams.get('target_id') ?? '');
	let filterFrom: string = $state(fromRfc3339ToLocalInput(page.url.searchParams.get('from') ?? ''));
	let filterTo: string = $state(fromRfc3339ToLocalInput(page.url.searchParams.get('to') ?? ''));

	let items: AuditLogEntry[] = $state([]);
	let totalPages: number = $state(1);
	let totalItems: number = $state(0);
	let loading: boolean = $state(false);
	let error: string | null = $state(null);
	let isRetrying: boolean = $state(false);

	// Sync URL with filter state
	$effect(() => {
		if (!tabInitialized) return;
		const from = toRfc3339(filterFrom);
		const to = toRfc3339(filterTo);
		const parts: string[] = [];
		if (hasBoth) parts.push(`tab=${encodeURIComponent(activeTab)}`);
		if (filterActorType) parts.push(`actor_type=${encodeURIComponent(filterActorType)}`);
		if (filterActionType) parts.push(`action_type=${encodeURIComponent(filterActionType)}`);
		if (filterOutcome) parts.push(`outcome=${encodeURIComponent(filterOutcome)}`);
		if (filterTargetType) parts.push(`target_type=${encodeURIComponent(filterTargetType)}`);
		if (filterTargetId) parts.push(`target_id=${encodeURIComponent(filterTargetId)}`);
		if (from) parts.push(`from=${encodeURIComponent(from)}`);
		if (to) parts.push(`to=${encodeURIComponent(to)}`);
		if (currentPage > 1) parts.push(`page=${currentPage}`);
		const qs = parts.join('&');
		goto(qs ? `${location.pathname}?${qs}` : location.pathname, {
			replaceState: true,
			keepFocus: true,
			noScroll: true
		});
	});

	onMount(() => {
		activeTab = resolveTab(page.url, canViewTenant, canViewSystem);
		tabInitialized = true;
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
			action_type: filterActionType || undefined,
			outcome: filterOutcome || undefined,
			target_type: filterTargetType || undefined,
			target_id: filterTargetId || undefined,
			from: toRfc3339(filterFrom),
			to: toRfc3339(filterTo)
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
		filterActionType = '';
		filterOutcome = '';
		filterTargetType = '';
		filterTargetId = '';
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
		filterActionType = '';
		filterOutcome = '';
		filterTargetType = '';
		filterTargetId = '';
		filterFrom = '';
		filterTo = '';
		load(1);
	}

	function outcomeTone(outcome: string): 'neutral' | 'info' | 'success' | 'warning' | 'danger' {
		if (outcome === 'success') return 'success';
		if (outcome === 'failed') return 'danger';
		if (outcome === 'denied' || outcome === 'validation_failed') return 'warning';
		if (outcome === 'partial') return 'info';
		return 'neutral';
	}

	function outcomeLabel(outcome: string): string {
		return outcome.replaceAll('_', ' ');
	}

	function targetLabel(entry: AuditLogEntry): string {
		if (entry.target_display) return entry.target_display;
		if (entry.target_type && entry.target_id) return `${entry.target_type}:${entry.target_id}`;
		return entry.target_type ?? entry.target_id ?? '—';
	}

	function actorLabel(entry: AuditLogEntry): string {
		if (entry.actor_display) return entry.actor_display;
		if (entry.actor_id) return `${entry.actor_type}:${entry.actor_id}`;
		return entry.actor_type;
	}

	const ACTOR_TYPES = ['user', 'api_token', 'oidc', 'service', 'system'];
	const OUTCOME_TYPES = ['success', 'denied', 'validation_failed', 'failed', 'partial'];
	const SCOPE_TAB_ITEMS: TabStripItem[] = [
		{ id: 'tenant', label: 'Tenant Logs' },
		{ id: 'system', label: 'System Logs' }
	];
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
				<SectionCard title="Log Scope" description="Switch between tenant and system audit streams.">
					<TabStrip
						items={SCOPE_TAB_ITEMS}
						activeId={activeTab}
						ariaLabel="Audit log scope"
						idBase="audit-logs"
						onSelect={(tab) => switchTab(tab as TabKey)}
					/>
				</SectionCard>
			{:else if canViewSystem}
				<SectionCard>
					<p class="text-sm text-[var(--text-muted)]">Showing system-level audit logs.</p>
				</SectionCard>
			{/if}

			<SectionCard title="Filters" description="Refine entries by actor, action, target, outcome, and timestamp range.">
				{#snippet actions()}
					<Button variant="primary" onclick={applyFilters}>Apply Filters</Button>
					<Button variant="secondary" onclick={clearFilters}>Clear Filters</Button>
				{/snippet}

				<div class="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
					<label class="label">
						<span class="text-xs font-medium">Action</span>
						<input class="input" type="text" placeholder="e.g. login" bind:value={filterActionType} />
					</label>

					<label class="label">
						<span class="text-xs font-medium">Outcome</span>
						<select class="select" bind:value={filterOutcome}>
							<option value="">All</option>
							{#each OUTCOME_TYPES as outcome (outcome)}
								<option value={outcome}>{outcomeLabel(outcome)}</option>
							{/each}
						</select>
					</label>

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
						<span class="text-xs font-medium">Target Type</span>
						<input class="input" type="text" placeholder="e.g. software_item" bind:value={filterTargetType} />
					</label>

					<label class="label">
						<span class="text-xs font-medium">Target ID</span>
						<input class="input" type="text" placeholder="Specific target id" bind:value={filterTargetId} />
					</label>

					<label class="label">
						<span class="text-xs font-medium">From (RFC 3339)</span>
						<input class="input" type="datetime-local" bind:value={filterFrom} />
					</label>

					<label class="label">
						<span class="text-xs font-medium">To (RFC 3339)</span>
						<input class="input" type="datetime-local" bind:value={filterTo} />
					</label>
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
							<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">Action</th>
							<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">Target</th>
							<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">Outcome</th>
							<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">Actor</th>
						</tr>
					{/snippet}
					{#snippet row(rowValue)}
						{@const entry = rowValue as unknown as AuditLogEntry}
						<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
							<td class="px-4 py-3 whitespace-nowrap text-xs text-[var(--text-primary)]"
								>{formatDate(entry.occurred_at)}</td
							>
							<td class="px-4 py-3 font-mono text-xs text-[var(--text-primary)]">{entry.action_type}</td>
							<td class="px-4 py-3 max-w-xs truncate text-xs text-[var(--text-primary)]" title={targetLabel(entry)}>
								{targetLabel(entry)}
							</td>
							<td class="px-4 py-3">
								<StatusBadge tone={outcomeTone(entry.outcome)} label={outcomeLabel(entry.outcome)} />
							</td>
							<td class="px-4 py-3 text-xs text-[var(--text-primary)]" title={actorLabel(entry)}>{actorLabel(entry)}</td
							>
						</tr>
					{/snippet}
					{#snippet errorActions()}
						<Button
							variant="primary"
							loading={isRetrying}
							onclick={async () => {
								isRetrying = true;
								try {
									await load(currentPage);
								} finally {
									isRetrying = false;
								}
							}}
							class="mt-3"
						>
							Retry
						</Button>
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
