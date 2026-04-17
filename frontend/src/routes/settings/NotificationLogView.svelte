<script lang="ts">
	import { listNotificationLog, listNotificationChannels } from '$lib/api';
	import type { NotificationLogEntry, NotificationChannelSummary, NotificationEventType } from '$lib/types';
	import { Callout, SectionCard, StatusBadge, type StatusBadgeTone } from '$lib/components/ui';

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
</script>

<SectionCard title="Notification Log">
	{#if loading}
		<p class="text-center text-surface-500">Loading log...</p>
	{:else if error}
		<Callout tone="danger" title="Unable to load notification log" message={error}>
			<button class="btn preset-filled-primary-500 mt-2" onclick={() => void loadData()}>Retry</button>
		</Callout>
	{:else if entries.length === 0}
		<p class="text-center text-surface-500">No notification log entries.</p>
	{:else}
		<div class="table-container">
			<table class="table table-hover">
				<thead>
					<tr>
						<th>Event Type</th>
						<th>Channel</th>
						<th>Status</th>
						<th>Created</th>
						<th>Delivered</th>
						<th>Error</th>
					</tr>
				</thead>
				<tbody>
					{#each entries as entry (entry.id)}
						<tr>
							<td>{EVENT_TYPE_LABELS[entry.event_type as NotificationEventType] ?? entry.event_type}</td>
							<td>{channelMap.get(entry.channel_id) ?? entry.channel_id.slice(0, 8)}</td>
							<td>
								<StatusBadge tone={statusTone(entry.status)} label={entry.status} />
							</td>
							<td class="text-sm">{formatDate(entry.created_at)}</td>
							<td class="text-sm">{formatDate(entry.delivered_at)}</td>
							<td class="text-sm text-error-500">{entry.error_message ?? ''}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>

		{#if totalPages > 1}
			<div class="mt-4 flex items-center justify-center gap-2">
				<button
					class="btn btn-sm preset-tonal"
					disabled={currentPage <= 1}
					onclick={() => {
						currentPage--;
						void loadData();
					}}
				>
					Previous
				</button>
				<span class="text-sm">Page {currentPage} of {totalPages}</span>
				<button
					class="btn btn-sm preset-tonal"
					disabled={currentPage >= totalPages}
					onclick={() => {
						currentPage++;
						void loadData();
					}}
				>
					Next
				</button>
			</div>
		{/if}
	{/if}
</SectionCard>
