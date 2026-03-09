<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import { getHosts, updateHost, deactivateHost, triggerHostDiscovery, batchHosts } from '$lib/api';
	import type { HostResponse, BatchActionResponse } from '$lib/types';
	import { Permission } from '$lib/types';
	import { formatDate, parseUrlPage } from '$lib/utils';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import { subscribeToEvent } from '$lib/stores/events.svelte';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import Modal from '$lib/components/Modal.svelte';
	import Pagination from '$lib/components/Pagination.svelte';
	import BatchActionBar from '$lib/components/BatchActionBar.svelte';
	import BatchResultDialog from '$lib/components/BatchResultDialog.svelte';
	import TagBadge from '$lib/components/TagBadge.svelte';

	let hosts: HostResponse[] = $state([]);
	let error: string | null = $state(null);
	let openMenuId: string | null = $state(null);
	let menuPos: { top: number; left: number } = $state({ top: 0, left: 0 });
	let confirmAction: { hostId: string; action: 'deactivate'; name: string } | null = $state(null);
	let editHost: { id: string; friendlyName: string } | null = $state(null);
	let submitting: boolean = $state(false);
	let currentPage: number = $state(parseUrlPage(page.url));
	let totalPages: number = $state(1);
	let totalItems: number = $state(0);

	let discoveringHostIds: Set<string> = $state(new Set());
	let selectedIds = new SvelteSet<string>();
	let batchConfirmAction: string | null = $state(null);
	let batchResult: BatchActionResponse | null = $state(null);

	const batchActions: { id: string; label: string; destructive?: boolean }[] = [
		{ id: 'deactivate', label: 'Deactivate', destructive: true }
	];

	let refreshInterval: ReturnType<typeof setInterval> | null = null;
	let unsubscribers: (() => void)[] = [];

	$effect(() => {
		const search = currentPage > 1 ? `page=${currentPage}` : '';
		goto(search ? `${location.pathname}?${search}` : location.pathname, {
			replaceState: true,
			keepFocus: true,
			noScroll: true
		});
	});

	onMount(() => {
		loadHosts(currentPage);
		unsubscribers.push(
			subscribeToEvent('host_updated', () => loadHosts(currentPage, true)),
			subscribeToEvent('host_created', () => loadHosts(currentPage, true)),
			subscribeToEvent('host_deleted', () => loadHosts(currentPage, true)),
			subscribeToEvent('discovery_completed', () => loadHosts(currentPage, true))
		);
		refreshInterval = setInterval(() => {
			if (document.visibilityState === 'visible') loadHosts(currentPage, true);
		}, 300_000);
	});

	onDestroy(() => {
		for (const unsub of unsubscribers) unsub();
		if (refreshInterval) clearInterval(refreshInterval);
	});

	async function loadHosts(page: number, background = false) {
		try {
			if (!background) error = null;
			const result = await getHosts(page);
			hosts = result.items;
			currentPage = result.page;
			totalPages = result.total_pages;
			totalItems = result.total;
			if (background) error = null;
		} catch (e) {
			if (!background) {
				error = e instanceof Error ? e.message : 'Failed to load hosts';
			}
		}
	}

	function toggleMenu(id: string, button: HTMLElement) {
		if (openMenuId === id) {
			openMenuId = null;
			return;
		}
		const rect = button.getBoundingClientRect();
		menuPos = { top: rect.bottom + 4, left: rect.right - 160 };
		openMenuId = id;
	}

	function closeMenu() {
		openMenuId = null;
	}

	function requestConfirm(hostId: string, action: 'deactivate', name: string) {
		closeMenu();
		confirmAction = { hostId, action, name };
	}

	function cancelConfirm() {
		confirmAction = null;
	}

	function openEditDialog(host: HostResponse) {
		closeMenu();
		editHost = { id: host.id, friendlyName: host.friendly_name };
	}

	function cancelEdit() {
		editHost = null;
	}

	async function executeEdit() {
		if (!editHost || submitting) return;
		submitting = true;
		try {
			error = null;
			const updated = await updateHost(editHost.id, { friendly_name: editHost.friendlyName });
			hosts = hosts.map((h) => (h.id === editHost?.id ? updated : h));
			editHost = null;
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to update host';
		} finally {
			submitting = false;
		}
	}

	async function executeConfirmed() {
		if (!confirmAction || submitting) return;
		const { hostId } = confirmAction;
		confirmAction = null;
		submitting = true;

		try {
			error = null;
			await deactivateHost(hostId);
			hosts = hosts.filter((h) => h.id !== hostId);
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to deactivate host';
		} finally {
			submitting = false;
		}
	}

	function toggleSelectAll() {
		if (selectedIds.size === hosts.length) {
			selectedIds.clear();
		} else {
			selectedIds.clear();
			for (const h of hosts) selectedIds.add(h.id);
		}
	}

	function toggleSelect(id: string) {
		if (selectedIds.has(id)) {
			selectedIds.delete(id);
		} else {
			selectedIds.add(id);
		}
	}

	function requestBatchAction(actionId: string) {
		batchConfirmAction = actionId;
	}

	async function executeBatchAction() {
		if (!batchConfirmAction || submitting) return;
		const action = batchConfirmAction;
		batchConfirmAction = null;
		submitting = true;
		try {
			const response = await batchHosts(action, [...selectedIds]);
			if (response.failed.length > 0) {
				batchResult = response;
			} else {
				showSuccess(`${response.succeeded.length} host(s) deactivated successfully.`);
			}
			selectedIds.clear();
			await loadHosts(currentPage);
		} catch (e) {
			showError(e instanceof Error ? e.message : `Failed to ${action} hosts`);
		} finally {
			submitting = false;
		}
	}

	function handleWindowClick(event: MouseEvent) {
		if (openMenuId && !(event.target as HTMLElement).closest('.actions-menu')) {
			closeMenu();
		}
	}

	async function triggerDiscovery(host: HostResponse) {
		closeMenu();
		discoveringHostIds = new Set([...discoveringHostIds, host.id]);
		try {
			const result = await triggerHostDiscovery(host.id);
			if (result.plugins_queued > 0) {
				showSuccess(`Discovery triggered — ${result.plugins_queued} plugin(s) queued for "${host.friendly_name}"`);
			} else {
				showSuccess(`Discovery triggered — no plugins queued for "${host.friendly_name}"`);
			}
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to trigger discovery.');
		} finally {
			discoveringHostIds = new Set([...discoveringHostIds].filter((id) => id !== host.id));
		}
	}

	const canManage = $derived(getUser()?.permissions.includes(Permission.ManageHosts) ?? false);
	const canManageSoftware = $derived(getUser()?.permissions.includes(Permission.ManageSoftware) ?? false);
