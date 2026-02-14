<script lang="ts">
	import { onMount } from 'svelte';
	import { user } from '$lib/auth';
	import { getServices, approveService, rejectService, deleteService, mergeService } from '$lib/api';
	import type { ServiceResponse, ServiceType } from '$lib/types';
	import { Permission } from '$lib/types';
	import { formatDate } from '$lib/utils';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import ModalBackdrop from '$lib/components/ModalBackdrop.svelte';
	import Pagination from '$lib/components/Pagination.svelte';

	let services: ServiceResponse[] = $state([]);
	let error: string | null = $state(null);
	let openMenuId: string | null = $state(null);
	let menuPos: { top: number; left: number } = $state({ top: 0, left: 0 });
	let confirmAction: { serviceId: string; action: 'approve' | 'reject' | 'delete'; name: string } | null =
		$state(null);
	let mergeSource: { id: string; name: string; type: ServiceType } | null = $state(null);
	let mergeTargetId: string | null = $state(null);
	let submitting: boolean = $state(false);
	let currentPage: number = $state(1);
	let totalPages: number = $state(1);
	let typeFilter: 'all' | ServiceType = $state('all');

	const canManage = $derived($user?.permissions.includes(Permission.ManageAgents) ?? false);

	const filteredServices = $derived(
		typeFilter === 'all' ? services : services.filter((service) => service.service_type === typeFilter)
	);

	onMount(() => loadServices(1));

	async function loadServices(page: number) {
		try {
			error = null;
			const result = await getServices(undefined, page);
			services = result.items;
			currentPage = result.page;
			totalPages = result.total_pages;
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load services';
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

	function setFilter(filter: 'all' | ServiceType) {
		typeFilter = filter;
		closeMenu();
	}

	function requestConfirm(serviceId: string, action: 'approve' | 'reject' | 'delete', name: string) {
		closeMenu();
		confirmAction = { serviceId, action, name };
	}

	function cancelConfirm() {
		confirmAction = null;
	}

	function openMergeDialog(service: ServiceResponse) {
		closeMenu();
		mergeSource = { id: service.id, name: service.friendly_name, type: service.service_type };
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

	function formatServiceType(serviceType: ServiceType): string {
		return serviceType === 'agent' ? 'Agent' : 'MQTT Service';
	}
</script>

<svelte:window onclick={handleWindowClick} onkeydown={(e) => {
	if (e.key === 'Escape') {
		if (confirmAction) cancelConfirm();
		else if (mergeSource) cancelMerge();
	}
}} />

{#if $user}
	<h1 class="h1 mb-4">Services</h1>

	<div class="mb-6 flex flex-wrap gap-2">
		<button
			class="btn btn-sm {typeFilter === 'all' ? 'preset-filled-primary-500' : 'preset-tonal'}"
			onclick={() => setFilter('all')}
		>
			All Services
		</button>
		<button
			class="btn btn-sm {typeFilter === 'agent' ? 'preset-filled-primary-500' : 'preset-tonal'}"
			onclick={() => setFilter('agent')}
		>
			Agents
		</button>
		<button
			class="btn btn-sm {typeFilter === 'mqtt' ? 'preset-filled-primary-500' : 'preset-tonal'}"
			onclick={() => setFilter('mqtt')}
		>
			MQTT Services
		</button>
	</div>

	{#if error}
		<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
			<p>{error}</p>
		</aside>
	{/if}

	<div class="table-wrap">
		<table class="table">
			<thead>
				<tr>
					<th>Name</th>
					<th>Type</th>
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
				{#each filteredServices as service (service.id)}
					<tr>
						<td>{service.friendly_name}</td>
						<td>
							<span class="badge preset-tonal">{formatServiceType(service.service_type)}</span>
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
						<td colspan={canManage ? 7 : 6} class="text-center py-8">
							<p class="text-lg font-medium">No services registered yet</p>
							<p class="mt-1 text-sm text-surface-500">
								Agents and MQTT services appear here when they enroll with the controller.
							</p>
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>

	<Pagination {currentPage} {totalPages} onPageChange={loadServices} />

	{#if openMenuId}
		{@const service = services.find((s) => s.id === openMenuId)}
		{#if service}
			<ContextMenu top={menuPos.top} left={menuPos.left}>
				{#if service.status === 'pending'}
					<li>
						<button
							class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800"
							role="menuitem"
							onclick={() => openMergeDialog(service)}
						>
							Merge Into&hellip;
						</button>
					</li>
					<li>
						<button
							class="w-full rounded-md px-3 py-2 text-left text-sm text-success-500 hover:bg-surface-200 dark:hover:bg-surface-800"
							role="menuitem"
							onclick={() => requestConfirm(service.id, 'approve', service.friendly_name)}
						>
							Approve
						</button>
					</li>
					<li>
						<button
							class="w-full rounded-md px-3 py-2 text-left text-sm text-error-500 hover:bg-surface-200 dark:hover:bg-surface-800"
							role="menuitem"
							onclick={() => requestConfirm(service.id, 'reject', service.friendly_name)}
						>
							Reject
						</button>
					</li>
				{:else}
					<li>
						<button
							class="w-full rounded-md px-3 py-2 text-left text-sm text-error-500 hover:bg-surface-200 dark:hover:bg-surface-800"
							role="menuitem"
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
			onconfirm={executeConfirmed}
			oncancel={cancelConfirm}
		/>
	{/if}

	{#if mergeSource}
		<ModalBackdrop onclose={cancelMerge}>
			<div class="card bg-surface-50 dark:bg-surface-900 w-full max-w-md space-y-4 p-6 shadow-xl" role="dialog" aria-modal="true">
				<h3 class="h3">Merge Service</h3>
				<p>
					Merge <strong>{mergeSource.name}</strong> into an existing service.
					The source service's enrollment will be transferred to the target, preserving the target's history.
				</p>
				<label class="label">
					<span>Select target service</span>
					<select class="select" bind:value={mergeTargetId}>
						<option value={null}>-- Select a service --</option>
						{#each services.filter((s) => s.status === 'approved' && s.service_type === mergeSource?.type && s.id !== mergeSource?.id) as target (target.id)}
							<option value={target.id}>{target.friendly_name} ({target.hostname})</option>
						{/each}
					</select>
				</label>
				<div class="flex justify-end gap-2">
					<button class="btn preset-tonal-surface" onclick={cancelMerge}>Cancel</button>
					<button
						class="btn preset-filled-primary-500"
						disabled={!mergeTargetId || submitting}
						onclick={executeMerge}
					>
						{submitting ? 'Merging...' : 'Merge'}
					</button>
				</div>
			</div>
		</ModalBackdrop>
	{/if}
{/if}
