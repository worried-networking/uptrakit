<script lang="ts">
	import { SvelteSet } from 'svelte/reactivity';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import {
		getHosts,
		updateHost,
		deactivateHost,
		triggerHostDiscovery,
		batchHosts,
		executeBatchChunked
	} from '$lib/api';
	import type { HostResponse, BatchActionResponse } from '$lib/types';
	import { Permission, hasAnyPermission } from '$lib/types';
	import { formatDate, parseUrlPage, nextValidPage } from '$lib/utils';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import { subscribeToEvent } from '$lib/stores/events.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import Pagination from '$lib/components/Pagination.svelte';
	import BatchActionBar from '$lib/components/BatchActionBar.svelte';
	import BatchResultDialog from '$lib/components/BatchResultDialog.svelte';
	import TagBadge from '$lib/components/TagBadge.svelte';
	import { ContextMenuShell, DataTable, ModalShell, PageShell, SectionCard, StatusBadge } from '$lib/components/ui';

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
	let selectingAllPages = $state(false);

	const batchActions = $derived.by(() => {
		const actions: { id: string; label: string; destructive?: boolean }[] = [];
		if (canManageSoftware) {
			actions.push({ id: 'discover', label: 'Trigger Discovery' });
		}
		if (canManage) {
			actions.push({ id: 'deactivate', label: 'Deactivate', destructive: true });
		}
		return actions;
	});

	const allPageSelected = $derived(hosts.length > 0 && hosts.every((h) => selectedIds.has(h.id)));

	const selectAllPagesInfo = $derived(
		allPageSelected && totalItems > hosts.length && selectedIds.size < totalItems
			? { total: totalItems, loading: selectingAllPages, onSelect: selectAllPages }
			: undefined
	);

	$effect(() => {
		const search = currentPage > 1 ? `page=${currentPage}` : '';
		goto(search ? `${location.pathname}?${search}` : location.pathname, {
			replaceState: true,
			keepFocus: true,
			noScroll: true
		});
	});

	$effect(() => {
		loadHosts(currentPage);
		const unsubs = [
			subscribeToEvent('host_updated', () => loadHosts(currentPage, true)),
			subscribeToEvent('host_created', () => loadHosts(currentPage, true)),
			subscribeToEvent('host_deleted', () => loadHosts(currentPage, true)),
			subscribeToEvent('discovery_completed', () => loadHosts(currentPage, true))
		];
		const interval = setInterval(() => {
			if (document.visibilityState === 'visible') loadHosts(currentPage, true);
		}, 300_000);
		return () => {
			for (const unsub of unsubs) unsub();
			clearInterval(interval);
		};
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
			await loadHosts(currentPage);
			const p = nextValidPage(currentPage, totalPages);
			if (p !== null) await loadHosts(p);
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to deactivate host';
		} finally {
			submitting = false;
		}
	}

	function toggleSelectAll() {
		if (allPageSelected) {
			for (const h of hosts) selectedIds.delete(h.id);
		} else {
			for (const h of hosts) selectedIds.add(h.id);
		}
	}

	async function selectAllPages() {
		selectingAllPages = true;
		try {
			let p = 1;
			while (true) {
				const result = await getHosts(p, 100);
				for (const host of result.items) selectedIds.add(host.id);
				if (p >= result.total_pages) break;
				p++;
			}
		} catch {
			showError('Failed to select all items');
		} finally {
			selectingAllPages = false;
		}
	}

	function toggleSelect(id: string) {
		if (selectedIds.has(id)) {
			selectedIds.delete(id);
		} else {
			selectedIds.add(id);
		}
	}

	async function executeBatchDiscover() {
		const ids = [...selectedIds];
		selectedIds.clear();
		const results = await Promise.allSettled(ids.map((id) => triggerHostDiscovery(id)));
		const succeeded = results.filter((r) => r.status === 'fulfilled').length;
		const failed = results.length - succeeded;
		if (succeeded > 0 && failed === 0) {
			showSuccess(`Discovery triggered for ${succeeded} host(s).`);
		} else if (succeeded > 0) {
			showSuccess(`Discovery triggered for ${succeeded} host(s). ${failed} failed.`);
		} else {
			showError(`Failed to trigger discovery for all ${ids.length} host(s).`);
		}
	}

	function requestBatchAction(actionId: string) {
		if (actionId === 'deactivate') {
			batchConfirmAction = actionId;
		} else if (actionId === 'discover') {
			void executeBatchDiscover();
		}
	}

	async function executeBatchAction() {
		if (!batchConfirmAction || submitting) return;
		const action = batchConfirmAction;
		batchConfirmAction = null;
		submitting = true;
		try {
			const response = await executeBatchChunked(action, [...selectedIds], batchHosts);
			if (response.failed.length > 0) {
				batchResult = response;
			} else {
				showSuccess(`${response.succeeded.length} host(s) deactivated successfully.`);
			}
			selectedIds.clear();
			await loadHosts(currentPage);
			const p = nextValidPage(currentPage, totalPages);
			if (p !== null) await loadHosts(p);
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

	const canManage = $derived(hasAnyPermission(getUser(), Permission.UpdateHosts, Permission.DeactivateHosts));
	const canManageSoftware = $derived(
		hasAnyPermission(
			getUser(),
			Permission.CreateSoftware,
			Permission.UpdateSoftware,
			Permission.DeleteSoftware,
			Permission.TriggerChecks,
			Permission.TriggerUpdates
		)
	);
</script>

<svelte:window onclick={handleWindowClick} />

{#if getUser()}
	<PageShell title="Hosts" description="Manage enrolled hosts and trigger host-level actions.">
		<SectionCard title="Registered Hosts" description="Hosts appear here after approved agents enroll.">
			<DataTable
				columns={[]}
				rows={hosts as unknown as Record<string, unknown>[]}
				{error}
				emptyTitle="No hosts discovered yet"
				emptyDescription="Hosts appear here automatically when an approved agent reports from a new machine."
				rowKey={(row) => (row as unknown as HostResponse).id}
			>
				{#snippet header()}
					<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
						{#if canManage || canManageSoftware}
							<th class="w-10 px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">
								<input
									type="checkbox"
									class="checkbox"
									checked={allPageSelected}
									indeterminate={!allPageSelected && selectedIds.size > 0}
									onchange={toggleSelectAll}
									aria-label="Select all"
								/>
							</th>
						{/if}
						<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">Name</th>
						<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">Tags</th>
						<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">Hostname</th>
						<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">OS</th>
						<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">
							Architecture
						</th>
						<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">IP</th>
						<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">Agents</th>
						<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">Last Seen</th>
						{#if canManage}
							<th
								class="w-20 px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em] sticky right-0 bg-[var(--bg-raised)]"
								scope="col"
							></th>
						{/if}
					</tr>
				{/snippet}
				{#snippet row(rowValue)}
					{@const host = rowValue as unknown as HostResponse}
					<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
						{#if canManage || canManageSoftware}
							<td class="px-4 py-3">
								<input
									type="checkbox"
									class="checkbox"
									checked={selectedIds.has(host.id)}
									onchange={() => toggleSelect(host.id)}
									aria-label="Select {host.friendly_name}"
								/>
							</td>
						{/if}
						<td class="px-4 py-3 text-[var(--text-primary)]">
							<a href="/hosts/{host.id}" class="hover:underline font-medium">{host.friendly_name}</a>
						</td>
						<td class="px-4 py-3 text-[var(--text-primary)]">
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
						<td class="px-4 py-3 text-[var(--text-primary)]">{host.hostname}</td>
						<td class="px-4 py-3 text-[var(--text-primary)]">{host.os_version ?? host.os_type ?? '\u2014'}</td>
						<td class="px-4 py-3 text-[var(--text-primary)]">{host.architecture ?? '\u2014'}</td>
						<td class="px-4 py-3 text-[var(--text-primary)]">{host.ip_address ?? '\u2014'}</td>
						<td class="px-4 py-3 text-[var(--text-primary)]">
							<StatusBadge
								tone={host.agents.length > 0 ? 'success' : 'neutral'}
								label={host.agents.length === 1 ? '1 agent' : `${host.agents.length} agents`}
							/>
						</td>
						<td class="px-4 py-3 text-[var(--text-primary)]">{formatDate(host.last_seen_at)}</td>
						{#if canManage}
							<td class="px-4 py-3 sticky right-0 bg-[var(--bg-surface)]">
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
				{/snippet}
				{#snippet errorActions()}
					<button class="btn preset-filled-primary-500 mt-3" onclick={() => loadHosts(currentPage)}>Retry</button>
				{/snippet}
			</DataTable>

			{#if !error}
				<div class="mt-4">
					<Pagination {currentPage} {totalPages} total={totalItems} onPageChange={loadHosts} />
				</div>
			{/if}
		</SectionCard>
	</PageShell>

	{#if (canManage || canManageSoftware) && selectedIds.size > 0}
		<BatchActionBar
			selectedCount={selectedIds.size}
			actions={batchActions}
			onaction={requestBatchAction}
			oncancel={() => selectedIds.clear()}
			selectAllPages={selectAllPagesInfo}
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
			<ContextMenuShell top={menuPos.top} left={menuPos.left} onclose={closeMenu}>
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
			</ContextMenuShell>
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
		<ModalShell title="Edit Host Name" onclose={cancelEdit}>
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
		</ModalShell>
	{/if}
{/if}
