<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { SvelteMap } from 'svelte/reactivity';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import {
		listUpdateHistory,
		triggerSoftwareUpdate,
		getSoftwareItems,
		getUpdateHistoryEntry,
		getSoftwareItem
	} from '$lib/api';
	import { formatDate, formatVersion, parseUrlParam, parseUrlPage } from '$lib/utils';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import TerminalOutput from '$lib/components/TerminalOutput.svelte';
	import { Input, Select } from '$lib/components/forms';
	import { connectInteractiveSession } from '$lib/interactive';
	import type { InteractiveConnectionState } from '$lib/interactive';
	import { connectEventStream } from '$lib/sse';
	import { Permission } from '$lib/types';
	import type { UpdateHistoryResponse, UpdateHistoryStatus, SoftwareItemResponse } from '$lib/types';
	import Button from '$lib/components/Button.svelte';
	import {
		Callout,
		EmptyState,
		PageShell,
		SectionCard,
		StatusBadge,
		TableFooterBar,
		ModalShell
	} from '$lib/components/ui';
	import { FormFieldRow } from '$lib/components/forms';

	type StatusFilter = 'all' | UpdateHistoryStatus;
	type HistoryDateGroup = {
		key: string;
		label: string;
		items: UpdateHistoryResponse[];
	};
	const STATUS_FILTER_VALUES = [
		'all',
		'queued',
		'pending',
		'in_progress',
		'completed',
		'failed'
	] as const satisfies readonly StatusFilter[];

	let items: UpdateHistoryResponse[] = $state([]);
	let loading: boolean = $state(false);
	let error: string | null = $state(null);
	let currentPage: number = $state(parseUrlPage(page.url));
	let totalPages: number = $state(1);
	let totalItems: number = $state(0);
	let statusFilter: StatusFilter = $state(parseUrlParam(page.url, 'status', STATUS_FILTER_VALUES, 'all'));
	let expandedId: string | null = $state(null);

	// Interactive WS state for expanded in-progress items
	let activeWsHandle: ReturnType<typeof connectInteractiveSession> | null = null;
	let activeStreamId: string | null = $state(null);
	let wsState: InteractiveConnectionState = $state('disconnected');
	let stdinAttention: boolean = $state(false);
	let terminalRef: TerminalOutput | undefined = $state(undefined);

	// Admin SSE event stream for real-time list updates
	let disconnectEventStream: (() => void) | null = null;

	// Trigger update modal state
	let showTriggerModal: boolean = $state(false);
	let softwareItems: SoftwareItemResponse[] = $state([]);
	let selectedItemHosts: { host_id: string; label: string }[] = $state([]);
	let selectedItemId: string = $state('');
	let selectedHostId: string = $state('');
	let targetVersion: string = $state('');
	let releaseTag: string = $state('');
	let releaseUrl: string = $state('');
	let triggering: boolean = $state(false);

	const canManage = $derived(getUser()?.permissions.includes(Permission.TriggerUpdates) ?? false);
	const canView = $derived(getUser()?.permissions.includes(Permission.ViewSoftware) ?? false);

	const selectedItem = $derived(softwareItems.find((i) => i.id === selectedItemId));
	const groupedHistory = $derived.by<HistoryDateGroup[]>(() => {
		const sorted = [...items].sort((a, b) => timestampValue(b.started_at) - timestampValue(a.started_at));
		const groups = new SvelteMap<string, HistoryDateGroup>();
		for (const item of sorted) {
			const key = historyDayKey(item.started_at);
			const existing = groups.get(key);
			if (existing) {
				existing.items.push(item);
				continue;
			}
			groups.set(key, {
				key,
				label: historyDayLabel(item.started_at),
				items: [item]
			});
		}
		return [...groups.values()];
	});
	const expandedItem = $derived.by<UpdateHistoryResponse | null>(() =>
		expandedId ? (items.find((item) => item.id === expandedId) ?? null) : null
	);

	$effect(() => {
		const parts: string[] = [];
		if (statusFilter !== 'all') parts.push(`status=${statusFilter}`);
		if (currentPage > 1) parts.push(`page=${currentPage}`);
		const search = parts.join('&');
		goto(search ? `${location.pathname}?${search}` : location.pathname, {
			replaceState: true,
			keepFocus: true,
			noScroll: true
		});
	});

	onMount(() => {
		if (canView) {
			loadHistory(currentPage);
			// Subscribe to admin events for real-time list updates.
			disconnectEventStream = connectEventStream({
				onEvent: (eventType, data) => {
					if (eventType === 'update_triggered') {
						// Reload page 1 when a new update is created so the entry
						// appears immediately, but only when the active filter would
						// include it (pending / queued / all).
						if (
							currentPage === 1 &&
							(statusFilter === 'all' || statusFilter === 'pending' || statusFilter === 'queued')
						) {
							loadHistory(1);
						}
					} else if (eventType === 'update_protection_started') {
						const historyId = data.update_history_id as string;
						items = items.map((i) => (i.id === historyId ? { ...i, status: 'in_progress' as const } : i));
						if (
							!items.some((i) => i.id === historyId) &&
							currentPage === 1 &&
							(statusFilter === 'all' || statusFilter === 'in_progress')
						) {
							loadHistory(1);
						}
					} else if (eventType === 'update_started') {
						const historyId = data.update_history_id as string;
						const interactive = data.interactive as boolean;
						const alreadyInList = items.some((i) => i.id === historyId);
						// Update status + interactive flag on the existing item immediately,
						// without a round-trip to the server.
						items = items.map((i) => (i.id === historyId ? { ...i, status: 'in_progress' as const, interactive } : i));
						// If the item is not visible yet, reload page 1 when the active
						// filter would include in-progress items.
						if (!alreadyInList && currentPage === 1 && (statusFilter === 'all' || statusFilter === 'in_progress')) {
							loadHistory(1);
						}
					} else if (eventType === 'update_completed') {
						const historyId = data.update_history_id as string;
						const status = data.status as string;
						if (items.some((i) => i.id === historyId)) {
							// Refresh only the affected row — fetches final output and status.
							reloadItem(historyId);
						} else if (
							currentPage === 1 &&
							(statusFilter === 'all' || statusFilter === 'completed' || statusFilter === status)
						) {
							// Completed items move into view on these filters.
							loadHistory(1);
						}
					}
				}
			});
		}
	});

	async function loadHistory(page: number) {
		loading = true;
		error = null;
		try {
			const res = await listUpdateHistory({
				page,
				status: statusFilter === 'all' ? undefined : statusFilter
			});
			items = res.items;
			currentPage = res.page;
			totalPages = res.total_pages;
			totalItems = res.total;
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load update history';
		} finally {
			loading = false;
		}
	}

	onDestroy(() => {
		disconnectStream();
		disconnectEventStream?.();
	});

	function disconnectStream() {
		if (activeWsHandle) {
			activeWsHandle.disconnect();
			activeWsHandle = null;
		}
		activeStreamId = null;
		wsState = 'disconnected';
		stdinAttention = false;
	}

	function isLiveStatus(status: UpdateHistoryStatus): boolean {
		return status === 'in_progress';
	}

	function isWaitingStatus(status: UpdateHistoryStatus): boolean {
		return status === 'queued' || status === 'pending';
	}

	function connectInteractive(updateHistoryId: string) {
		activeStreamId = updateHistoryId;
		wsState = 'connecting';
		stdinAttention = false;

		activeWsHandle = connectInteractiveSession(updateHistoryId, {
			onOutput: (line) => {
				terminalRef?.write(line.text);
			},
			onCompleted: () => {
				stdinAttention = false;
				reloadItem(updateHistoryId);
			},
			onStdinAttention: () => {
				stdinAttention = true;
			},
			onStateChange: (state) => {
				wsState = state;
			},
			onError: (msg) => {
				showError(`Interactive session error: ${msg}`);
			}
		});
	}

	async function reloadItem(id: string) {
		try {
			const updated = await getUpdateHistoryEntry(id);
			items = items.map((i) => (i.id === id ? updated : i));
		} catch {
			// Fallback: reload the whole page
			loadHistory(currentPage);
		}
	}

	function statusBadgeTone(status: UpdateHistoryStatus): 'neutral' | 'info' | 'success' | 'warning' | 'danger' {
		switch (status) {
			case 'queued':
				return 'neutral';
			case 'pending':
				return 'info';
			case 'in_progress':
				return 'warning';
			case 'completed':
				return 'success';
			case 'failed':
				return 'danger';
		}
	}

	function statusLabel(status: UpdateHistoryStatus): string {
		return status === 'in_progress' ? 'In Progress' : status.charAt(0).toUpperCase() + status.slice(1);
	}

	function historyStatusGlyph(status: UpdateHistoryStatus): string {
		switch (status) {
			case 'completed':
				return '✓';
			case 'failed':
				return '✕';
			case 'in_progress':
				return '↑';
			case 'queued':
			case 'pending':
				return '·';
		}
	}

	function historyStatusGlyphClasses(status: UpdateHistoryStatus): string {
		switch (status) {
			case 'completed':
				return 'border-[var(--color-success-border)] bg-[var(--color-success-bg)] text-[var(--color-success)]';
			case 'failed':
				return 'border-[var(--color-danger-border)] bg-[var(--color-danger-bg)] text-[var(--color-danger)]';
			case 'in_progress':
				return 'border-[var(--color-warning-border)] bg-[var(--color-warning-bg)] text-[var(--color-warning)]';
			case 'queued':
			case 'pending':
				return 'border-[var(--color-info-border)] bg-[var(--color-info-bg)] text-[var(--color-info)]';
		}
	}

	function historyEntryLabel(item: UpdateHistoryResponse): string {
		return `${item.software_item_name} on ${item.host_name}`;
	}

	function historyDayKey(timestamp: string | null): string {
		const date = timestamp ? new Date(timestamp) : new Date(0);
		return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
	}

	function historyDayLabel(timestamp: string | null): string {
		const date = timestamp ? new Date(timestamp) : new Date(0);
		const now = new Date();
		const dateStart = startOfLocalDay(date);
		const todayStart = startOfLocalDay(now);
		const daysDelta = Math.round((todayStart.getTime() - dateStart.getTime()) / 86_400_000);
		if (daysDelta === 0) return 'Today';
		if (daysDelta === 1) return 'Yesterday';
		return date.toLocaleDateString(undefined, {
			day: '2-digit',
			month: 'short',
			year: 'numeric'
		});
	}

	function startOfLocalDay(date: Date): Date {
		return new Date(date.getFullYear(), date.getMonth(), date.getDate());
	}

	function formatRelativeTime(timestamp: string | null): string {
		if (!timestamp) return 'Unknown';
		const value = new Date(timestamp).getTime();
		const deltaMs = Date.now() - value;
		if (!Number.isFinite(deltaMs)) return formatDate(timestamp);
		const absSeconds = Math.max(0, Math.floor(deltaMs / 1000));
		if (absSeconds < 60) return 'Just now';
		const minutes = Math.floor(absSeconds / 60);
		if (minutes < 60) return `${minutes}m ago`;
		const hours = Math.floor(minutes / 60);
		if (hours < 24) return `${hours}h ago`;
		const days = Math.floor(hours / 24);
		if (days < 7) return `${days}d ago`;
		return formatDate(timestamp);
	}

	function timestampValue(timestamp: string | null): number {
		if (!timestamp) return 0;
		const value = new Date(timestamp).getTime();
		return Number.isFinite(value) ? value : 0;
	}

	type SummaryBucket = {
		label: 'Running' | 'Waiting' | 'Failed' | 'Completed';
		value: number;
		tone: 'warning' | 'info' | 'danger' | 'success';
	};

	const showSummaryStrip = $derived(statusFilter === 'all' && currentPage === 1 && !loading && !error);

	const summaryBuckets = $derived.by<SummaryBucket[]>(() => {
		const counts = {
			running: items.filter((item) => item.status === 'in_progress').length,
			waiting: items.filter((item) => item.status === 'queued' || item.status === 'pending').length,
			failed: items.filter((item) => item.status === 'failed').length,
			completed: items.filter((item) => item.status === 'completed').length
		};
		return [
			{ label: 'Running', value: counts.running, tone: 'warning' },
			{ label: 'Waiting', value: counts.waiting, tone: 'info' },
			{ label: 'Failed', value: counts.failed, tone: 'danger' },
			{ label: 'Completed', value: counts.completed, tone: 'success' }
		];
	});

	function historySummaryValueClass(tone: SummaryBucket['tone']): string {
		switch (tone) {
			case 'warning':
				return 'text-[var(--color-warning)]';
			case 'info':
				return 'text-[var(--color-info)]';
			case 'danger':
				return 'text-[var(--color-danger)]';
			case 'success':
				return 'text-[var(--color-success)]';
		}
	}

	function historyActorLabel(item: UpdateHistoryResponse): string {
		const normalizedType = item.actor_type?.replaceAll(/[_-]+/g, ' ').trim().toLowerCase();
		const actorName = item.actor_name?.trim();
		if (normalizedType === 'user' && actorName) return `Triggered by user ${actorName}`;
		if (normalizedType === 'scheduler' && actorName) return `Triggered by scheduler ${actorName}`;
		if (actorName) return `Triggered by ${normalizedType || 'service'} ${actorName}`;
		if (normalizedType) return `Triggered by ${normalizedType}`;
		return 'Trigger source unknown';
	}

	function closeHistoryModal() {
		disconnectStream();
		expandedId = null;
	}

	function openHistoryModal(id: string) {
		if (expandedId === id) {
			return;
		}

		disconnectStream();
		expandedId = id;

		const item = items.find((entry) => entry.id === id);
		if (item && isLiveStatus(item.status)) {
			setTimeout(() => connectInteractive(id), 0);
		}
	}

	function connectionTone(state: InteractiveConnectionState): 'success' | 'warning' | 'neutral' {
		if (state === 'connected') return 'success';
		if (state === 'connecting') return 'warning';
		return 'neutral';
	}

	function terminalStatusLabelFor(item: UpdateHistoryResponse): string {
		if (item.status === 'in_progress' && activeStreamId === item.id) {
			if (wsState === 'connected') return 'Live';
			if (wsState === 'connecting') return 'Connecting';
		}
		return statusLabel(item.status);
	}

	function terminalStatusToneFor(item: UpdateHistoryResponse): 'neutral' | 'info' | 'success' | 'warning' | 'danger' {
		if (item.status === 'in_progress' && activeStreamId === item.id) {
			return connectionTone(wsState);
		}
		return statusBadgeTone(item.status);
	}

	function terminalDurationLabel(item: UpdateHistoryResponse): string {
		if (!item.started_at) return '0m';
		const startedAt = new Date(item.started_at).getTime();
		const endedAt = item.completed_at ? new Date(item.completed_at).getTime() : Date.now();
		if (!Number.isFinite(startedAt) || !Number.isFinite(endedAt)) return '0m';
		const elapsedSeconds = Math.max(0, Math.floor((endedAt - startedAt) / 1000));
		if (elapsedSeconds < 60) return '<1m';
		const minutes = Math.floor(elapsedSeconds / 60);
		if (minutes < 60) return `${minutes}m`;
		const hours = Math.floor(minutes / 60);
		const remainingMinutes = minutes % 60;
		return remainingMinutes === 0 ? `${hours}h` : `${hours}h ${remainingMinutes}m`;
	}

	function terminalMetadataFor(item: UpdateHistoryResponse): string {
		return `${item.host_name} · started ${formatRelativeTime(item.started_at)} · ${terminalDurationLabel(item)}`;
	}

	function terminalInlineBadgesFor(item: UpdateHistoryResponse) {
		const badges: Array<{ id: string; tone: 'warning' | 'info'; label: string }> = [];
		if (activeStreamId === item.id && item.interactive) {
			badges.push({
				id: 'interactive',
				tone: stdinAttention ? 'warning' : 'info',
				label: 'Interactive terminal'
			});
		}
		return badges;
	}

	function terminalCriticalBannerFor(
		item: UpdateHistoryResponse
	): { tone: 'warning' | 'danger' | 'info'; label: string; message: string } | undefined {
		if (item.output_truncated) {
			return {
				tone: 'warning',
				label: 'Output truncated',
				message: 'This update produced more than 50 MB of output. Only the first 50 MB is stored.'
			};
		}
		if (stdinAttention && activeStreamId === item.id) {
			return {
				tone: 'warning',
				label: 'Input required',
				message: 'The remote process is waiting for input.'
			};
		}
		return undefined;
	}

	function terminalEmptyStateFor(item: UpdateHistoryResponse): { label?: string; message: string } | undefined {
		if (isWaitingStatus(item.status)) {
			return {
				label: item.status === 'queued' ? 'Queued' : 'Pending',
				message:
					item.status === 'queued'
						? 'Waiting for another update on this host to finish.'
						: 'Waiting for the agent to start the update.'
			};
		}
		if (!isLiveStatus(item.status) && !item.output) {
			return { message: 'No output recorded.' };
		}
		return undefined;
	}

	function terminalDetailsFor(item: UpdateHistoryResponse): Array<{ id: string; label: string; value: string }> {
		const details: Array<{ id: string; label: string; value: string }> = [];
		if (item.actor_type) {
			details.push({
				id: 'actor',
				label: 'Actor',
				value: `${item.actor_type} (${item.actor_id})`
			});
		}
		if (item.pre_update_protection_summary) {
			details.push({
				id: 'protection',
				label: 'Additional details',
				value: item.pre_update_protection_summary
			});
		}
		if (item.recovery_hint) {
			details.push({
				id: 'recovery',
				label: 'Recovery hint',
				value: item.recovery_hint
			});
		}
		return details;
	}

	function terminalActionsFor(item: UpdateHistoryResponse): Array<{
		id: string;
		label: string;
		title: string;
		tone: 'danger';
		onclick: () => void;
	}> {
		if (activeStreamId === item.id && (wsState === 'connected' || wsState === 'connecting')) {
			return [
				{
					id: 'sigint',
					label: 'Ctrl+C',
					title: 'Send Ctrl+C (SIGINT)',
					tone: 'danger',
					onclick: () => activeWsHandle?.sendSignal(2)
				}
			];
		}
		return [];
	}

	async function openTriggerModal() {
		showTriggerModal = true;
		selectedItemId = '';
		selectedHostId = '';
		targetVersion = '';
		releaseTag = '';
		releaseUrl = '';
		try {
			const res = await getSoftwareItems(1, 100);
			softwareItems = res.items;
			selectedItemHosts = [];
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load software items');
			showTriggerModal = false;
		}
	}

	function closeTriggerModal() {
		showTriggerModal = false;
		selectedItemHosts = [];
	}

	$effect(() => {
		if (!showTriggerModal || !selectedItemId) {
			selectedItemHosts = [];
			return;
		}

		void (async () => {
			try {
				const detail = await getSoftwareItem(selectedItemId);
				selectedItemHosts = detail.hosts.map((host) => ({
					host_id: host.host_id,
					label: host.friendly_name || host.hostname
				}));
				if (selectedHostId && !detail.hosts.some((host) => host.host_id === selectedHostId)) {
					selectedHostId = '';
				}
			} catch (error) {
				selectedItemHosts = [];
				showError(error instanceof Error ? error.message : 'Failed to load hosts for the selected software item.');
			}
		})();
	});

	async function handleTrigger() {
		if (!selectedItemId || !selectedHostId || !targetVersion.trim() || triggering) return;
		triggering = true;
		try {
			const release_info =
				releaseTag.trim() || releaseUrl.trim()
					? {
							tag: releaseTag.trim() || targetVersion.trim(),
							release_url: releaseUrl.trim()
						}
					: undefined;
			const res = await triggerSoftwareUpdate(selectedItemId, selectedHostId, {
				to_version: targetVersion.trim(),
				release_info
			});
			closeTriggerModal();
			if (res.status === 'failed') {
				showError(`Update failed before dispatch — history ID: ${res.update_history_id}`);
				loadHistory(1);
				return;
			}
			showSuccess(`Update triggered — history ID: ${res.update_history_id}`);
			loadHistory(1);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to trigger update');
		} finally {
			triggering = false;
		}
	}
