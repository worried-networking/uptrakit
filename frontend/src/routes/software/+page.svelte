<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { getUser } from '$lib/auth.svelte';
	import { getSoftwareItems, deleteSoftwareItem } from '$lib/api';
	import { formatDate } from '$lib/utils';
	import { showError } from '$lib/notifications.svelte';
	import Pagination from '$lib/components/Pagination.svelte';
	import AddSoftwareModal from '$lib/components/AddSoftwareModal.svelte';
	import AssignToHostModal from '$lib/components/AssignToHostModal.svelte';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import type { SoftwareItemResponse } from '$lib/types';
	import { Permission } from '$lib/types';

	let items: SoftwareItemResponse[] = $state([]);
	let error: string | null = $state(null);
	let currentPage: number = $state(1);
	let totalPages: number = $state(1);
	let loading: boolean = $state(false);
	let showAddModal: boolean = $state(false);
	let openMenuId: string | null = $state(null);
	let menuPos: { top: number; left: number } = $state({ top: 0, left: 0 });
	let confirmDelete: { id: string; name: string } | null = $state(null);
	let assignItem: { id: string; name: string } | null = $state(null);
	let submitting: boolean = $state(false);

	const canView = $derived(getUser()?.permissions.includes(Permission.ViewSoftware) ?? false);
	const canManage = $derived(getUser()?.permissions.includes(Permission.ManageSoftware) ?? false);

	let refreshInterval: ReturnType<typeof setInterval> | null = null;

	onMount(() => {
		if (canView) {
			loadAll(1);
			refreshInterval = setInterval(() => {
				if (document.visibilityState === 'visible') loadAll(currentPage);
			}, 60_000);
		}
	});

	onDestroy(() => {
		if (refreshInterval) clearInterval(refreshInterval);
	});

	async function loadAll(page: number) {
		loading = true;
		try {
			error = null;
			const result = await getSoftwareItems(page);
			items = result.items;
			currentPage = result.page;
			totalPages = result.total_pages;
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load software items';
		} finally {
			loading = false;
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
		{#if canManage}
			<div class="mb-4 flex items-center justify-between">
				<div></div>
				<button class="btn preset-filled-primary-500" onclick={() => (showAddModal = true)}> Add Software </button>
			</div>
		{/if}

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
						<th>Provider</th>
						<th>Package</th>
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
							<td colspan={canManage ? 7 : 6} class="py-6 text-center">Loading...</td>
						</tr>
					{:else}
						{#each items as item (item.id)}
							<tr>
								<td>{item.name}</td>
								<td>
									<span class="badge preset-tonal">{item.provider_config_name}</span>
								</td>
								<td>{item.package_identifier || '\u2014'}</td>
								<td>
									{#if item.enabled}
										<span class="badge preset-filled-success-500">Enabled</span>
									{:else}
										<span class="badge preset-tonal">Disabled</span>
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
								<td colspan={canManage ? 7 : 6} class="py-8 text-center">
									<p class="text-lg font-medium">No software registered yet</p>
									<p class="mt-1 text-sm text-surface-500">Register a package to start tracking.</p>
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
