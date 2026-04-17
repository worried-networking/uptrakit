<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
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

	type StatusFilter = 'all' | UpdateHistoryStatus;
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

	function statusBadgeClass(status: UpdateHistoryStatus): string {
		switch (status) {
			case 'queued':
				return 'preset-tonal-surface';
			case 'pending':
				return 'preset-tonal';
			case 'in_progress':
				return 'preset-filled-warning-500';
			case 'completed':
				return 'preset-filled-success-500';
			case 'failed':
				return 'preset-filled-error-500';
		}
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
	<h1 class="h1 mb-4">Update History</h1>

	{#if !canView}
		<aside class="rounded-lg p-4 preset-filled-error-500">
			<p>You do not have permission to view update history.</p>
		</aside>
	{:else}
		<div class="mb-4 flex items-center justify-between flex-wrap gap-2">
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
			{#if canManage}
				<button class="btn preset-filled-primary-500" onclick={openTriggerModal}>Trigger Update</button>
			{/if}
		</div>

		{#if error}
			<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
				<p>{error}</p>
				<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadHistory(currentPage)}>Retry</button>
			</aside>
		{/if}

		<div class="table-wrap">
			<table class="table">
				<thead>
					<tr>
						<th>Host</th>
						<th>Software</th>
						<th>Version</th>
						<th>Status</th>
						<th>Started</th>
						<th>Completed</th>
					</tr>
				</thead>
				<tbody>
					{#if loading}
						<tr>
							<td colspan="6" class="py-6 text-center">Loading...</td>
						</tr>
					{:else}
						{#each items as item (item.id)}
							<tr
								class="cursor-pointer hover:bg-surface-100 dark:hover:bg-surface-800"
								onclick={() => toggleExpand(item.id)}
							>
								<td>{item.host_name}</td>
								<td>{item.software_item_name}</td>
								<td>
									<span class="text-surface-500" title={item.from_version ?? undefined}
										>{formatVersion(item.from_version, '?')}</span
									>
									→
									<span class="font-medium" title={item.to_version}>{formatVersion(item.to_version)}</span>
								</td>
								<td>
									<span class="badge {statusBadgeClass(item.status)}">
										{item.status === 'in_progress'
											? 'In Progress'
											: item.status.charAt(0).toUpperCase() + item.status.slice(1)}
									</span>
									{#if item.status === 'in_progress' && item.interactive}
										<span class="badge preset-filled-warning-500 text-xs animate-pulse">Input Required</span>
									{/if}
								</td>
								<td>{formatDate(item.started_at)}</td>
								<td>{formatDate(item.completed_at)}</td>
							</tr>
							{#if expandedId === item.id}
								<tr>
									<td colspan="6" class="bg-surface-50 dark:bg-surface-900 p-4">
										<div class="mb-1 flex items-center gap-2">
											<p class="text-sm font-medium text-surface-600 dark:text-surface-400">Output</p>
											{#if activeStreamId === item.id && wsState === 'connected'}
												<span class="badge preset-filled-success-500 text-xs animate-pulse">Live</span>
											{:else if activeStreamId === item.id && wsState === 'connecting'}
												<span class="badge preset-tonal text-xs">Connecting…</span>
											{/if}
											{#if stdinAttention && activeStreamId === item.id}
												<span class="badge preset-filled-warning-500 text-xs animate-pulse">Input Required</span>
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
												onInput={(data) => (activeStreamId === item.id ? activeWsHandle?.sendInput(data) : undefined)}
											/>
										{:else if isWaitingStatus(item.status)}
											<p class="text-sm text-surface-500">
												{item.status === 'queued'
													? 'Queued — waiting for another update on this host to finish.'
													: 'Pending — waiting for the agent to start the update.'}
											</p>
										{:else if item.output}
											<TerminalOutput output={item.output} class="h-80" />
										{:else}
											<p class="text-sm text-surface-500">No output recorded.</p>
										{/if}
										{#if item.output_truncated}
											<aside class="mt-2 rounded-lg p-3 preset-tonal-warning text-sm">
												<strong>Output truncated</strong> — this update produced more than 50 MB of output. Only the first
												50 MB is stored.
											</aside>
										{/if}
										{#if item.pre_update_protection_summary || item.recovery_hint}
											<aside class="mt-2 rounded-lg p-3 preset-tonal text-sm space-y-1">
												<p class="font-medium">Additional details</p>
												{#if item.pre_update_protection_summary}
													<p>{item.pre_update_protection_summary}</p>
												{/if}
												{#if item.recovery_hint}
													<p>{item.recovery_hint}</p>
												{/if}
											</aside>
										{/if}
										{#if item.actor_type}
											<p class="mt-2 text-xs text-surface-500">
												Actor: {item.actor_type} ({item.actor_id})
											</p>
										{/if}
									</td>
								</tr>
							{/if}
						{:else}
							<tr>
								<td colspan="6" class="py-8 text-center">
									<p class="text-lg font-medium">No update history</p>
									<p class="mt-1 text-sm text-surface-500">No updates have been triggered yet.</p>
								</td>
							</tr>
						{/each}
					{/if}
				</tbody>
			</table>
		</div>

		<Pagination {currentPage} {totalPages} total={totalItems} onPageChange={loadHistory} />
	{/if}
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
