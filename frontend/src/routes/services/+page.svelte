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
	import { formatDate, parseUrlParam, parseUrlPage, nextValidPage } from '$lib/utils';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import { subscribeToEvent } from '$lib/stores/events.svelte';
	import Button from '$lib/components/Button.svelte';
	import EllipsisIcon from '$lib/components/icons/EllipsisIcon.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import BatchActionBar from '$lib/components/BatchActionBar.svelte';
	import BatchResultDialog from '$lib/components/BatchResultDialog.svelte';
	import {
		ContextMenuItem,
		ContextMenuShell,
		DataTable,
		FormFieldRow,
		ModalShell,
		PageShell,
		SectionCard,
		StatusBadge,
		TableFooterBar
	} from '$lib/components/ui';
	import Input from '$lib/components/Input.svelte';
	import Checkbox from '$lib/components/Checkbox.svelte';

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
	let isRetrying: boolean = $state(false);
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

	function canSelect(service: ServiceResponse) {
		return !service.is_embedded;
	}

	function hasActions(service: ServiceResponse) {
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
		if (selected.some((s) => s.status !== 'deactivated' && !s.is_embedded)) {
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

	async function loadServices(page: number, background = false, retry = false) {
		try {
			if (!background && !retry) error = null;
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
			error = null;
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
			mergeSource = null;
			mergeTargetId = null;
			await loadServices(currentPage);
			const p = nextValidPage(currentPage, totalPages);
			if (p !== null) await loadServices(p);
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
				await loadServices(currentPage);
				const p = nextValidPage(currentPage, totalPages);
				if (p !== null) await loadServices(p);
			} else if (action === 'delete') {
				await deleteService(serviceId);
				await loadServices(currentPage);
				const p = nextValidPage(currentPage, totalPages);
				if (p !== null) await loadServices(p);
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
				const result = await getServices({
					capability: capabilityFilter === 'all' ? undefined : capabilityFilter,
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
			const response = await executeBatchChunked(action, [...selectedIds], batchServices);
			if (response.failed.length > 0) {
				batchResult = response;
			} else {
				showSuccess(`${response.succeeded.length} service(s) ${action}d successfully.`);
			}
			selectedIds.clear();
			selectedItemsMap.clear();
			await loadServices(currentPage);
			const p = nextValidPage(currentPage, totalPages);
			if (p !== null) await loadServices(p);
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
		approve: { title: 'Approve Service', verb: 'approve', confirmVariant: 'primary' as const },
		reject: { title: 'Reject Service', verb: 'reject', confirmVariant: 'danger' as const },
		delete: { title: 'Delete Service', verb: 'permanently delete', confirmVariant: 'danger' as const }
	} as const;
</script>

<svelte:window onclick={handleWindowClick} />

{#if getUser()}
	<PageShell title="Services" description="Review enrolled runtime services and manage approvals.">
		<SectionCard title="Service Filters">
			<div class="flex flex-wrap gap-2">
				<Button variant={capabilityFilter === 'all' ? 'accent' : 'ghost'} size="sm" onclick={() => setFilter('all')}>
					All Services
				</Button>
				<Button
					variant={capabilityFilter === 'software_discovery' ? 'accent' : 'ghost'}
					size="sm"
					onclick={() => setFilter('software_discovery')}
				>
					Agents
				</Button>
				<Button
					variant={capabilityFilter === 'ssh_remote' ? 'accent' : 'ghost'}
					size="sm"
					onclick={() => setFilter('ssh_remote')}
				>
					SSH Agents
				</Button>
			</div>
		</SectionCard>

		<SectionCard title="Registered Services">
			<DataTable
				columns={[]}
				rows={services as unknown as Record<string, unknown>[]}
				{error}
				emptyTitle="No services registered yet"
				emptyDescription="Agents, MQTT services, SSH agents, and schedulers appear here when they enroll with the controller."
				rowKey={(row) => (row as unknown as ServiceResponse).id}
			>
				{#snippet header()}
					<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
						{#if canManage}
							<th
								class="w-10 table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								scope="col"
							>
								<Checkbox
									id="services-batch-select-all"
									checked={allPageSelected}
									indeterminate={!allPageSelected && selectedIds.size > 0}
									disabled={selectableServices.length === 0}
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
							scope="col">Label</th
						>
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col"
						>
							Hostname
						</th>
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col">IP</th
						>
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col">Status</th
						>
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col"
						>
							Last Seen
						</th>
						{#if canManage}
							<th
								class="w-20 table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								scope="col"
							></th>
						{/if}
					</tr>
				{/snippet}
				{#snippet row(rowValue, _index)}
					{@const service = rowValue as unknown as ServiceResponse}
					<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
						{#if canManage}
							<td class="table-cell-pad">
								{#if canSelect(service)}
									<Checkbox
										id="service-row-{service.id}"
										checked={selectedIds.has(service.id)}
										onchange={() => toggleSelect(service.id)}
										aria-label="Select {service.friendly_name}"
									/>
								{:else}
									<span class="text-[var(--text-muted)]">-</span>
								{/if}
							</td>
						{/if}
						<td class="table-cell-pad text-[var(--text-primary)]">{service.friendly_name}</td>
						<td class="table-cell-pad text-[var(--text-primary)]">
							<StatusBadge tone="neutral" label={service.service_label} />
						</td>
						<td class="table-cell-pad text-[var(--text-primary)]">{service.hostname}</td>
						<td class="table-cell-pad text-[var(--text-primary)]">{service.ip_address ?? '\u2014'}</td>
						<td class="table-cell-pad text-[var(--text-primary)]">
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
						<td class="table-cell-pad text-[var(--text-primary)]">{formatDate(service.last_seen_at)}</td>
						{#if canManage}
							<td class="table-cell-pad">
								{#if hasActions(service)}
									<div class="actions-menu">
										<Button
											variant="ghost"
											size="sm"
											ariaLabel="Actions for {service.friendly_name}"
											onclick={(e) => {
												e.stopPropagation();
												toggleMenu(service.id, e.currentTarget);
											}}
										>
											{#snippet leadingIcon()}<EllipsisIcon />{/snippet}
											<span class="sr-only">Actions for {service.friendly_name}</span>
										</Button>
									</div>
								{/if}
							</td>
						{/if}
					</tr>
				{/snippet}
				{#snippet errorActions()}
					<Button
						variant="primary"
						class="mt-3"
						loading={isRetrying}
						onclick={async () => {
							isRetrying = true;
							try {
								await loadServices(currentPage, false, true);
							} finally {
								isRetrying = false;
							}
						}}
					>
						Retry
					</Button>
				{/snippet}
				{#snippet footer()}
					{#if !error}
						<TableFooterBar {currentPage} {totalPages} total={totalItems} onPageChange={loadServices} />
					{/if}
				{/snippet}
			</DataTable>
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
			entityName="{selectedIds.size} service(s)"
			confirmLabel={submitting
				? 'Processing...'
				: batchConfirmAction === 'approve'
					? 'Approve'
					: batchConfirmAction === 'reject'
						? 'Reject'
						: 'Deactivate'}
			confirmVariant={batchConfirmAction === 'approve' ? 'primary' : 'danger'}
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
						<ContextMenuItem label="Merge Into..." onclick={() => openMergeDialog(service)} />
					</li>
					<li>
						<ContextMenuItem
							label="Approve"
							onclick={() => requestConfirm(service.id, 'approve', service.friendly_name, service.capabilities)}
						/>
					</li>
					<li>
						<ContextMenuItem
							label="Reject"
							destructive
							onclick={() => requestConfirm(service.id, 'reject', service.friendly_name)}
						/>
					</li>
				{:else}
					<li>
						<ContextMenuItem label="Edit Ping Interval" onclick={() => openPingDialog(service)} />
					</li>
					{#if !service.is_embedded}
						<li>
							<ContextMenuItem
								label="Delete"
								destructive
								onclick={() => requestConfirm(service.id, 'delete', service.friendly_name)}
							/>
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
			confirmVariant={labels.confirmVariant}
			confirmDisabled={submitting}
			warnings={confirmAction.action === 'approve' ? getCredentialWarnings(confirmAction.capabilities) : []}
			onconfirm={executeConfirmed}
			oncancel={cancelConfirm}
		/>
	{/if}

	{#if mergeSource}
		<ModalShell title="Merge Service" onclose={cancelMerge}>
			<p>
				Merge <strong>{mergeSource.name}</strong> into an existing service. The source service's enrollment will be transferred
				to the target, preserving the target's history.
			</p>
			<FormFieldRow label="Select target service">
				<select class="select" bind:value={mergeTargetId}>
					<option value={null}>-- Select a service --</option>
					{#each services.filter((s) => s.status === 'approved' && s.capabilities.includes('software_discovery') && s.id !== mergeSource?.id) as target (target.id)}
						<option value={target.id}>{target.friendly_name} ({target.hostname})</option>
					{/each}
				</select>
			</FormFieldRow>
			{#snippet footer()}
				<Button variant="secondary" onclick={cancelMerge}>Cancel</Button>
				<Button variant="primary" loading={submitting} disabled={!mergeTargetId} onclick={executeMerge}>Merge</Button>
			{/snippet}
		</ModalShell>
	{/if}

	{#if editPingService}
		<ModalShell title="Edit Ping Interval" onclose={cancelPingEdit}>
			<p>
				Set a custom ping interval for <strong>{editPingService.name}</strong>. Leave empty to use the service-profile
				default.
			</p>
			<FormFieldRow label="Ping interval (seconds)">
				<Input
					id="services-ping-interval"
					type="number"
					min="0"
					placeholder="Default"
					class="w-full"
					bind:value={editPingService.pingInterval}
				/>
			</FormFieldRow>
			{#snippet footer()}
				<Button variant="secondary" onclick={cancelPingEdit}>Cancel</Button>
				<Button variant="primary" loading={submitting} onclick={executePingEdit}>Save</Button>
			{/snippet}
		</ModalShell>
	{/if}
{/if}
