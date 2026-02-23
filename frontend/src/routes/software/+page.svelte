<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { getUser } from '$lib/auth.svelte';
	import { getSoftwareItems, deleteSoftwareItem, approveSoftwareItem, checkSoftwareItemVersions } from '$lib/api';
	import { formatDate } from '$lib/utils';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import Pagination from '$lib/components/Pagination.svelte';
	import AddSoftwareModal from '$lib/components/AddSoftwareModal.svelte';
	import AssignToHostModal from '$lib/components/AssignToHostModal.svelte';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import type { SoftwareItemResponse } from '$lib/types';
	import { Permission } from '$lib/types';

	type FilterTab = 'all' | 'pending' | 'active';

	let items: SoftwareItemResponse[] = $state([]);
	let error: string | null = $state(null);
	let currentPage: number = $state(1);
	let totalPages: number = $state(1);
	let totalItems: number = $state(0);
	let loading: boolean = $state(false);
	let showAddModal: boolean = $state(false);
	let openMenuId: string | null = $state(null);
	let menuPos: { top: number; left: number } = $state({ top: 0, left: 0 });
	let confirmDelete: { id: string; name: string } | null = $state(null);
	let assignItem: { id: string; name: string } | null = $state(null);
	let submitting: boolean = $state(false);
	let checkingVersionsId: string | null = $state(null);
	let approvingId: string | null = $state(null);
	let activeTab: FilterTab = $state('all');

	const canView = $derived(getUser()?.permissions.includes(Permission.ViewSoftware) ?? false);
	const canManage = $derived(getUser()?.permissions.includes(Permission.ManageSoftware) ?? false);

	let refreshInterval: ReturnType<typeof setInterval> | null = null;

	onMount(() => {
		if (canView) {
			loadAll(1);
			refreshInterval = setInterval(() => {
				if (document.visibilityState === 'visible') loadAll(currentPage, true);
			}, 60_000);
		}
	});

	onDestroy(() => {
		if (refreshInterval) clearInterval(refreshInterval);
	});

	function discoveryStateFilter(): 'pending' | 'approved' | undefined {
		if (activeTab === 'pending') return 'pending';
		if (activeTab === 'active') return 'approved';
		return undefined;
	}

	async function loadAll(page: number, background = false) {
		if (!background) {
			loading = true;
			error = null;
		}
		try {
			const result = await getSoftwareItems(page, undefined, discoveryStateFilter());
			items = result.items;
			currentPage = result.page;
			totalPages = result.total_pages;
			totalItems = result.total;
			if (background) error = null;
		} catch (e) {
			if (!background) {
				error = e instanceof Error ? e.message : 'Failed to load software items';
			}
		} finally {
			if (!background) loading = false;
		}
	}

	function switchTab(tab: FilterTab) {
		if (activeTab === tab) return;
		activeTab = tab;
		loadAll(1);
	}

	function toggleMenu(id: string, button: HTMLElement) {
		if (openMenuId === id) {
			openMenuId = null;
			return;
		}
		const rect = button.getBoundingClientRect();
		menuPos = { top: rect.bottom + 4, left: rect.right - 180 };
		openMenuId = id;
	}

	function closeMenu() {
		openMenuId = null;
	}

	function requestDelete(item: SoftwareItemResponse) {
		closeMenu();
		confirmDelete = { id: item.id, name: item.name };
	}

	function openAssignModal(item: SoftwareItemResponse) {
		closeMenu();
		assignItem = { id: item.id, name: item.name };
	}

	async function executeDelete() {
		if (!confirmDelete || submitting) return;
		const { id } = confirmDelete;
		confirmDelete = null;
		submitting = true;
		try {
			await deleteSoftwareItem(id);
			items = items.filter((i) => i.id !== id);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to delete software item.');
			loadAll(currentPage);
		} finally {
			submitting = false;
		}
	}

	async function triggerVersionCheck(item: SoftwareItemResponse) {
		closeMenu();
		checkingVersionsId = item.id;
		try {
			const result = await checkSoftwareItemVersions(item.id);
			if (result.agents_notified > 0) {
				showSuccess(`Version check triggered — ${result.agents_notified} agent(s) notified`);
			} else {
				showSuccess('Version check queued — no agents currently connected');
			}
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to trigger version check.');
		} finally {
			checkingVersionsId = null;
		}
	}

	async function approveItem(item: SoftwareItemResponse) {
		closeMenu();
		approvingId = item.id;
		try {
			const updated = await approveSoftwareItem(item.id);
			showSuccess(`"${item.name}" approved for version tracking`);
			if (activeTab === 'pending') {
				items = items.filter((i) => i.id !== item.id);
			} else {
				items = items.map((i) => (i.id === item.id ? updated : i));
			}
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to approve software item.');
		} finally {
			approvingId = null;
		}
	}

	function handleWindowClick(event: MouseEvent) {
		if (openMenuId && !(event.target as HTMLElement).closest('.actions-menu')) {
			closeMenu();
		}
	}
