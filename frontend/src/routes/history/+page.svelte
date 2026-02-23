<script lang="ts">
	import { onMount } from 'svelte';
	import { getUser } from '$lib/auth.svelte';
	import { listUpdateHistory, triggerSoftwareUpdate, getSoftwareItems } from '$lib/api';
	import { formatDate } from '$lib/utils';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import Pagination from '$lib/components/Pagination.svelte';
	import ModalBackdrop from '$lib/components/ModalBackdrop.svelte';
	import { Permission } from '$lib/types';
	import type { UpdateHistoryResponse, UpdateHistoryStatus, SoftwareItemResponse } from '$lib/types';

	type StatusFilter = 'all' | UpdateHistoryStatus;

	let items: UpdateHistoryResponse[] = $state([]);
	let loading: boolean = $state(false);
	let error: string | null = $state(null);
	let currentPage: number = $state(1);
	let totalPages: number = $state(1);
	let statusFilter: StatusFilter = $state('all');
	let expandedId: string | null = $state(null);

	// Trigger update modal state
	let showTriggerModal: boolean = $state(false);
	let softwareItems: SoftwareItemResponse[] = $state([]);
	let selectedItemId: string = $state('');
	let selectedHostId: string = $state('');
	let targetVersion: string = $state('');
	let releaseTag: string = $state('');
	let releaseUrl: string = $state('');
	let triggering: boolean = $state(false);

	const canManage = $derived(getUser()?.permissions.includes(Permission.ManageSoftware) ?? false);
	const canView = $derived(getUser()?.permissions.includes(Permission.ViewSoftware) ?? false);

	const selectedItem = $derived(softwareItems.find((i) => i.id === selectedItemId));

	onMount(() => {
		if (canView) loadHistory(1);
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
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load update history';
		} finally {
			loading = false;
		}
	}

	function toggleExpand(id: string) {
		expandedId = expandedId === id ? null : id;
	}

	function statusBadgeClass(status: UpdateHistoryStatus): string {
		switch (status) {
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
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load software items');
			showTriggerModal = false;
		}
	}

	function closeTriggerModal() {
		showTriggerModal = false;
	}

	async function handleTrigger() {
		if (!selectedItemId || !selectedHostId || !targetVersion.trim() || triggering) return;
		triggering = true;
		try {
			const res = await triggerSoftwareUpdate(selectedItemId, selectedHostId, {
				to_version: targetVersion.trim(),
				release_tag: releaseTag.trim() || undefined,
				release_url: releaseUrl.trim() || undefined
			});
			showSuccess(`Update triggered — history ID: ${res.update_history_id}`);
			closeTriggerModal();
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
									<span class="text-surface-500">{item.from_version ?? '?'}</span>
									→
									<span class="font-medium">{item.to_version}</span>
								</td>
								<td>
									<span class="badge {statusBadgeClass(item.status)}">
										{item.status === 'in_progress'
											? 'In Progress'
											: item.status.charAt(0).toUpperCase() + item.status.slice(1)}
									</span>
								</td>
								<td>{formatDate(item.started_at)}</td>
								<td>{formatDate(item.completed_at)}</td>
							</tr>
							{#if expandedId === item.id}
								<tr>
									<td colspan="6" class="bg-surface-50 dark:bg-surface-900 p-4">
										<p class="mb-1 text-sm font-medium text-surface-600 dark:text-surface-400">Output</p>
										{#if item.output}
											<pre
												class="rounded-md bg-surface-100 dark:bg-surface-800 p-3 font-mono text-xs whitespace-pre-wrap">{item.output}</pre>
										{:else}
											<p class="text-sm text-surface-500">No output recorded.</p>
										{/if}
										{#if item.initiated_by}
											<p class="mt-2 text-xs text-surface-500">Initiated by: {item.initiated_by}</p>
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

		<Pagination {currentPage} {totalPages} onPageChange={loadHistory} />
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
						<!-- hosts are loaded via software item detail; for now let user enter host UUID -->
					</select>
					<p class="text-xs text-surface-500 mt-1">
						Host selection requires loading the software item detail. Enter the host UUID manually if needed.
					</p>
					<input class="input mt-1" type="text" placeholder="Host UUID" bind:value={selectedHostId} />
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
