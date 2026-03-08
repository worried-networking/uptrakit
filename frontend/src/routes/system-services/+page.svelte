<script lang="ts">
	import { onMount, onDestroy, untrack } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import {
		getSystemServices,
		approveSystemService,
		rejectSystemService,
		deleteSystemService,
		updateSystemService
	} from '$lib/api';
	import type { SystemServiceResponse } from '$lib/types';
	import { Permission } from '$lib/types';
	import { formatDate, parseUrlParam, parseUrlPage } from '$lib/utils';
	import { subscribeToEvent } from '$lib/stores/events.svelte';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import Modal from '$lib/components/Modal.svelte';
	import Pagination from '$lib/components/Pagination.svelte';

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

	const canView = $derived(getUser()?.permissions.includes(Permission.ViewSystemServices) ?? false);
	const canManage = $derived(getUser()?.permissions.includes(Permission.ManageSystemServices) ?? false);

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
				services = services.filter((s) => s.id !== serviceId);
			} else if (action === 'delete') {
				await deleteSystemService(serviceId);
				services = services.filter((s) => s.id !== serviceId);
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
	<h1 class="h1 mb-4">System Services</h1>

	<div class="mb-6 flex flex-wrap gap-2">
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
					<th>Name</th>
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
						<td>{service.friendly_name}</td>
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
						<td colspan={canManage ? 6 : 5} class="text-center py-8">
							<p class="text-lg font-medium">No system services registered yet</p>
							<p class="mt-1 text-sm text-surface-500">
								System services appear here when they enroll with the controller.
							</p>
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>

	<Pagination {currentPage} {totalPages} total={totalItems} onPageChange={loadServices} />

	{#if openMenuId}
		{@const service = services.find((s) => s.id === openMenuId)}
		{#if service}
			<ContextMenu top={menuPos.top} left={menuPos.left} onclose={closeMenu}>
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
				{:else if service.status !== 'deactivated'}
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
