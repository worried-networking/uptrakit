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
	import Modal from '$lib/components/Modal.svelte';
	import { Input, Select } from '$lib/components/forms';
	import { showSuccess } from '$lib/notifications.svelte';
	import StateTab from './StateTab.svelte';
	import {
		Callout,
		DataTable,
		PageShell,
		PillBadge,
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
	let filterCorrelationId: string = $state(page.url.searchParams.get('correlation_id') ?? '');
	let filterActionKind: string = $state(page.url.searchParams.get('action_kind') ?? '');
	let selected: AuditLogEntry | null = $state(null);
	let detailTab: string = $state('details');

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
		if (filterCorrelationId) parts.push(`correlation_id=${encodeURIComponent(filterCorrelationId)}`);
		if (filterActionKind) parts.push(`action_kind=${encodeURIComponent(filterActionKind)}`);
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
			to: toRfc3339(filterTo),
			correlation_id: filterCorrelationId || undefined,
			action_kind: (filterActionKind as 'stateful' | 'event') || undefined
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
		filterCorrelationId = '';
		filterActionKind = '';
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
		filterCorrelationId = '';
		filterActionKind = '';
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

	const ACTOR_TYPES = ['user', 'api_token', 'oidc', 'service', 'system'];
	const OUTCOME_TYPES = ['success', 'denied', 'validation_failed', 'failed', 'partial'];
	const SCOPE_TAB_ITEMS: TabStripItem[] = [
		{ id: 'tenant', label: 'Tenant Logs' },
		{ id: 'system', label: 'System Logs' }
	];

	function detailTabs(entry: AuditLogEntry): TabStripItem[] {
		return [
			{ id: 'details', label: 'Details' },
			...(entry.action_kind === 'stateful' ? [{ id: 'state', label: 'State' }] : []),
			{ id: 'raw', label: 'Raw' }
		];
	}
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
				<TabStrip
					items={SCOPE_TAB_ITEMS}
					activeId={activeTab}
					ariaLabel="Audit log scope"
					idBase="audit-logs"
					onSelect={(tab) => switchTab(tab as TabKey)}
				/>
			{/if}

			<SectionCard title="Filters" description="Refine entries by actor, action, target, outcome, and timestamp range.">
				{#snippet actions()}
					<Button variant="primary" onclick={applyFilters}>Apply Filters</Button>
					<Button variant="secondary" onclick={clearFilters}>Clear Filters</Button>
				{/snippet}

				<div class="grid grid-cols-1 gap-x-4 gap-y-3 sm:grid-cols-2 lg:grid-cols-4">
					<div>
						<label for="filter-action-type" class="mb-1 block text-xs font-medium text-[var(--text-secondary)]"
							>Action</label
						>
						<Input id="filter-action-type" type="text" placeholder="e.g. login" bind:value={filterActionType} />
					</div>

					<div>
						<label for="filter-outcome" class="mb-1 block text-xs font-medium text-[var(--text-secondary)]"
							>Outcome</label
						>
						<Select
							id="filter-outcome"
							bind:value={filterOutcome}
							options={[
								{ value: '', label: 'All' },
								...OUTCOME_TYPES.map((o) => ({ value: o, label: outcomeLabel(o) }))
							]}
						/>
					</div>

					<div>
						<label for="filter-actor-type" class="mb-1 block text-xs font-medium text-[var(--text-secondary)]"
							>Actor Type</label
						>
						<Select
							id="filter-actor-type"
							bind:value={filterActorType}
							options={[{ value: '', label: 'All' }, ...ACTOR_TYPES.map((t) => ({ value: t, label: t }))]}
						/>
					</div>

					<div>
						<label for="filter-target-type" class="mb-1 block text-xs font-medium text-[var(--text-secondary)]"
							>Target Type</label
						>
						<Input id="filter-target-type" type="text" placeholder="e.g. software_item" bind:value={filterTargetType} />
					</div>

					<div>
						<label for="filter-target-id" class="mb-1 block text-xs font-medium text-[var(--text-secondary)]"
							>Target ID</label
						>
						<Input id="filter-target-id" type="text" placeholder="Specific target id" bind:value={filterTargetId} />
					</div>

					<div>
						<label for="filter-from" class="mb-1 block text-xs font-medium text-[var(--text-secondary)]">From</label>
						<Input id="filter-from" type="datetime-local" bind:value={filterFrom} />
					</div>

					<div>
						<label for="filter-to" class="mb-1 block text-xs font-medium text-[var(--text-secondary)]">To</label>
						<Input id="filter-to" type="datetime-local" bind:value={filterTo} />
					</div>

					<div>
						<label for="filter-correlation-id" class="mb-1 block text-xs font-medium text-[var(--text-secondary)]"
							>Correlation ID</label
						>
						<Input
							id="filter-correlation-id"
							type="text"
							placeholder="00000000-0000-0000-0000-000000000000"
							bind:value={filterCorrelationId}
						/>
					</div>

					<div>
						<label for="filter-action-kind" class="mb-1 block text-xs font-medium text-[var(--text-secondary)]"
							>Action Kind</label
						>
						<Select
							id="filter-action-kind"
							bind:value={filterActionKind}
							options={[
								{ value: '', label: 'All' },
								{ value: 'stateful', label: 'stateful' },
								{ value: 'event', label: 'event' }
							]}
						/>
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
							<th
								class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								scope="col"
							>
								Occurred At
							</th>
							<th
								class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								scope="col">Action</th
							>
							<th
								class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								scope="col">Target</th
							>
							<th
								class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								scope="col">Outcome</th
							>
							<th
								class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								scope="col">Actor</th
							>
						</tr>
					{/snippet}
					{#snippet row(rowValue, _index)}
						{@const entry = rowValue as unknown as AuditLogEntry}
						<tr
							class="cursor-pointer border-b border-[var(--border-subtle)] last:border-b-0 hover:bg-[var(--bg-hover)]"
							onclick={() => {
								selected = entry;
								detailTab = 'details';
							}}
						>
							<td class="table-cell-pad whitespace-nowrap text-table-body text-[var(--text-primary)]"
								>{formatDate(entry.occurred_at)}</td
							>
							<td class="table-cell-pad font-mono text-table-body text-[var(--text-primary)]">{entry.action_type}</td>
							<td
								class="table-cell-pad max-w-xs truncate text-table-body text-[var(--text-primary)]"
								title={targetLabel(entry)}
							>
								{targetLabel(entry)}
							</td>
							<td class="table-cell-pad">
								<StatusBadge tone={outcomeTone(entry.outcome)} label={outcomeLabel(entry.outcome)} />
							</td>
							<td class="table-cell-pad" title={entry.actor_display ?? entry.actor_id ?? entry.actor_type}>
								<div class="flex items-center gap-2">
									<PillBadge label={entry.actor_type} />
									{#if entry.actor_display ?? entry.actor_id}
										<span class="text-table-body text-[var(--text-primary)]"
											>{entry.actor_display ?? entry.actor_id}</span
										>
									{/if}
								</div>
							</td>
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

			{#if selected}
				<Modal
					onclose={() => {
						selected = null;
					}}
					title={selected.action_type}
					maxWidth="max-w-3xl"
				>
					<div class="flex flex-col gap-4">
						<TabStrip
							items={detailTabs(selected)}
							activeId={detailTab}
							ariaLabel="Audit log detail"
							idBase="detail"
							onSelect={(id) => (detailTab = id)}
						/>

						{#if detailTab === 'state'}
							<StateTab before={selected.before_snapshot} after={selected.after_snapshot} />
						{:else if detailTab === 'raw'}
							<pre
								class="overflow-auto rounded-card bg-[var(--bg-raised)] p-4 text-xs text-[var(--text-primary)]">{JSON.stringify(
									selected,
									null,
									2
								)}</pre>
						{:else}
							<div class="flex flex-col gap-2">
								{#each [['Action', selected.action_type], ['Outcome', selected.outcome], ['Actor', selected.actor_display ?? selected.actor_type], ['Target', selected.target_display ?? selected.target_type ?? '—'], ['Occurred at', selected.occurred_at], ['Correlation ID', selected.correlation_id ?? '—']] as [label, value] (label)}
									<div class="flex gap-2 text-sm">
										<span class="w-32 shrink-0 text-[var(--text-secondary)]">{label}</span>
										<span class="text-[var(--text-primary)]">{value}</span>
										{#if label === 'Correlation ID' && selected.correlation_id}
											<Button
												variant="ghost"
												onclick={() => {
													navigator.clipboard.writeText(selected!.correlation_id!);
													showSuccess('Copied to clipboard');
												}}>Copy</Button
											>
										{/if}
									</div>
								{/each}
								{#if selected.details_json}
									{#if selected.action_type === 'system_config_reload_requested'}
										<div class="mt-2 flex flex-col gap-1 text-sm">
											<div class="flex gap-2">
												<span class="w-32 shrink-0 text-[var(--text-secondary)]">Source</span>
												<span class="font-mono text-[var(--text-primary)]"
													>{String(selected.details_json['source'] ?? '—')}</span
												>
											</div>
										</div>
									{:else if selected.action_type === 'system_config_reload_applied'}
										{@const sectionList = selected.details_json['sections'] as string[] | undefined}
										{@const perSubMs = selected.details_json['per_subsystem_ms'] as Record<string, number> | undefined}
										<div class="mt-2 flex flex-col gap-1 text-sm">
											{#if sectionList?.length}
												<div class="flex gap-2">
													<span class="w-32 shrink-0 text-[var(--text-secondary)]">Sections</span>
													<span class="text-[var(--text-primary)]">{sectionList.join(', ')}</span>
												</div>
											{/if}
											{#if perSubMs && Object.keys(perSubMs).length > 0}
												<details class="mt-1">
													<summary class="cursor-pointer text-xs text-[var(--text-secondary)]"
														>Per-subsystem timing</summary
													>
													<dl class="mt-1 grid grid-cols-2 gap-x-4 gap-y-1 text-xs">
														{#each Object.entries(perSubMs) as [name, ms] (name)}
															<dt class="text-[var(--text-secondary)]">{name}</dt>
															<dd class="font-mono text-[var(--text-primary)]">{ms} ms</dd>
														{/each}
													</dl>
												</details>
											{/if}
										</div>
									{:else if selected.action_type === 'system_config_reload_failed'}
										<div class="mt-2 flex flex-col gap-1 text-sm">
											<div class="flex items-center gap-2">
												<span class="w-32 shrink-0 text-[var(--text-secondary)]">Severity</span>
												<StatusBadge tone="danger" label="Error" />
											</div>
											<div class="flex gap-2">
												<span class="w-32 shrink-0 text-[var(--text-secondary)]">Phase</span>
												<span class="font-mono text-[var(--text-primary)]"
													>{String(selected.details_json['phase'] ?? '—')}</span
												>
											</div>
											{#if selected.details_json['subsystem']}
												<div class="flex gap-2">
													<span class="w-32 shrink-0 text-[var(--text-secondary)]">Subsystem</span>
													<span class="font-mono text-[var(--text-primary)]"
														>{String(selected.details_json['subsystem'])}</span
													>
												</div>
											{/if}
											<div class="flex gap-2">
												<span class="w-32 shrink-0 text-[var(--text-secondary)]">Error</span>
												<span class="text-[var(--text-primary)]">{String(selected.details_json['error'] ?? '—')}</span>
											</div>
										</div>
									{:else if selected.action_type === 'system_config_reload_reverted'}
										<div class="mt-2 flex flex-col gap-1 text-sm">
											<div class="flex items-center gap-2">
												<span class="w-32 shrink-0 text-[var(--text-secondary)]">Severity</span>
												<StatusBadge tone="warning" label="Warning" />
											</div>
											<div class="flex gap-2">
												<span class="w-32 shrink-0 text-[var(--text-secondary)]">Subsystem</span>
												<span class="font-mono text-[var(--text-primary)]"
													>{String(selected.details_json['subsystem'] ?? '—')}</span
												>
											</div>
											<div class="flex gap-2">
												<span class="w-32 shrink-0 text-[var(--text-secondary)]">Reason</span>
												<span class="text-[var(--text-primary)]">{String(selected.details_json['reason'] ?? '—')}</span>
											</div>
										</div>
									{:else}
										<details class="mt-2">
											<summary class="cursor-pointer text-xs text-[var(--text-secondary)]">Event details</summary>
											<pre
												class="mt-2 overflow-auto rounded-card bg-[var(--bg-raised)] p-4 text-xs text-[var(--text-primary)]">{JSON.stringify(
													selected.details_json,
													null,
													2
												)}</pre>
										</details>
									{/if}
								{/if}
							</div>
						{/if}
					</div>
				</Modal>
			{/if}
		{/if}
	</PageShell>
{/if}