</script>

<svelte:window
	onclick={handleWindowClick}
	onkeydown={(e) => {
		if (e.key === 'Escape' && confirmDelete) confirmDelete = null;
	}}
/>

{#if getUser()}
	<h1 class="h1 mb-4">Software</h1>

	{#if !canView}
		<aside class="rounded-lg p-4 preset-filled-error-500">
			<p>You do not have permission to view software items.</p>
		</aside>
	{:else}
		<div class="mb-4 flex items-center justify-between">
			<!-- Filter tabs -->
			<div class="flex gap-1">
				<button
					class="btn btn-sm {activeTab === 'all' ? 'preset-filled-primary-500' : 'preset-tonal'}"
					onclick={() => switchTab('all')}
				>
					All
				</button>
				<button
					class="btn btn-sm {activeTab === 'pending' ? 'preset-filled-warning-500' : 'preset-tonal'}"
					onclick={() => switchTab('pending')}
				>
					Pending{activeTab === 'pending' && totalItems > 0 ? ` (${totalItems})` : ''}
				</button>
				<button
					class="btn btn-sm {activeTab === 'active' ? 'preset-filled-success-500' : 'preset-tonal'}"
					onclick={() => switchTab('active')}
				>
					Active
				</button>
			</div>
			{#if canManage}
				<button class="btn preset-filled-primary-500" onclick={() => (showAddModal = true)}> Add Software </button>
			{/if}
		</div>

		{#if error}
			<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
				<p>{error}</p>
				<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadAll(currentPage)}>Retry</button>
			</aside>
		{/if}

		<div class="table-wrap">
			<table class="table">
				<thead>
					<tr>
						<th>Name</th>
						<th>Providers</th>
						<th>Status</th>
						<th>Hosts</th>
						<th>Last Checked</th>
						{#if canManage}
							<th class="w-20"></th>
						{/if}
					</tr>
				</thead>
				<tbody>
					{#if loading}
						<tr>
							<td colspan={canManage ? 6 : 5} class="py-6 text-center">Loading...</td>
						</tr>
					{:else}
						{#each items as item (item.id)}
							<tr>
								<td>{item.name}</td>
								<td>{item.provider_types.join(', ') || '\u2014'}</td>
								<td class="flex flex-wrap items-center gap-1">
									{#if item.enabled}
										<span class="badge preset-filled-success-500">Enabled</span>
									{:else}
										<span class="badge preset-tonal">Disabled</span>
									{/if}
									{#if item.discovery_state === 'pending'}
										<span class="badge preset-filled-warning-500">Pending</span>
									{:else if item.discovery_state === 'approved'}
										<span class="badge preset-tonal-success">Approved</span>
									{/if}
								</td>
								<td>{item.host_count}</td>
								<td>{formatDate(item.last_checked_at)}</td>
								{#if canManage}
									<td>
										<div class="actions-menu">
											<button
												class="btn btn-sm preset-tonal"
												aria-label="Actions for {item.name}"
												onclick={(e) => {
													e.stopPropagation();
													toggleMenu(item.id, e.currentTarget);
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
								<td colspan={canManage ? 6 : 5} class="py-8 text-center">
									{#if activeTab === 'pending'}
										<p class="text-lg font-medium">No pending items</p>
										<p class="mt-1 text-sm text-surface-500">No discovered software awaiting review.</p>
									{:else if activeTab === 'active'}
										<p class="text-lg font-medium">No active software</p>
										<p class="mt-1 text-sm text-surface-500">Register or approve software to start tracking.</p>
									{:else}
										<p class="text-lg font-medium">No software registered yet</p>
										<p class="mt-1 text-sm text-surface-500">Register a package to start tracking.</p>
									{/if}
								</td>
							</tr>
						{/each}
					{/if}
				</tbody>
			</table>
		</div>

		<Pagination {currentPage} {totalPages} onPageChange={loadAll} />

		{#if openMenuId}
			{@const item = items.find((i) => i.id === openMenuId)}
			{#if item}
				<ContextMenu top={menuPos.top} left={menuPos.left} onclose={closeMenu}>
					{#if item.discovery_state === 'pending'}
						<li>
							<button
								class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800 disabled:cursor-not-allowed disabled:opacity-50"
								role="menuitem"
								tabindex="-1"
								disabled={approvingId === item.id}
								onclick={() => approveItem(item)}
							>
								{approvingId === item.id ? 'Approving…' : 'Approve'}
							</button>
						</li>
						<li>
							<button
								class="w-full rounded-md px-3 py-2 text-left text-sm text-error-500 hover:bg-surface-200 dark:hover:bg-surface-800"
								role="menuitem"
								tabindex="-1"
								onclick={() => requestDelete(item)}
							>
								Delete
							</button>
						</li>
					{:else}
						<li>
							<button
								class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800 disabled:cursor-not-allowed disabled:opacity-50"
								role="menuitem"
								tabindex="-1"
								disabled={checkingVersionsId === item.id}
								onclick={() => triggerVersionCheck(item)}
							>
								{checkingVersionsId === item.id ? 'Checking…' : 'Check Versions'}
							</button>
						</li>
						<li>
							<button
								class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800"
								role="menuitem"
								tabindex="-1"
								onclick={() => openAssignModal(item)}
							>
								Assign to Host
							</button>
						</li>
						<li>
							<button
								class="w-full rounded-md px-3 py-2 text-left text-sm text-error-500 hover:bg-surface-200 dark:hover:bg-surface-800"
								role="menuitem"
								tabindex="-1"
								onclick={() => requestDelete(item)}
							>
								Delete
							</button>
						</li>
					{/if}
				</ContextMenu>
			{/if}
		{/if}

		{#if confirmDelete}
			<ConfirmDialog
				title="Delete Software Item"
				messagePrefix="Are you sure you want to delete"
				entityName={confirmDelete.name}
				confirmLabel={submitting ? 'Deleting...' : 'Delete'}
				confirmClass="preset-filled-error-500"
				confirmDisabled={submitting}
				onconfirm={executeDelete}
				oncancel={() => (confirmDelete = null)}
			/>
		{/if}

		{#if assignItem}
			<AssignToHostModal
				softwareItemId={assignItem.id}
				softwareItemName={assignItem.name}
				onclose={() => (assignItem = null)}
				onsuccess={() => {
					assignItem = null;
					loadAll(currentPage);
				}}
			/>
		{/if}

		{#if showAddModal}
			<AddSoftwareModal
				onclose={() => (showAddModal = false)}
				onsuccess={() => {
					showAddModal = false;
					loadAll(1);
				}}
			/>
		{/if}
	{/if}
{/if}