</script>

<svelte:window
	onkeydown={(e) => {
		if (e.key === 'Escape' && showTriggerModal) closeTriggerModal();
	}}
/>

{#if getUser()}
	<PageShell title="Update History" description="Inspect update execution status, output, and actor metadata.">
		{#if !canView}
			<Callout tone="danger" message="You do not have permission to view update history." />
		{:else}
			{#if showSummaryStrip}
				<section class="grid gap-3 sm:grid-cols-2 lg:grid-cols-4" data-ui="history-summary-strip">
					{#each summaryBuckets as bucket (bucket.label)}
						<div class="rounded-card border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-4">
							<p class="text-badge font-bold uppercase tracking-badge text-[var(--text-secondary)]">{bucket.label}</p>
							<p class={`mt-1 text-sm font-bold ${historySummaryValueClass(bucket.tone)}`}>
								{bucket.value}
							</p>
						</div>
					{/each}
				</section>
			{/if}

			<SectionCard title="Filters">
				{#snippet actions()}
					{#if canManage}
						<Button variant="primary" size="sm" onclick={openTriggerModal}>Trigger Update</Button>
					{/if}
				{/snippet}
				<div class="flex gap-1 flex-wrap">
					{#each ['all', 'pending', 'in_progress', 'completed', 'failed'] as s (s)}
						{@const chipLabel = s === 'in_progress' ? 'In Progress' : s.charAt(0).toUpperCase() + s.slice(1)}
						<Button
							variant={statusFilter === s ? 'accent' : 'ghost'}
							size="sm"
							ariaLabel={chipLabel}
							onclick={() => {
								currentPage = 1;
								statusFilter = s as StatusFilter;
								loadHistory(1);
							}}
						>
							{chipLabel}
						</Button>
					{/each}
				</div>
			</SectionCard>

			<SectionCard title="History Feed">
				{#if loading}
					<p class="py-8 text-center text-sm text-[var(--text-muted)]">Loading update history…</p>
				{:else if error}
					<Callout tone="danger" title="Failed to load update history" message={error} />
					<div class="mt-3">
						<Button variant="primary" size="sm" onclick={() => loadHistory(currentPage)}>Retry</Button>
					</div>
				{:else if groupedHistory.length === 0}
					<EmptyState title="No update history" description="No updates have been triggered yet." />
				{:else}
					<div class="space-y-5" data-ui="history-feed-list">
						{#each groupedHistory as group (group.key)}
							<section class="space-y-2" data-ui="history-feed-group">
								<h3
									class="px-1 text-table-header font-semibold uppercase tracking-table-header text-[var(--text-secondary)]"
								>
									{group.label}
								</h3>
								<div class="space-y-2">
									{#each group.items as item (item.id)}
										<article
											class="rounded-panel border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-3"
											data-ui="history-feed-item"
											data-status={item.status}
											data-testid={`history-feed-item-${item.id}`}
										>
											<div class="grid grid-cols-[24px_1fr] gap-3">
												<div
													class={`flex h-6 w-6 items-center justify-center rounded-card border text-table-body font-bold ${historyStatusGlyphClasses(item.status)}`}
													data-state={item.status}
													data-ui="history-status-glyph"
												>
													{historyStatusGlyph(item.status)}
												</div>
												<div class="space-y-2">
													<div class="flex flex-col gap-2 md:flex-row md:items-start md:justify-between">
														<div class="space-y-0.5">
															<p class="text-table-body font-semibold leading-tight text-[var(--text-primary)]">
																{historyEntryLabel(item)}
															</p>
														</div>
														<Button
															variant="ghost"
															size="sm"
															aria-haspopup="dialog"
															onclick={() => openHistoryModal(item.id)}
														>
															{item.status === 'in_progress' && item.interactive ? 'Attach terminal' : 'View logs'}
														</Button>
													</div>
													<div class="flex flex-wrap items-center gap-2 text-table-body text-[var(--text-secondary)]">
														<span class="font-mono">
															{formatVersion(item.from_version, '?')} →
															<span class="text-[var(--accent-bright)]">{formatVersion(item.to_version)}</span>
														</span>
														<StatusBadge tone={statusBadgeTone(item.status)} label={statusLabel(item.status)} />
														<span data-visual-dynamic="">{formatRelativeTime(item.started_at)}</span>
														<span>{historyActorLabel(item)}</span>
													</div>
												</div>
											</div>
										</article>
									{/each}
								</div>
							</section>
						{/each}
					</div>
				{/if}

				{#if !error}
					<div class="mt-4">
						<TableFooterBar total={totalItems} {currentPage} {totalPages} onPageChange={loadHistory} />
					</div>
				{/if}
			</SectionCard>

			{#if expandedItem}
				<TerminalOutput
					bind:this={terminalRef}
					open={true}
					title={`${expandedItem.software_item_name} on ${expandedItem.host_name}`}
					statusLabel={terminalStatusLabelFor(expandedItem)}
					statusTone={terminalStatusToneFor(expandedItem)}
					metadata={terminalMetadataFor(expandedItem)}
					criticalBanner={terminalCriticalBannerFor(expandedItem)}
					emptyState={terminalEmptyStateFor(expandedItem)}
					details={terminalDetailsFor(expandedItem)}
					actions={terminalActionsFor(expandedItem)}
					inlineBadges={terminalInlineBadgesFor(expandedItem)}
					showTerminal={isLiveStatus(expandedItem.status) || Boolean(expandedItem.output?.trim())}
					output={expandedItem.output ?? ''}
					onInput={isLiveStatus(expandedItem.status)
						? (data) => (activeStreamId === expandedItem.id ? activeWsHandle?.sendInput(data) : undefined)
						: undefined}
					onclose={closeHistoryModal}
				/>
			{/if}
		{/if}
	</PageShell>
{/if}

{#if showTriggerModal}
	<ModalShell onclose={closeTriggerModal} title="Trigger Software Update" maxWidth="max-w-lg">
		<div class="space-y-4">
			<FormFieldRow label="Software Item" inputId="trigger-software-item">
				<Select
					id="trigger-software-item"
					bind:value={selectedItemId}
					options={softwareItems.map((si) => ({ value: si.id, label: si.name }))}
					placeholder="— select —"
				/>
			</FormFieldRow>

			{#if selectedItem}
				<FormFieldRow label="Host" inputId="trigger-host">
					<Select
						id="trigger-host"
						bind:value={selectedHostId}
						options={selectedItemHosts.map((host) => ({ value: host.host_id, label: host.label }))}
						placeholder="— select —"
					/>
				</FormFieldRow>
			{/if}

			<FormFieldRow label="Target Version" inputId="trigger-target-version" required>
				<Input id="trigger-target-version" type="text" placeholder="e.g. 1.2.3" bind:value={targetVersion} />
			</FormFieldRow>

			<FormFieldRow label="Release Tag (optional)" inputId="trigger-release-tag">
				<Input id="trigger-release-tag" type="text" placeholder="e.g. v1.2.3" bind:value={releaseTag} />
			</FormFieldRow>

			<FormFieldRow label="Release URL (optional)" inputId="trigger-release-url">
				<Input id="trigger-release-url" type="url" placeholder="https://..." bind:value={releaseUrl} />
			</FormFieldRow>
		</div>

		{#snippet footer()}
			<Button variant="secondary" onclick={closeTriggerModal}>Cancel</Button>
			<Button
				variant="primary"
				loading={triggering}
				disabled={!selectedItemId || !selectedHostId || !targetVersion.trim()}
				onclick={handleTrigger}
			>
				Trigger Update
			</Button>
		{/snippet}
	</ModalShell>
{/if}
