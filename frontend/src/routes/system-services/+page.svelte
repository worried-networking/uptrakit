<script lang="ts">
	import { onMount, onDestroy, untrack } from 'svelte';
	import { SvelteSet, SvelteMap } from 'svelte/reactivity';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import {
		getSystemServices,
		approveSystemService,
		rejectSystemService,
		deleteSystemService,
		updateSystemService,
		batchSystemServices,
		executeBatchChunked
	} from '$lib/api';
	import type { SystemServiceResponse, BatchActionResponse } from '$lib/types';
	import { Permission, hasAnyPermission } from '$lib/types';
	import { formatDate, parseUrlParam, parseUrlPage, nextValidPage } from '$lib/utils';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import { subscribeToEvent } from '$lib/stores/events.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import Pagination from '$lib/components/Pagination.svelte';
	import BatchActionBar from '$lib/components/BatchActionBar.svelte';
	import BatchResultDialog from '$lib/components/BatchResultDialog.svelte';
	import { ContextMenuShell, DataTable, ModalShell, PageShell, SectionCard, StatusBadge } from '$lib/components/ui';

	const STATUS_FILTER_VALUES = ['all', 'pending', 'approved', 'rejected', 'deactivated'] as const;
	type StatusFilter = (typeof STATUS_FILTER_VALUES)[number];

	let services: SystemServiceResponse[] = $state([]);
	let error: string | null = $state(null);
	let openMenuId: string | null = $state(null);
	let menuPos: { top: number; left: number } = $state({ top: 0, left: 0 });
	let confirmAction: {
		serviceId: string;
		action: 'approve' | 'reject' | 'delete';
		name: string;
		capabilities: string[];
	} | null = $state(null);
	let editPingService: { id: string; name: string; pingInterval: string } | null = $state(null);
	let submitting: boolean = $state(false);
	let currentPage: number = $state(parseUrlPage(page.url));
	let totalPages: number = $state(1);
	let totalItems: number = $state(0);
	let statusFilter: StatusFilter = $state(parseUrlParam(page.url, 'status', STATUS_FILTER_VALUES, 'all'));

	let selectedIds = new SvelteSet<string>();
	const selectedItemsMap = new SvelteMap<string, SystemServiceResponse>();
	let batchConfirmAction: string | null = $state(null);
	let batchResult: BatchActionResponse | null = $state(null);
	let selectingAllPages = $state(false);

	function canSelect(service: SystemServiceResponse) {
		return !service.is_embedded;
	}

	function hasActions(service: SystemServiceResponse) {
		if (service.status === 'pending') return true;
		if (service.status === 'deactivated') return false;
		if (service.is_embedded) return service.status === 'approved';
		return true;
	}

	const selectableServices = $derived(services.filter(canSelect));

	const allPageSelected = $derived(
		selectableServices.length > 0 && selectableServices.every((s) => selectedIds.has(s.id))
	);

	const selectAllPagesInfo = $derived(
		allPageSelected && totalItems > services.length && selectedIds.size < totalItems
			? { total: totalItems, loading: selectingAllPages, onSelect: selectAllPages }
			: undefined
	);

	const canView = $derived(getUser()?.permissions.includes(Permission.ViewSystemServices) ?? false);
	const canManage = $derived(
		hasAnyPermission(
			getUser(),
			Permission.ApproveSystemServices,
			Permission.RejectSystemServices,
			Permission.RemoveSystemServices,
			Permission.UpdateSystemServices
		)
	);

	const batchActions = $derived.by(() => {
		const acts: { id: string; label: string; destructive?: boolean }[] = [];
		const selected = [...selectedItemsMap.values()];
		if (selected.some((s) => s.status === 'pending')) {
			acts.push({ id: 'approve', label: 'Approve' });
			acts.push({ id: 'reject', label: 'Reject', destructive: true });
		}
		if (selected.some((s) => s.status !== 'deactivated' && !s.is_embedded)) {
			acts.push({ id: 'deactivate', label: 'Deactivate', destructive: true });
		}
		return acts;
	});

	let refreshInterval: ReturnType<typeof setInterval> | null = null;
	let unsubscribers: (() => void)[] = [];

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

	$effect(() => {
		const _filter = statusFilter; // explicit dependency tracking
		loadServices(untrack(() => currentPage));

		refreshInterval = setInterval(() => {
			if (document.visibilityState === 'visible') loadServices(currentPage, true);
		}, 300_000);

		return () => {
			if (refreshInterval) clearInterval(refreshInterval);
		};
	});

	onMount(() => {
		unsubscribers.push(subscribeToEvent('system_service_status_changed', () => loadServices(currentPage, true)));
	});

	onDestroy(() => {
		for (const unsub of unsubscribers) unsub();
		if (refreshInterval) clearInterval(refreshInterval);
	});

	async function loadServices(p: number, background = false) {
		try {
			if (!background) error = null;
			const result = await getSystemServices({
				status: statusFilter === 'all' ? undefined : statusFilter,
				page: p
			});
			services = result.items;
			for (const service of result.items) {
				if (selectedIds.has(service.id)) selectedItemsMap.set(service.id, service);
			}
			currentPage = result.page;
			totalPages = result.total_pages;
			totalItems = result.total;
			if (background) error = null;
		} catch (e) {
			if (!background) {
				error = e instanceof Error ? e.message : 'Failed to load system services';
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

	function setFilter(filter: StatusFilter) {
		selectedIds.clear();
		selectedItemsMap.clear();
		currentPage = 1;
		statusFilter = filter;
		closeMenu();
	}

	function requestConfirm(
		serviceId: string,
		action: 'approve' | 'reject' | 'delete',
		name: string,
		capabilities: string[] = []
	) {
		closeMenu();
		confirmAction = { serviceId, action, name, capabilities };
	}

	function getCredentialWarnings(capabilities: string[]): string[] {
		const warnings: string[] = [];
		if (capabilities.includes('database_access')) {
			warnings.push('This service will receive direct database access credentials.');
		}
		if (capabilities.includes('master_key_access')) {
			warnings.push('This service will receive the master encryption key.');
		}
		if (capabilities.includes('nats_access')) {
			warnings.push('This service will receive NATS connection details.');
		}
		if (capabilities.includes('ca_management')) {
			warnings.push('This service can request CA certificate rotation.');
		}
		return warnings;
	}

	function cancelConfirm() {
		confirmAction = null;
	}

	async function executeConfirmed() {
		if (!confirmAction || submitting) return;
		const { serviceId, action } = confirmAction;
		confirmAction = null;
		submitting = true;

		try {
			error = null;
			if (action === 'approve') {
				const updated = await approveSystemService(serviceId);
				services = services.map((s) => (s.id === serviceId ? updated : s));
			} else if (action === 'reject') {
				await rejectSystemService(serviceId);
				await loadServices(currentPage);
				const p = nextValidPage(currentPage, totalPages);
				if (p !== null) await loadServices(p);
			} else if (action === 'delete') {
				await deleteSystemService(serviceId);
				await loadServices(currentPage);
				const p = nextValidPage(currentPage, totalPages);
				if (p !== null) await loadServices(p);
			}
		} catch (e) {
			error = e instanceof Error ? e.message : `Failed to ${action} system service`;
		} finally {
			submitting = false;
		}
	}

	function openPingDialog(service: SystemServiceResponse) {
		closeMenu();
		editPingService = {
			id: service.id,
			name: service.friendly_name,
			pingInterval: service.ping_interval_seconds != null ? String(service.ping_interval_seconds) : ''
		};
	}

	function cancelPingEdit() {
		editPingService = null;
	}

	async function executePingEdit() {
		if (!editPingService || submitting) return;
		const val = editPingService.pingInterval.trim();
		const seconds = val === '' ? 0 : parseInt(val, 10);
		if (val !== '' && (isNaN(seconds) || seconds < 0)) {
			error = 'Ping interval must be a positive number or empty to clear.';
			return;
		}
		submitting = true;
		try {
			error = null;
			const updated = await updateSystemService(editPingService.id, {
				ping_interval_seconds: val === '' ? 0 : seconds
			});
			services = services.map((s) => (s.id === updated.id ? updated : s));
			editPingService = null;
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to update ping interval';
		} finally {
			submitting = false;
		}
	}

	function toggleSelectAll() {
		if (allPageSelected) {
			for (const s of selectableServices) {
				selectedIds.delete(s.id);
				selectedItemsMap.delete(s.id);
			}
		} else {
			for (const s of selectableServices) {
				selectedIds.add(s.id);
				selectedItemsMap.set(s.id, s);
			}
		}
	}

	function toggleSelect(id: string) {
		const service = services.find((s) => s.id === id);
		if (!service || !canSelect(service)) return;
		if (selectedIds.has(id)) {
			selectedIds.delete(id);
			selectedItemsMap.delete(id);
		} else {
			selectedIds.add(id);
			selectedItemsMap.set(id, service);
		}
	}

	function requestBatchAction(actionId: string) {
		batchConfirmAction = actionId;
	}

	async function selectAllPages() {
		selectingAllPages = true;
		try {
			let p = 1;
			while (true) {
				const result = await getSystemServices({
					status: statusFilter === 'all' ? undefined : statusFilter,
					page: p,
					perPage: 100
				});
				for (const service of result.items) {
					if (!canSelect(service)) continue;
					selectedIds.add(service.id);
					selectedItemsMap.set(service.id, service);
				}
				if (p >= result.total_pages) break;
				p++;
			}
		} catch {
			showError('Failed to select all items');
		} finally {
			selectingAllPages = false;
		}
	}

	async function executeBatchAction() {
		if (!batchConfirmAction || submitting) return;
		const action = batchConfirmAction;
		batchConfirmAction = null;
		submitting = true;
		try {
			const response = await executeBatchChunked(action, [...selectedIds], batchSystemServices);
			if (response.failed.length > 0) {
				batchResult = response;
			} else {
				showSuccess(`${response.succeeded.length} system service(s) ${action}d successfully.`);
			}
			selectedIds.clear();
			selectedItemsMap.clear();
			await loadServices(currentPage);
			const p = nextValidPage(currentPage, totalPages);
			if (p !== null) await loadServices(p);
		} catch (e) {
			showError(e instanceof Error ? e.message : `Failed to ${action} system services`);
		} finally {
			submitting = false;
		}
	}

	function handleWindowClick(event: MouseEvent) {
		if (openMenuId && !(event.target as HTMLElement).closest('.actions-menu')) {
			closeMenu();
		}
	}

	const confirmLabels = {
		approve: { title: 'Approve System Service', verb: 'approve', btnClass: 'preset-filled-success-500' },
		reject: { title: 'Reject System Service', verb: 'reject', btnClass: 'preset-filled-error-500' },
		delete: { title: 'Delete System Service', verb: 'permanently delete', btnClass: 'preset-filled-error-500' }
	} as const;
</script>

<svelte:window onclick={handleWindowClick} />

{#if canView}
	<PageShell title="System Services" description="Manage scheduler and system-level service enrollment.">
		<SectionCard title="Status Filters">
			<div class="flex flex-wrap gap-2">
				<button
					class="btn btn-sm {statusFilter === 'all' ? 'preset-filled-primary-500' : 'preset-tonal'}"
					onclick={() => setFilter('all')}
				>
					All
				</button>
				<button
					class="btn btn-sm {statusFilter === 'pending' ? 'preset-filled-primary-500' : 'preset-tonal'}"
					onclick={() => setFilter('pending')}
				>
					Pending
				</button>
				<button
					class="btn btn-sm {statusFilter === 'approved' ? 'preset-filled-primary-500' : 'preset-tonal'}"
					onclick={() => setFilter('approved')}
				>
					Approved
				</button>
				<button
					class="btn btn-sm {statusFilter === 'rejected' ? 'preset-filled-primary-500' : 'preset-tonal'}"
					onclick={() => setFilter('rejected')}
				>
					Rejected
				</button>
				<button
					class="btn btn-sm {statusFilter === 'deactivated' ? 'preset-filled-primary-500' : 'preset-tonal'}"
					onclick={() => setFilter('deactivated')}
				>
					Deactivated
				</button>
			</div>
		</SectionCard>

		<SectionCard title="Registered System Services">
			<DataTable
				columns={[]}
				rows={services as unknown as Record<string, unknown>[]}
				{error}
				emptyTitle="No system services registered yet"
				emptyDescription="System services appear here when they enroll with the controller."
				rowKey={(row) => (row as unknown as SystemServiceResponse).id}
			>
				{#snippet header()}
					<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
						{#if canManage}
							<th class="w-10 px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">
								<input
									type="checkbox"
									class="checkbox"
									checked={allPageSelected}
									indeterminate={!allPageSelected && selectedIds.size > 0}
									disabled={selectableServices.length === 0}
									onchange={toggleSelectAll}
									aria-label="Select all"
								/>
							</th>
						{/if}
						<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">Name</th>
						<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">
							Hostname
						</th>
						<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">IP</th>
						<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">Status</th>
						<th class="px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col">
							Last Seen
						</th>
						{#if canManage}
							<th class="w-20 px-4 py-3 text-left text-xs font-semibold uppercase tracking-[0.14em]" scope="col"></th>
						{/if}
					</tr>
				{/snippet}
				{#snippet row(rowValue)}
					{@const service = rowValue as unknown as SystemServiceResponse}
					<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
						{#if canManage}
							<td class="px-4 py-3">
								{#if canSelect(service)}
									<input
										type="checkbox"
										class="checkbox"
										checked={selectedIds.has(service.id)}
										onchange={() => toggleSelect(service.id)}
										aria-label="Select {service.friendly_name}"
									/>
								{:else}
									<span class="text-surface-500">-</span>
								{/if}
							</td>
						{/if}
						<td class="px-4 py-3 text-[var(--text-primary)]">{service.friendly_name}</td>
						<td class="px-4 py-3 text-[var(--text-primary)]">{service.hostname}</td>
						<td class="px-4 py-3 text-[var(--text-primary)]">{service.ip_address ?? '\u2014'}</td>
						<td class="px-4 py-3 text-[var(--text-primary)]">
							<div class="flex flex-col items-start gap-1.5" data-ui="status-badge-stack">
								{#if service.status === 'pending'}
									<StatusBadge tone="warning" label="Pending" />
								{:else if service.status === 'approved'}
									<StatusBadge tone="success" label="Approved" />
								{:else if service.status === 'deactivated'}
									<StatusBadge tone="neutral" label="Deactivated" />
								{:else}
									<StatusBadge tone="danger" label="Rejected" />
								{/if}
								{#if service.is_embedded}
									<StatusBadge tone="neutral" label="Embedded" />
								{/if}
								{#if service.yielded_to && service.yielded_to.length > 0}
									<StatusBadge tone="warning" label={`Yielded (${service.yielded_to.length})`} />
								{/if}
							</div>
						</td>
						<td class="px-4 py-3 text-[var(--text-primary)]">{formatDate(service.last_seen_at)}</td>
						{#if canManage}
							<td class="px-4 py-3">
								{#if hasActions(service)}
									<div class="actions-menu">
										<button
											class="btn btn-sm preset-tonal"
											aria-label="Actions for {service.friendly_name}"
											onclick={(e) => {
												e.stopPropagation();
												toggleMenu(service.id, e.currentTarget);
											}}
										>
											&#8943;
										</button>
									</div>
								{/if}
							</td>
						{/if}
					</tr>
				{/snippet}
				{#snippet errorActions()}
					<button class="btn preset-filled-primary-500 mt-3" onclick={() => loadServices(currentPage)}>Retry</button>
				{/snippet}
			</DataTable>

			{#if !error}
				<div class="mt-4">
					<Pagination {currentPage} {totalPages} total={totalItems} onPageChange={loadServices} />
				</div>
			{/if}
		</SectionCard>
	</PageShell>

	{#if canManage && selectedIds.size > 0}
		<BatchActionBar
			selectedCount={selectedIds.size}
			actions={batchActions}
			onaction={requestBatchAction}
			oncancel={() => {
				selectedIds.clear();
				selectedItemsMap.clear();
			}}
			selectAllPages={selectAllPagesInfo}
		/>
	{/if}

	{#if batchConfirmAction}
		<ConfirmDialog
			title="Batch {batchConfirmAction}"
			messagePrefix="Are you sure you want to {batchConfirmAction}"
			entityName="{selectedIds.size} system service(s)"
			confirmLabel={submitting
				? 'Processing...'
				: batchConfirmAction === 'approve'
					? 'Approve'
					: batchConfirmAction === 'reject'
						? 'Reject'
						: 'Deactivate'}
			confirmClass={batchConfirmAction === 'approve' ? 'preset-filled-success-500' : 'preset-filled-error-500'}
			confirmDisabled={submitting}
			onconfirm={executeBatchAction}
			oncancel={() => (batchConfirmAction = null)}
		/>
	{/if}

	{#if batchResult}
		<BatchResultDialog title="Batch Action Results" response={batchResult} onclose={() => (batchResult = null)} />
	{/if}

	{#if openMenuId}
		{@const service = services.find((s) => s.id === openMenuId)}
		{#if service}
			<ContextMenuShell top={menuPos.top} left={menuPos.left} onclose={closeMenu}>
				{#if service.status === 'pending'}
					<li>
						<button
							class="w-full rounded-md px-3 py-2 text-left text-sm text-success-500 hover:bg-surface-200 dark:hover:bg-surface-800"
							role="menuitem"
							tabindex="-1"
							onclick={() => requestConfirm(service.id, 'approve', service.friendly_name, service.capabilities)}
						>
							Approve
						</button>
					</li>
					<li>
						<button
							class="w-full rounded-md px-3 py-2 text-left text-sm text-error-500 hover:bg-surface-200 dark:hover:bg-surface-800"
							role="menuitem"
							tabindex="-1"
							onclick={() => requestConfirm(service.id, 'reject', service.friendly_name)}
						>
							Reject
						</button>
					</li>
				{:else if service.status === 'approved'}
					<li>
						<button
							class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800"
							role="menuitem"
							tabindex="-1"
							onclick={() => openPingDialog(service)}
						>
							Edit Ping Interval
						</button>
					</li>
					{#if !service.is_embedded}
						<li>
							<button
								class="w-full rounded-md px-3 py-2 text-left text-sm text-error-500 hover:bg-surface-200 dark:hover:bg-surface-800"
								role="menuitem"
								tabindex="-1"
								onclick={() => requestConfirm(service.id, 'delete', service.friendly_name)}
							>
								Delete
							</button>
						</li>
					{/if}
				{:else if service.status !== 'deactivated'}
					{#if !service.is_embedded}
						<li>
							<button
								class="w-full rounded-md px-3 py-2 text-left text-sm text-error-500 hover:bg-surface-200 dark:hover:bg-surface-800"
								role="menuitem"
								tabindex="-1"
								onclick={() => requestConfirm(service.id, 'delete', service.friendly_name)}
							>
								Delete
							</button>
						</li>
					{/if}
				{/if}
			</ContextMenuShell>
		{/if}
	{/if}

	{#if confirmAction}
		{@const labels = confirmLabels[confirmAction.action]}
		<ConfirmDialog
			title={labels.title}
			messagePrefix="Are you sure you want to {labels.verb}"
			entityName={confirmAction.name}
			confirmLabel={submitting ? 'Processing...' : labels.title}
			confirmClass={labels.btnClass}
			confirmDisabled={submitting}
			warnings={confirmAction.action === 'approve' ? getCredentialWarnings(confirmAction.capabilities) : []}
			onconfirm={executeConfirmed}
			oncancel={cancelConfirm}
		/>
	{/if}

	{#if editPingService}
		<ModalShell title="Edit Ping Interval" onclose={cancelPingEdit}>
			<p>
				Set a custom ping interval for <strong>{editPingService.name}</strong>. Leave empty to use the service-profile
				default.
			</p>
			<label class="label">
				<span>Ping interval (seconds)</span>
				<input class="input" type="number" min="0" placeholder="Default" bind:value={editPingService.pingInterval} />
			</label>
			{#snippet footer()}
				<button class="btn preset-tonal-surface" onclick={cancelPingEdit}>Cancel</button>
				<button class="btn preset-filled-primary-500" disabled={submitting} onclick={executePingEdit}>
					{submitting ? 'Saving...' : 'Save'}
				</button>
			{/snippet}
		</ModalShell>
	{/if}
{/if}
