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
	import Pagination from '$lib/components/Pagination.svelte';
	import ModalBackdrop from '$lib/components/ModalBackdrop.svelte';
	import TerminalOutput from '$lib/components/TerminalOutput.svelte';
	import { connectInteractiveSession } from '$lib/interactive';
	import type { InteractiveConnectionState } from '$lib/interactive';
	import { connectEventStream } from '$lib/sse';
	import { Permission } from '$lib/types';
	import type { UpdateHistoryResponse, UpdateHistoryStatus, SoftwareItemResponse } from '$lib/types';
	import { Callout, PageShell, SectionCard, StatusBadge } from '$lib/components/ui';

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
	let terminalRefs: Record<string, TerminalOutput> = {};

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

	function toggleExpand(id: string) {
		const wasExpanded = expandedId === id;
		// Disconnect any existing SSE stream
		disconnectStream();

		if (wasExpanded) {
			expandedId = null;
			return;
		}

		expandedId = id;

		// If the item is in-progress, connect interactive WS
		const item = items.find((i) => i.id === id);
		if (item && isLiveStatus(item.status)) {
			// Defer connection to next tick so the terminal has mounted.
			setTimeout(() => connectInteractive(id), 0);
		}
	}

	function connectInteractive(updateHistoryId: string) {
		const termRef = terminalRefs[updateHistoryId];
		activeStreamId = updateHistoryId;
		wsState = 'connecting';
		stdinAttention = false;

		activeWsHandle = connectInteractiveSession(updateHistoryId, {
			onOutput: (line) => {
				termRef?.write(line.text);
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

	function connectionLabel(state: InteractiveConnectionState): string {
		if (state === 'connected') return 'Live';
		if (state === 'connecting') return 'Connecting';
		return 'Disconnected';
	}

	function connectionTone(state: InteractiveConnectionState): 'success' | 'warning' | 'neutral' {
		if (state === 'connected') return 'success';
		if (state === 'connecting') return 'warning';
		return 'neutral';
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
			<SectionCard title="Filters">
				{#snippet actions()}
					{#if canManage}
						<button class="btn preset-filled-primary-500" onclick={openTriggerModal}>Trigger Update</button>
					{/if}
				{/snippet}
				<div class="flex gap-1 flex-wrap">
					{#each ['all', 'pending', 'in_progress', 'completed', 'failed'] as const as s (s)}
						<button
							class="btn btn-sm {statusFilter === s ? 'preset-filled-primary-500' : 'preset-tonal'}"
							onclick={() => {
								currentPage = 1;
								statusFilter = s;
								loadHistory(1);
							}}
						>
							{s === 'in_progress' ? 'In Progress' : s.charAt(0).toUpperCase() + s.slice(1)}
						</button>
					{/each}
				</div>
			</SectionCard>

			<SectionCard title="History Feed">
				{#if loading}
					<Callout tone="info" message="Loading update history…" />
				{:else if error}
					<Callout tone="danger" title="Failed to load update history" message={error} />
					<div class="mt-3">
						<button class="btn preset-filled-primary-500" onclick={() => loadHistory(currentPage)}>Retry</button>
					</div>
				{:else if groupedHistory.length === 0}
					<Callout tone="info" title="No update history" message="No updates have been triggered yet." />
				{:else}
					<div class="space-y-5" data-ui="history-feed-list">
						{#each groupedHistory as group (group.key)}
							<section class="space-y-2" data-ui="history-feed-group">
								<h3 class="px-1 text-[10px] font-semibold uppercase tracking-[0.14em] text-[var(--text-secondary)]">
									{group.label}
								</h3>
								<div class="space-y-2">
									{#each group.items as item (item.id)}
										<article
											class="rounded-[4px] border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-2"
											data-ui="history-feed-item"
											data-status={item.status}
											data-testid={`history-feed-item-${item.id}`}
										>
											<div class="grid grid-cols-[24px_1fr_auto] items-start gap-3">
												<div
													class={`flex h-6 w-6 items-center justify-center rounded-[3px] border text-[12px] font-bold ${historyStatusGlyphClasses(item.status)}`}
													data-state={item.status}
													data-ui="history-status-glyph"
												>
													{historyStatusGlyph(item.status)}
												</div>
												<div class="space-y-0.5">
													<p class="text-[12px] font-semibold leading-tight text-[var(--text-primary)]">
														{historyEntryLabel(item)}
													</p>
													<p class="font-mono text-[11px] leading-tight text-[var(--text-secondary)]">
														{formatVersion(item.from_version, '?')} →
														<span class="text-[var(--accent-bright)]">{formatVersion(item.to_version)}</span>
													</p>
												</div>
												<div class="flex flex-col items-end gap-1">
													<StatusBadge tone={statusBadgeTone(item.status)} label={statusLabel(item.status)} />
													{#if item.status === 'in_progress' && item.interactive}
														<StatusBadge tone="warning" label="Input Required" />
													{/if}
													<span class="text-[10px] text-[var(--text-secondary)]"
														>{formatRelativeTime(item.started_at)}</span
													>
													<button
														type="button"
														class="text-[10px] font-semibold uppercase tracking-[0.08em] text-[var(--accent-bright)] hover:opacity-80"
														aria-label={`${expandedId === item.id ? 'Collapse' : 'Expand'} output for ${historyEntryLabel(item)}`}
														aria-expanded={expandedId === item.id}
														onclick={() => toggleExpand(item.id)}
													>
														{expandedId === item.id ? '▼ hide log' : '▶ view log'}
													</button>
												</div>
											</div>

											{#if expandedId === item.id}
												<div
													class="mt-3 space-y-3 border-t border-[var(--border-subtle)] pt-3"
													data-ui="history-feed-output"
												>
													<div class="flex flex-wrap items-center gap-2">
														<StatusBadge tone="neutral" label="Output" />
														{#if activeStreamId === item.id && (wsState === 'connected' || wsState === 'connecting')}
															<StatusBadge tone={connectionTone(wsState)} label={connectionLabel(wsState)} />
														{/if}
														{#if stdinAttention && activeStreamId === item.id}
															<StatusBadge tone="warning" label="Input Required" />
														{/if}
														{#if activeStreamId === item.id && (wsState === 'connected' || wsState === 'connecting')}
															<button
																class="btn btn-sm preset-tonal-error ml-auto text-xs"
																title="Send Ctrl+C (SIGINT)"
																onclick={() => activeWsHandle?.sendSignal(2)}
															>
																Ctrl+C
															</button>
														{/if}
													</div>

													{#if isLiveStatus(item.status)}
														<TerminalOutput
															bind:this={terminalRefs[item.id]}
															class="h-80"
															onInput={(data) =>
																activeStreamId === item.id ? activeWsHandle?.sendInput(data) : undefined}
														/>
													{:else if isWaitingStatus(item.status)}
														<Callout
															tone="info"
															message={item.status === 'queued'
																? 'Queued — waiting for another update on this host to finish.'
																: 'Pending — waiting for the agent to start the update.'}
														/>
													{:else if item.output}
														<TerminalOutput output={item.output} class="h-80" />
													{:else}
														<Callout tone="info" message="No output recorded." />
													{/if}

													{#if item.output_truncated}
														<Callout
															tone="warning"
															title="Output truncated"
															message="This update produced more than 50 MB of output. Only the first 50 MB is stored."
														/>
													{/if}
													{#if item.pre_update_protection_summary || item.recovery_hint}
														<Callout
															tone="info"
															title="Additional details"
															message={[item.pre_update_protection_summary, item.recovery_hint]
																.filter(Boolean)
																.join(' ')}
														/>
													{/if}
													{#if item.actor_type}
														<div class="flex flex-wrap items-center gap-2 text-xs text-[var(--text-secondary)]">
															<StatusBadge tone="neutral" label="Actor" />
															<span>{item.actor_type} ({item.actor_id})</span>
														</div>
													{/if}
												</div>
											{/if}
										</article>
									{/each}
								</div>
							</section>
						{/each}
					</div>
				{/if}

				{#if !error}
					<div class="mt-4">
						<Pagination {currentPage} {totalPages} total={totalItems} onPageChange={loadHistory} />
					</div>
				{/if}
			</SectionCard>
		{/if}
	</PageShell>
{/if}

{#if showTriggerModal}
	<ModalBackdrop onclose={closeTriggerModal}>
		<div
			class="card bg-surface-50 dark:bg-surface-900 w-full max-w-lg space-y-4 p-6 shadow-xl"
			role="dialog"
			aria-modal="true"
		>
			<h3 class="h3">Trigger Software Update</h3>

			<label class="label">
				<span>Software Item</span>
				<select class="select" bind:value={selectedItemId}>
					<option value="">— select —</option>
					{#each softwareItems as si (si.id)}
						<option value={si.id}>{si.name}</option>
					{/each}
				</select>
			</label>

			{#if selectedItem}
				<label class="label">
					<span>Host</span>
					<select class="select" bind:value={selectedHostId}>
						<option value="">— select —</option>
						{#each selectedItemHosts as host (host.host_id)}
							<option value={host.host_id}>{host.label}</option>
						{/each}
					</select>
				</label>
			{/if}

			<label class="label">
				<span>Target Version <span class="text-error-500">*</span></span>
				<input class="input" type="text" placeholder="e.g. 1.2.3" bind:value={targetVersion} />
			</label>

			<div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
				<label class="label">
					<span>Release Tag (optional)</span>
					<input class="input" type="text" placeholder="e.g. v1.2.3" bind:value={releaseTag} />
				</label>
				<label class="label">
					<span>Release URL (optional)</span>
					<input class="input" type="text" placeholder="https://..." bind:value={releaseUrl} />
				</label>
			</div>

			<div class="flex justify-end gap-2">
				<button class="btn preset-tonal-surface" onclick={closeTriggerModal}>Cancel</button>
				<button
					class="btn preset-filled-primary-500"
					onclick={handleTrigger}
					disabled={!selectedItemId || !selectedHostId || !targetVersion.trim() || triggering}
				>
					{triggering ? 'Triggering...' : 'Trigger Update'}
				</button>
			</div>
		</div>
	</ModalBackdrop>
{/if}