</script>

<svelte:window onclick={handleWindowClick} />

{#if getUser()}
	<h1 class="h1 mb-6">Hosts</h1>

	{#if error}
		<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
			<p>{error}</p>
			<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadHosts(currentPage)}>Retry</button>
		</aside>
	{/if}

	<div class="table-wrap">
		<table class="table">
			<thead>
				<tr>
					{#if canManage}
						<th class="w-10">
							<input
								type="checkbox"
								class="checkbox"
								checked={hosts.length > 0 && selectedIds.size === hosts.length}
								indeterminate={selectedIds.size > 0 && selectedIds.size < hosts.length}
								onchange={toggleSelectAll}
								aria-label="Select all"
							/>
						</th>
					{/if}
					<th>Name</th>
					<th>Tags</th>
					<th>Hostname</th>
					<th>OS</th>
					<th>Architecture</th>
					<th>IP</th>
					<th>Agents</th>
					<th>Last Seen</th>
					{#if canManage}
						<th class="w-20 sticky right-0 bg-surface-50 dark:bg-surface-900"></th>
					{/if}
				</tr>
			</thead>
			<tbody>
				{#each hosts as host (host.id)}
					<tr>
						{#if canManage}
							<td>
								<input
									type="checkbox"
									class="checkbox"
									checked={selectedIds.has(host.id)}
									onchange={() => toggleSelect(host.id)}
									aria-label="Select {host.friendly_name}"
								/>
							</td>
						{/if}
						<td>
							<a href="/hosts/{host.id}" class="hover:underline font-medium">{host.friendly_name}</a>
						</td>
						<td>
							{#if host.tags && host.tags.length > 0}
								<div class="flex flex-wrap gap-1">
									{#each host.tags as tag (tag.id)}
										<TagBadge name={tag.name} color={tag.color} />
									{/each}
								</div>
							{:else}
								<span class="text-surface-400">&mdash;</span>
							{/if}
						</td>
						<td>{host.hostname}</td>
						<td>{host.os_version ?? host.os_type ?? '\u2014'}</td>
						<td>{host.architecture ?? '\u2014'}</td>
						<td>{host.ip_address ?? '\u2014'}</td>
						<td>{host.agents.length}</td>
						<td>{formatDate(host.last_seen_at)}</td>
						{#if canManage}
							<td class="sticky right-0 bg-surface-50 dark:bg-surface-900">
								<div class="actions-menu">
									<button
										class="btn btn-sm preset-tonal"
										aria-label="Actions for {host.friendly_name}"
										onclick={(e) => {
											e.stopPropagation();
											toggleMenu(host.id, e.currentTarget);
										}}
									>
										&#8943;
									</button>
								</div>
							</td>
						{/if}
					</tr>
				{:else}
					<tr>
						<td colspan={canManage ? 11 : 8} class="text-center py-8">
							<p class="text-lg font-medium">No hosts discovered yet</p>
							<p class="mt-1 text-sm text-surface-500">
								Hosts appear here automatically when an approved agent reports from a new machine.
							</p>
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>

	<Pagination {currentPage} {totalPages} total={totalItems} onPageChange={loadHosts} />

	{#if canManage && selectedIds.size > 0}
		<BatchActionBar
			selectedCount={selectedIds.size}
			actions={batchActions}
			onaction={requestBatchAction}
			oncancel={() => selectedIds.clear()}
		/>
	{/if}

	{#if batchConfirmAction}
		<ConfirmDialog
			title="Batch Deactivate"
			messagePrefix="Are you sure you want to deactivate"
			entityName="{selectedIds.size} host(s)"
			confirmLabel={submitting ? 'Processing...' : 'Deactivate'}
			confirmClass="preset-filled-error-500"
			confirmDisabled={submitting}
			onconfirm={executeBatchAction}
			oncancel={() => (batchConfirmAction = null)}
		/>
	{/if}

	{#if batchResult}
		<BatchResultDialog title="Batch Action Results" response={batchResult} onclose={() => (batchResult = null)} />
	{/if}

	{#if openMenuId}
		{@const host = hosts.find((h) => h.id === openMenuId)}
		{#if host}
			<ContextMenu top={menuPos.top} left={menuPos.left} onclose={closeMenu}>
				<li>
					<button
						class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800"
						role="menuitem"
						tabindex="-1"
						onclick={() => openEditDialog(host)}
					>
						Edit Name
					</button>
				</li>
				{#if canManageSoftware}
					<li>
						<button
							class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800 disabled:cursor-not-allowed disabled:opacity-50"
							role="menuitem"
							tabindex="-1"
							disabled={discoveringHostIds.has(host.id)}
							onclick={() => triggerDiscovery(host)}
						>
							{discoveringHostIds.has(host.id) ? 'Triggering...' : 'Trigger Discovery'}
						</button>
					</li>
				{/if}
				<li>
					<button
						class="w-full rounded-md px-3 py-2 text-left text-sm text-error-500 hover:bg-surface-200 dark:hover:bg-surface-800"
						role="menuitem"
						tabindex="-1"
						onclick={() => requestConfirm(host.id, 'deactivate', host.friendly_name)}
					>
						Deactivate
					</button>
				</li>
			</ContextMenu>
		{/if}
	{/if}

	{#if confirmAction}
		<ConfirmDialog
			title="Deactivate Host"
			messagePrefix="Are you sure you want to deactivate"
			entityName={confirmAction.name}
			confirmLabel={submitting ? 'Processing...' : 'Deactivate'}
			confirmDisabled={submitting}
			onconfirm={executeConfirmed}
			oncancel={cancelConfirm}
		/>
	{/if}

	{#if editHost}
		<Modal title="Edit Host Name" onclose={cancelEdit}>
			<label class="label">
				<span>Friendly Name</span>
				<input class="input" type="text" bind:value={editHost.friendlyName} />
			</label>
			{#snippet footer()}
				<button class="btn preset-tonal-surface" onclick={cancelEdit}>Cancel</button>
				<button class="btn preset-filled-primary-500" disabled={submitting} onclick={executeEdit}>
					{submitting ? 'Saving...' : 'Save'}
				</button>
			{/snippet}
		</Modal>
	{/if}
{/if}
