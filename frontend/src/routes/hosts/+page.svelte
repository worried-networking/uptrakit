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
	import {
		ActionBadge,
		ContextMenuItem,
		ContextMenuShell,
		DataTable,
		ModalShell,
		PageShell,
		SectionCard,
		StatusBadge
	} from '$lib/components/ui';
	import { FormFieldRow, Checkbox, Input } from '$lib/components/forms';
	import Button from '$lib/components/Button.svelte';

	let hosts: HostResponse[] = $state([]);
	let error: string | null = $state(null);
	let isRetrying: boolean = $state(false);
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

	async function retryLoad() {
		isRetrying = true;
		try {
			await loadHosts(currentPage);
		} finally {
			isRetrying = false;
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
	const canViewSoftware = $derived(hasAnyPermission(getUser(), Permission.ViewSoftware));
	const canManageSoftware = $derived(
		hasAnyPermission(
			getUser(),
			Permission.ViewSoftware,
			Permission.CreateSoftware,
			Permission.UpdateSoftware,
			Permission.DeleteSoftware,
			Permission.TriggerChecks,
			Permission.TriggerUpdates
		)
	);

	function softwareStatus(host: HostResponse): { known: boolean; update_count: number; error_count: number } {
		return host.software_status ?? { known: false, update_count: 0, error_count: 0 };
	}

	function updateBadgeLabel(updateCount: number): string {
		return updateCount === 1 ? '1 update' : `${updateCount} updates`;
	}

	function errorBadgeLabel(errorCount: number): string {
		return errorCount === 1 ? '1 error' : `${errorCount} errors`;
	}

	function openHostSoftware(hostId: string): void {
		void goto(`/software?host_id=${hostId}`);
	}

	function openHostHistory(hostId: string): void {
		void goto(`/history?host_id=${hostId}`);
	}

	const onlineCount = $derived(hosts.filter((host) => host.last_seen_at !== null).length);
	const offlineCount = $derived(hosts.length - onlineCount);
	const updatesPendingCount = $derived(hosts.filter((host) => softwareStatus(host).update_count > 0).length);
	const errorCount = $derived(hosts.filter((host) => softwareStatus(host).error_count > 0).length);
</script>

<svelte:window onclick={handleWindowClick} />

{#if getUser()}
	<PageShell title="Hosts" description="Manage enrolled hosts and trigger host-level actions.">
		<SectionCard title="Registered Hosts" description="Hosts appear here after approved agents enroll.">
			<div class="mb-4 grid grid-cols-2 gap-2 lg:grid-cols-4" data-ui="host-stat-grid">
				<article
					class="rounded-card border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-2"
					data-ui="host-stat-card"
					data-testid="host-stat-online"
				>
					<p class="text-badge uppercase tracking-badge text-[var(--text-secondary)]">Online</p>
					<p class="mt-1 text-sm font-bold text-[var(--color-success)]">{onlineCount}</p>
				</article>
				<article
					class="rounded-card border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-2"
					data-ui="host-stat-card"
					data-testid="host-stat-offline"
				>
					<p class="text-badge uppercase tracking-badge text-[var(--text-secondary)]">Offline</p>
					<p class="mt-1 text-sm font-bold text-[var(--text-muted)]">{offlineCount}</p>
				</article>
				<article
					class="rounded-card border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-2"
					data-ui="host-stat-card"
					data-testid="host-stat-updates"
				>
					<p class="text-badge uppercase tracking-badge text-[var(--text-secondary)]">Updates pending</p>
					<p class="mt-1 text-sm font-bold text-[var(--color-info)]">{updatesPendingCount}</p>
				</article>
				<article
					class="rounded-card border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-2"
					data-ui="host-stat-card"
					data-testid="host-stat-errors"
				>
					<p class="text-badge uppercase tracking-badge text-[var(--text-secondary)]">Errors</p>
					<p class="mt-1 text-sm font-bold text-[var(--color-danger)]">{errorCount}</p>
				</article>
			</div>

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
							<th
								class="w-10 table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								scope="col"
							>
								<Checkbox
									id="hosts-batch-select-all"
									checked={allPageSelected}
									indeterminate={!allPageSelected && selectedIds.size > 0}
									onchange={toggleSelectAll}
									aria-label="Select all"
								/>
							</th>
						{/if}
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col">Name</th
						>
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col">Tags</th
						>
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col">Hostname</th
						>
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col">OS</th
						>
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col"
						>
							Architecture
						</th>
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col">IP</th
						>
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col">Software</th
						>
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col">Last Seen</th
						>
						{#if canManage}
							<th
								class="w-20 table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header sticky right-0 bg-[var(--bg-raised)]"
								scope="col"
							></th>
						{/if}
					</tr>
				{/snippet}
				{#snippet row(rowValue, _index)}
					{@const host = rowValue as unknown as HostResponse}
					{@const status = softwareStatus(host)}
					<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
						{#if canManage || canManageSoftware}
							<td class="table-cell-pad text-table-body">
								<Checkbox
									id="host-row-{host.id}"
									checked={selectedIds.has(host.id)}
									onchange={() => toggleSelect(host.id)}
									aria-label="Select {host.friendly_name}"
								/>
							</td>
						{/if}
						<td class="table-cell-pad text-table-body text-[var(--text-primary)]">
							<a href="/hosts/{host.id}" class="hover:underline font-medium">{host.friendly_name}</a>
						</td>
						<td class="table-cell-pad text-table-body text-[var(--text-primary)]">
							{#if host.tags && host.tags.length > 0}
								<div class="flex flex-wrap gap-1">
									{#each host.tags as tag (tag.id)}
										<TagBadge name={tag.name} color={tag.color} />
									{/each}
								</div>
							{:else}
								<span class="text-[var(--text-muted)]">&mdash;</span>
							{/if}
						</td>
						<td class="table-cell-pad text-table-body text-[var(--text-primary)]">{host.hostname}</td>
						<td class="table-cell-pad text-table-body text-[var(--text-primary)]"
							>{host.os_version ?? host.os_type ?? '\u2014'}</td
						>
						<td class="table-cell-pad text-table-body text-[var(--text-primary)]">{host.architecture ?? '\u2014'}</td>
						<td class="table-cell-pad text-table-body text-[var(--text-primary)]">{host.ip_address ?? '\u2014'}</td>
						<td class="table-cell-pad text-table-body text-[var(--text-primary)]">
							{#if status.update_count > 0}
								{#if canViewSoftware}
									<ActionBadge
										variant="navigation"
										tone="info"
										idleLabel={updateBadgeLabel(status.update_count)}
										hoverLabel="→ Software"
										onclick={() => openHostSoftware(host.id)}
									/>
								{:else}
									<StatusBadge tone="info" label={updateBadgeLabel(status.update_count)} />
								{/if}
							{:else if status.error_count > 0}
								{#if canViewSoftware}
									<ActionBadge
										variant="navigation"
										tone="danger"
										idleLabel={errorBadgeLabel(status.error_count)}
										hoverLabel="→ History"
										onclick={() => openHostHistory(host.id)}
									/>
								{:else}
									<StatusBadge tone="danger" label={errorBadgeLabel(status.error_count)} />
								{/if}
							{:else if status.known}
								<StatusBadge tone="success" label="Up to date" />
							{:else}
								<StatusBadge tone="neutral" label="Unknown" />
							{/if}
						</td>
						<td class="table-cell-pad text-table-body text-[var(--text-primary)]">{formatDate(host.last_seen_at)}</td>
						{#if canManage}
							<td class="table-cell-pad text-table-body sticky right-0 bg-[var(--bg-surface)]">
								<div class="actions-menu">
									<Button
										variant="ghost"
										size="sm"
										ariaLabel="Actions for {host.friendly_name}"
										onclick={(e: MouseEvent) => {
											e.stopPropagation();
											toggleMenu(host.id, e.currentTarget as HTMLElement);
										}}
									>
										{#snippet leadingIcon()}<span aria-hidden="true">&#8943;</span>{/snippet}
									</Button>
								</div>
							</td>
						{/if}
					</tr>
				{/snippet}
				{#snippet errorActions()}
					<Button variant="primary" loading={isRetrying} onclick={retryLoad}>Retry</Button>
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
			confirmLabel="Deactivate"
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
					<ContextMenuItem label="Edit Name" onclick={() => openEditDialog(host)} />
				</li>
				{#if canManageSoftware}
					<li>
						<ContextMenuItem
							label={discoveringHostIds.has(host.id) ? 'Triggering...' : 'Trigger Discovery'}
							disabled={discoveringHostIds.has(host.id)}
							onclick={() => triggerDiscovery(host)}
						/>
					</li>
				{/if}
				<li>
					<ContextMenuItem
						label="Deactivate"
						destructive
						onclick={() => requestConfirm(host.id, 'deactivate', host.friendly_name)}
					/>
				</li>
			</ContextMenuShell>
		{/if}
	{/if}

	{#if confirmAction}
		<ConfirmDialog
			title="Deactivate Host"
			messagePrefix="Are you sure you want to deactivate"
			entityName={confirmAction.name}
			confirmLabel="Deactivate"
			confirmDisabled={submitting}
			onconfirm={executeConfirmed}
			oncancel={cancelConfirm}
		/>
	{/if}

	{#if editHost}
		<ModalShell title="Edit Host Name" onclose={cancelEdit}>
			<FormFieldRow label="Friendly Name" inputId="host-edit-friendly-name">
				<Input id="host-edit-friendly-name" type="text" bind:value={editHost.friendlyName} />
			</FormFieldRow>
			{#snippet footer()}
				<Button variant="secondary" onclick={cancelEdit}>Cancel</Button>
				<Button variant="primary" loading={submitting} onclick={executeEdit}>Save</Button>
			{/snippet}
		</ModalShell>
	{/if}
{/if}
