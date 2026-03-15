<script lang="ts">
	import { untrack } from 'svelte';
	import { SvelteSet, SvelteMap } from 'svelte/reactivity';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import {
		getServices,
		approveService,
		rejectService,
		deleteService,
		mergeService,
		updateService,
		batchServices,
		executeBatchChunked
	} from '$lib/api';
	import type { ServiceResponse, BatchActionResponse } from '$lib/types';
	import { Permission, hasAnyPermission } from '$lib/types';
	import { formatDate, parseUrlParam, parseUrlPage } from '$lib/utils';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import { subscribeToEvent } from '$lib/stores/events.svelte';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import Modal from '$lib/components/Modal.svelte';
	import Pagination from '$lib/components/Pagination.svelte';
	import BatchActionBar from '$lib/components/BatchActionBar.svelte';
	import BatchResultDialog from '$lib/components/BatchResultDialog.svelte';

	const CAPABILITY_FILTER_VALUES = ['all', 'software_discovery', 'ssh_remote'] as const;
	type CapabilityFilter = (typeof CAPABILITY_FILTER_VALUES)[number];

	let services: ServiceResponse[] = $state([]);
	let error: string | null = $state(null);
	let openMenuId: string | null = $state(null);
	let menuPos: { top: number; left: number } = $state({ top: 0, left: 0 });
	let confirmAction: {
		serviceId: string;
		action: 'approve' | 'reject' | 'delete';
		name: string;
		capabilities: string[];
	} | null = $state(null);
	let mergeSource: { id: string; name: string; capabilities: string[] } | null = $state(null);
	let mergeTargetId: string | null = $state(null);
	let editPingService: { id: string; name: string; pingInterval: string } | null = $state(null);
	let submitting: boolean = $state(false);
	let currentPage: number = $state(parseUrlPage(page.url));
	let totalPages: number = $state(1);
	let totalItems: number = $state(0);
	let capabilityFilter: CapabilityFilter = $state(
		parseUrlParam(page.url, 'capability', CAPABILITY_FILTER_VALUES, 'all')
	);

	let selectedIds = new SvelteSet<string>();
	const selectedItemsMap = new SvelteMap<string, ServiceResponse>();
	let batchConfirmAction: string | null = $state(null);
	let batchResult: BatchActionResponse | null = $state(null);
	let selectingAllPages = $state(false);

	const allPageSelected = $derived(services.length > 0 && services.every((s) => selectedIds.has(s.id)));

	const selectAllPagesInfo = $derived(
		allPageSelected && totalItems > services.length && selectedIds.size < totalItems
			? { total: totalItems, loading: selectingAllPages, onSelect: selectAllPages }
			: undefined
	);

	const canManage = $derived(
		hasAnyPermission(
			getUser(),
			Permission.ApproveServices,
			Permission.RejectServices,
			Permission.RemoveServices,
			Permission.UpdateServices
		)
	);

	const batchActions = $derived.by(() => {
		const acts: { id: string; label: string; destructive?: boolean }[] = [];
		const selected = [...selectedItemsMap.values()];
		if (selected.some((s) => s.status === 'pending')) {
			acts.push({ id: 'approve', label: 'Approve' });
			acts.push({ id: 'reject', label: 'Reject', destructive: true });
		}
		if (selected.some((s) => s.status !== 'deactivated')) {
			acts.push({ id: 'deactivate', label: 'Deactivate', destructive: true });
		}
		return acts;
	});

	$effect(() => {
		const parts: string[] = [];
		if (capabilityFilter !== 'all') parts.push(`capability=${capabilityFilter}`);
		if (currentPage > 1) parts.push(`page=${currentPage}`);
		const search = parts.join('&');
		goto(search ? `${location.pathname}?${search}` : location.pathname, {
			replaceState: true,
			keepFocus: true,
			noScroll: true
		});
	});

	$effect(() => {
		const _filter = capabilityFilter; // explicit dependency tracking
		loadServices(untrack(() => currentPage));

		const interval = setInterval(() => {
			if (document.visibilityState === 'visible') loadServices(currentPage, true);
		}, 300_000);
		const unsubStatusChanged = subscribeToEvent('service_status_changed', () => loadServices(currentPage, true));

		return () => {
			clearInterval(interval);
			unsubStatusChanged();
		};
	});

	async function loadServices(page: number, background = false) {
		try {
			if (!background) error = null;
			const result = await getServices({
				capability: capabilityFilter === 'all' ? undefined : capabilityFilter,
				page
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
				error = e instanceof Error ? e.message : 'Failed to load services';
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

	function setFilter(filter: CapabilityFilter) {
		selectedIds.clear();
		selectedItemsMap.clear();
		currentPage = 1;
		capabilityFilter = filter;
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

	function openMergeDialog(service: ServiceResponse) {
		closeMenu();
		mergeSource = { id: service.id, name: service.friendly_name, capabilities: service.capabilities };
		mergeTargetId = null;
	}

	function cancelMerge() {
		mergeSource = null;
		mergeTargetId = null;
	}

	async function executeMerge() {
		if (!mergeSource || !mergeTargetId || submitting) return;
		const sourceId = mergeSource.id;
		submitting = true;
		try {
			error = null;
			await mergeService(mergeTargetId, sourceId);
			services = services.filter((service) => service.id !== sourceId);
			mergeSource = null;
			mergeTargetId = null;
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to merge service';
		} finally {
			submitting = false;
		}
	}

	async function executeConfirmed() {
		if (!confirmAction || submitting) return;
		const { serviceId, action } = confirmAction;
		confirmAction = null;
		submitting = true;

		try {
			error = null;
			if (action === 'approve') {
				const updated = await approveService(serviceId);
				services = services.map((service) => (service.id === serviceId ? updated : service));
			} else if (action === 'reject') {
				await rejectService(serviceId);
				services = services.filter((service) => service.id !== serviceId);
			} else if (action === 'delete') {
				await deleteService(serviceId);
				services = services.filter((service) => service.id !== serviceId);
			}
		} catch (e) {
			error = e instanceof Error ? e.message : `Failed to ${action} service`;
		} finally {
			submitting = false;
		}
	}

	function openPingDialog(service: ServiceResponse) {
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
			const updated = await updateService(editPingService.id, {
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
			for (const s of services) {
				selectedIds.delete(s.id);
				selectedItemsMap.delete(s.id);
			}
		} else {
			for (const s of services) {
				selectedIds.add(s.id);
				selectedItemsMap.set(s.id, s);
			}
		}
	}

	function toggleSelect(id: string) {
		if (selectedIds.has(id)) {
			selectedIds.delete(id);
			selectedItemsMap.delete(id);
		} else {
			selectedIds.add(id);
			const service = services.find((s) => s.id === id);
			if (service) selectedItemsMap.set(id, service);
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
				const result = await getServices({
					capability: capabilityFilter === 'all' ? undefined : capabilityFilter,
					page: p,
					perPage: 100
				});
				for (const service of result.items) {
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
			const response = await executeBatchChunked(action, [...selectedIds], batchServices);
			if (response.failed.length > 0) {
				batchResult = response;
			} else {
				showSuccess(`${response.succeeded.length} service(s) ${action}d successfully.`);
			}
			selectedIds.clear();
			selectedItemsMap.clear();
			await loadServices(currentPage);
		} catch (e) {
			showError(e instanceof Error ? e.message : `Failed to ${action} services`);
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
		approve: { title: 'Approve Service', verb: 'approve', btnClass: 'preset-filled-success-500' },
		reject: { title: 'Reject Service', verb: 'reject', btnClass: 'preset-filled-error-500' },
		delete: { title: 'Delete Service', verb: 'permanently delete', btnClass: 'preset-filled-error-500' }
	} as const;
</script>

<svelte:window onclick={handleWindowClick} />

{#if getUser()}
	<h1 class="h1 mb-4">Services</h1>

	<div class="mb-6 flex flex-wrap gap-2">
		<button
			class="btn btn-sm {capabilityFilter === 'all' ? 'preset-filled-primary-500' : 'preset-tonal'}"
			onclick={() => setFilter('all')}
		>
			All Services
		</button>
		<button
			class="btn btn-sm {capabilityFilter === 'software_discovery' ? 'preset-filled-primary-500' : 'preset-tonal'}"
			onclick={() => setFilter('software_discovery')}
		>
			Agents
		</button>
		<button
			class="btn btn-sm {capabilityFilter === 'ssh_remote' ? 'preset-filled-primary-500' : 'preset-tonal'}"
			onclick={() => setFilter('ssh_remote')}
		>
			SSH Agents
		</button>
	</div>

	{#if error}
		<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
			<p>{error}</p>
			<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadServices(currentPage)}>Retry</button>
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
								checked={allPageSelected}
								indeterminate={!allPageSelected && selectedIds.size > 0}
								onchange={toggleSelectAll}
								aria-label="Select all"
							/>
						</th>
					{/if}
					<th>Name</th>
					<th>Label</th>
					<th>Hostname</th>
					<th>IP</th>
					<th>Status</th>
					<th>Last Seen</th>
					{#if canManage}
						<th class="w-20"></th>
					{/if}
				</tr>
			</thead>
			<tbody>
				{#each services as service (service.id)}
					<tr>
						{#if canManage}
							<td>
								<input
									type="checkbox"
									class="checkbox"
									checked={selectedIds.has(service.id)}
									onchange={() => toggleSelect(service.id)}
									aria-label="Select {service.friendly_name}"
								/>
							</td>
						{/if}
						<td>{service.friendly_name}</td>
						<td>
							<span class="badge preset-tonal">{service.service_label}</span>
						</td>
						<td>{service.hostname}</td>
						<td>{service.ip_address ?? '\u2014'}</td>
						<td>
							{#if service.status === 'pending'}
								<span class="badge preset-filled-warning-500">Pending</span>
							{:else if service.status === 'approved'}
								<span class="badge preset-filled-success-500">Approved</span>
							{:else if service.status === 'deactivated'}
								<span class="badge preset-tonal">Deactivated</span>
							{:else}
								<span class="badge preset-filled-error-500">Rejected</span>
							{/if}
						</td>
						<td>{formatDate(service.last_seen_at)}</td>
						{#if canManage}
							<td>
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
							</td>
						{/if}
					</tr>
				{:else}
					<tr>
						<td colspan={canManage ? 9 : 6} class="text-center py-8">
							<p class="text-lg font-medium">No services registered yet</p>
							<p class="mt-1 text-sm text-surface-500">
								Agents, MQTT services, SSH agents, and schedulers appear here when they enroll with the controller.
							</p>
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>

	<Pagination {currentPage} {totalPages} total={totalItems} onPageChange={loadServices} />

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
			entityName="{selectedIds.size} service(s)"
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
			<ContextMenu top={menuPos.top} left={menuPos.left} onclose={closeMenu}>
				{#if service.status === 'pending'}
					<li>
						<button
							class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800"
							role="menuitem"
							tabindex="-1"
							onclick={() => openMergeDialog(service)}
						>
							Merge Into&hellip;
						</button>
					</li>
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
				{:else}
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
			</ContextMenu>
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

	{#if mergeSource}
		<Modal title="Merge Service" onclose={cancelMerge}>
			<p>
				Merge <strong>{mergeSource.name}</strong> into an existing service. The source service's enrollment will be transferred
				to the target, preserving the target's history.
			</p>
			<label class="label">
				<span>Select target service</span>
				<select class="select" bind:value={mergeTargetId}>
					<option value={null}>-- Select a service --</option>
					{#each services.filter((s) => s.status === 'approved' && s.capabilities.includes('software_discovery') && s.id !== mergeSource?.id) as target (target.id)}
						<option value={target.id}>{target.friendly_name} ({target.hostname})</option>
					{/each}
				</select>
			</label>
			{#snippet footer()}
				<button class="btn preset-tonal-surface" onclick={cancelMerge}>Cancel</button>
				<button class="btn preset-filled-primary-500" disabled={!mergeTargetId || submitting} onclick={executeMerge}>
					{submitting ? 'Merging...' : 'Merge'}
				</button>
			{/snippet}
		</Modal>
	{/if}

	{#if editPingService}
		<Modal title="Edit Ping Interval" onclose={cancelPingEdit}>
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
		</Modal>
	{/if}
{/if}
