<script lang="ts">
	import { listNotificationLog, listNotificationChannels } from '$lib/api';
	import type { NotificationLogEntry, NotificationChannelSummary, NotificationEventType } from '$lib/types';
	import {
		DataTable,
		SectionCard,
		StatusBadge,
		TableFooterBar,
		type DataTableColumn,
		type StatusBadgeTone
	} from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';

	const EVENT_TYPE_LABELS: Record<NotificationEventType, string> = {
		update_available: 'Update Available',
		update_completed: 'Update Completed',
		update_failed: 'Update Failed',
		new_software_discovered: 'New Software Discovered',
		new_service_enrolled: 'New Service Enrolled',
		ca_rotated: 'CA Rotated',
		batch_update_completed: 'Batch Update Completed',
		batch_update_partially_completed: 'Batch Update Partially Completed',
		stdin_attention: 'Input Attention Required'
	};

	let entries: NotificationLogEntry[] = $state([]);
	let channels: NotificationChannelSummary[] = $state([]);
	let channelMap: Map<string, string> = $derived(new Map(channels.map((c) => [c.id, c.name])));
	let loading: boolean = $state(true);
	let error: string | null = $state(null);
	let currentPage: number = $state(1);
	let totalPages: number = $state(1);
	let totalItems: number = $state(0);
	let isRetrying: boolean = $state(false);

	$effect(() => {
		void loadData();
	});

	async function loadData() {
		loading = true;
		error = null;
		try {
			const [logRes, channelsRes] = await Promise.all([
				listNotificationLog(currentPage),
				listNotificationChannels(1, 1000)
			]);
			entries = logRes.items;
			totalPages = logRes.total_pages;
			totalItems = logRes.total;
			channels = channelsRes.items;
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load notification log';
		} finally {
			loading = false;
		}
	}

	function formatDate(iso: string | null): string {
		if (!iso) return '—';
		return new Date(iso).toLocaleString();
	}

	function statusTone(status: string): StatusBadgeTone {
		switch (status) {
			case 'delivered':
				return 'success';
			case 'failed':
				return 'danger';
			case 'pending':
				return 'warning';
			default:
				return 'neutral';
		}
	}

	function handlePageChange(page: number) {
		if (page === currentPage) return;
		currentPage = page;
	}

	const logColumns: DataTableColumn[] = [
		{ key: 'event_type', label: 'Event Type' },
		{ key: 'channel', label: 'Channel' },
		{ key: 'status', label: 'Status' },
		{ key: 'created_at', label: 'Created' },
		{ key: 'delivered_at', label: 'Delivered' },
		{ key: 'error_message', label: 'Error' }
	];

	const loadingSkeletonRows = [0, 1, 2, 3, 4];
</script>

<SectionCard title="Notification Log">
	{#if loading}
		<div
			class="overflow-hidden rounded-panel border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm"
			data-ui="known-shape-table-loading"
			aria-busy="true"
		>
			<div class="overflow-x-auto">
				<table class="min-w-full border-collapse text-table-body">
					<caption class="sr-only">Loading notification log entries</caption>
					<thead>
						<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
							{#each logColumns as column (column.key)}
								<th
									class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
									scope="col"
								>
									{column.label}
								</th>
							{/each}
						</tr>
					</thead>
					<tbody>
						{#each loadingSkeletonRows as rowIndex (rowIndex)}
							<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
								<td class="table-cell-pad"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-24 animate-pulse rounded-card bg-[var(--bg-raised)]"
									></div></td
								>
								<td class="table-cell-pad"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-20 animate-pulse rounded-card bg-[var(--bg-raised)]"
									></div></td
								>
								<td class="table-cell-pad"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-16 animate-pulse rounded-card bg-[var(--bg-raised)]"
									></div></td
								>
								<td class="table-cell-pad"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-28 animate-pulse rounded-card bg-[var(--bg-raised)]"
									></div></td
								>
								<td class="table-cell-pad"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-28 animate-pulse rounded-card bg-[var(--bg-raised)]"
									></div></td
								>
								<td class="table-cell-pad"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-20 animate-pulse rounded-card bg-[var(--bg-raised)]"
									></div></td
								>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		</div>
	{:else}
		<DataTable
			columns={logColumns}
			rows={entries as unknown as Record<string, unknown>[]}
			loading={false}
			{error}
			emptyTitle="No notification log entries."
			emptyDescription="Notifications will appear here once delivery attempts occur."
			rowKey={(row) => (row as unknown as NotificationLogEntry).id}
		>
			{#snippet row(rowValue, _index)}
				{@const entry = rowValue as unknown as NotificationLogEntry}
				<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
					<td class="table-cell-pad"
						>{EVENT_TYPE_LABELS[entry.event_type as NotificationEventType] ?? entry.event_type}</td
					>
					<td class="table-cell-pad">{channelMap.get(entry.channel_id) ?? entry.channel_id.slice(0, 8)}</td>
					<td class="table-cell-pad">
						<StatusBadge tone={statusTone(entry.status)} label={entry.status} />
					</td>
					<td class="table-cell-pad text-sm">{formatDate(entry.created_at)}</td>
					<td class="table-cell-pad text-sm">{formatDate(entry.delivered_at)}</td>
					<td class="table-cell-pad text-sm text-[var(--color-danger)]">{entry.error_message ?? ''}</td>
				</tr>
			{/snippet}
			{#snippet errorActions()}
				<Button
					variant="primary"
					loading={isRetrying}
					onclick={async () => {
						isRetrying = true;
						try {
							await loadData();
						} finally {
							isRetrying = false;
						}
					}}>Retry</Button
				>
			{/snippet}
			{#snippet footer()}
				{#if !error && totalPages > 1}
					<TableFooterBar total={totalItems} {currentPage} {totalPages} onPageChange={handlePageChange} />
				{/if}
			{/snippet}
		</DataTable>
	{/if}
</SectionCard>
