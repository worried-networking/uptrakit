<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import {
		getHost,
		listHostPackages,
		updateHostPackage,
		deleteHostPackage,
		promoteHostPackage,
		batchHostPackages
	} from '$lib/api';
	import { formatDate, formatVersion, parseUrlPage } from '$lib/utils';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import { subscribeToEvent } from '$lib/stores/events.svelte';
	import { Permission } from '$lib/types';
	import type { HostResponse, HostPackageResponse, BatchActionResponse } from '$lib/types';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import Modal from '$lib/components/Modal.svelte';
	import Pagination from '$lib/components/Pagination.svelte';
	import BatchActionBar from '$lib/components/BatchActionBar.svelte';
	import BatchResultDialog from '$lib/components/BatchResultDialog.svelte';
	import ContextMenu from '$lib/components/ContextMenu.svelte';

	const id = $derived(page.params.id as string);

	let host: HostResponse | null = $state(null);
	let packages: HostPackageResponse[] = $state([]);
	let loading: boolean = $state(true);
	let error: string | null = $state(null);
	let currentPage: number = $state(parseUrlPage(page.url));
	let totalPages: number = $state(1);
	let total: number = $state(0);

	// Filters from URL
	let filterHasUpdate: boolean | undefined = $state(
		page.url.searchParams.get('has_update') === 'true' ? true : undefined
	);
	let filterCategory: string | undefined = $state(page.url.searchParams.get('category') ?? undefined);
	let filterSearch: string = $state(page.url.searchParams.get('search') ?? '');

	let confirmDelete: { pkg: HostPackageResponse; ignore: boolean } | null = $state(null);
	let togglingIds: Set<string> = $state(new Set());

	// Promote state
	let promoteModal: { pkg: HostPackageResponse } | null = $state(null);
	let promoteName: string = $state('');
	let promoteExistingId: string = $state('');
	let promoteShowAdvanced: boolean = $state(false);
	let promoting: boolean = $state(false);
	let selectedIds = new SvelteSet<string>();
	let batchConfirmAction: string | null = $state(null);
	let batchResult: BatchActionResponse | null = $state(null);
	let batchSubmitting: boolean = $state(false);
	let openMenuId: string | null = $state(null);
	let menuPos: { top: number; left: number } = $state({ top: 0, left: 0 });

	const canManageSoftware = $derived(getUser()?.permissions.includes(Permission.ManageSoftware) ?? false);

	const batchActions = $derived.by(() => {
		const selected = packages.filter((p) => selectedIds.has(p.id));
		const acts: { id: string; label: string; destructive?: boolean }[] = [];
		if (selected.some((p) => !p.enabled)) {
			acts.push({ id: 'enable', label: 'Enable' });
		}
		if (selected.some((p) => p.enabled)) {
			acts.push({ id: 'disable', label: 'Disable' });
		}
		acts.push({ id: 'delete', label: 'Delete', destructive: true });
		acts.push({ id: 'ignore', label: 'Ignore', destructive: true });
		return acts;
	});

	let refreshInterval: ReturnType<typeof setInterval> | null = null;
	let unsubscribers: (() => void)[] = [];

	onMount(() => {
		loadData();
		unsubscribers.push(
			subscribeToEvent('host_packages_changed', (data) => {
				if (data.host_id === id) loadPackages(currentPage, true);
			}),
			subscribeToEvent('batch_host_package_update_completed', (data) => {
				if (data.host_id === id) loadPackages(currentPage, true);
			})
		);
		refreshInterval = setInterval(() => {
			if (document.visibilityState === 'visible') loadPackages(currentPage, true);
		}, 300_000);
	});

	onDestroy(() => {
		for (const unsub of unsubscribers) unsub();
		if (refreshInterval) clearInterval(refreshInterval);
	});

	async function loadData() {
		loading = true;
		error = null;
		try {
			const [hostResult] = await Promise.all([getHost(id), loadPackages(currentPage)]);
			host = hostResult;
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load data';
		} finally {
			loading = false;
		}
	}

	async function loadPackages(pg: number, background = false) {
		try {
			const result = await listHostPackages(id, {
				page: pg,
				per_page: 25,
				has_update: filterHasUpdate,
				category: filterCategory,
				search: filterSearch || undefined
			});
			packages = result.items;
			currentPage = result.page;
			totalPages = result.total_pages;
			total = result.total;
		} catch (e) {
			if (!background) throw e;
		}
	}

	function applyFilters() {
		currentPage = 1;
		syncUrl();
		loadPackages(1);
	}

	function clearFilters() {
		filterHasUpdate = undefined;
		filterCategory = undefined;
		filterSearch = '';
		currentPage = 1;
		syncUrl();
		loadPackages(1);
	}

	function syncUrl() {
		const parts: string[] = [];
		if (currentPage > 1) parts.push(`page=${currentPage}`);
		if (filterHasUpdate != null) parts.push(`has_update=${filterHasUpdate}`);
		if (filterCategory) parts.push(`category=${encodeURIComponent(filterCategory)}`);
		if (filterSearch) parts.push(`search=${encodeURIComponent(filterSearch)}`);
		const query = parts.join('&');
		goto(query ? `${location.pathname}?${query}` : location.pathname, {
			replaceState: true,
			keepFocus: true,
			noScroll: true
		});
	}

	function handlePageChange(pg: number) {
		currentPage = pg;
		syncUrl();
		loadPackages(pg);
	}

	async function toggleEnabled(pkg: HostPackageResponse) {
		togglingIds = new Set([...togglingIds, pkg.id]);
		try {
			const updated = await updateHostPackage(id, pkg.id, { enabled: !pkg.enabled });
			packages = packages.map((p) => (p.id === pkg.id ? updated : p));
			showSuccess(`Package "${pkg.name}" ${updated.enabled ? 'enabled' : 'disabled'}`);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to update package');
		} finally {
			togglingIds = new Set([...togglingIds].filter((i) => i !== pkg.id));
		}
	}

	function requestDelete(pkg: HostPackageResponse, ignore: boolean) {
		confirmDelete = { pkg, ignore };
	}

	async function executeDelete() {
		if (!confirmDelete) return;
		const { pkg, ignore } = confirmDelete;
		confirmDelete = null;
		try {
			await deleteHostPackage(id, pkg.id, ignore);
			packages = packages.filter((p) => p.id !== pkg.id);
			total = Math.max(0, total - 1);
			showSuccess(ignore ? `Package "${pkg.name}" deleted and ignore rule created` : `Package "${pkg.name}" deleted`);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to delete package');
		}
	}

	function toggleMenu(pkgId: string, button: HTMLElement) {
		if (openMenuId === pkgId) {
			openMenuId = null;
			return;
		}
		const rect = button.getBoundingClientRect();
		menuPos = { top: rect.bottom + 4, left: rect.right - 160 };
		openMenuId = pkgId;
	}

	function closeMenu() {
		openMenuId = null;
	}

	function handleWindowClick(event: MouseEvent) {
		if (openMenuId && !(event.target as HTMLElement).closest('.actions-menu')) {
			closeMenu();
		}
	}

	function toggleSelectAll() {
		if (selectedIds.size === packages.length) {
			selectedIds.clear();
		} else {
			selectedIds.clear();
			for (const pkg of packages) selectedIds.add(pkg.id);
		}
	}

	function toggleSelect(pkgId: string) {
		if (selectedIds.has(pkgId)) {
			selectedIds.delete(pkgId);
		} else {
			selectedIds.add(pkgId);
		}
	}

	function requestBatchAction(actionId: string) {
		batchConfirmAction = actionId;
	}

	async function executeBatchAction() {
		if (!batchConfirmAction || batchSubmitting) return;
		const action = batchConfirmAction;
		batchConfirmAction = null;
		batchSubmitting = true;
		try {
			if (action === 'ignore') {
				// Client-side orchestration: delete each package with ignore=true
				const succeeded: { id: string }[] = [];
				const failed: { id: string; error: string }[] = [];
				for (const pkgId of [...selectedIds]) {
					try {
						await deleteHostPackage(id, pkgId, true);
						succeeded.push({ id: pkgId });
					} catch (e) {
						failed.push({ id: pkgId, error: e instanceof Error ? e.message : 'Unknown error' });
					}
				}
				if (failed.length > 0) {
					batchResult = { succeeded, failed };
				} else {
					showSuccess(`${succeeded.length} package(s) deleted and ignore rules created.`);
				}
				selectedIds.clear();
				await loadPackages(currentPage);
				batchSubmitting = false;
				return;
			}
			const response = await batchHostPackages(id, action, [...selectedIds]);
			if (response.failed.length > 0) {
				batchResult = response;
			} else {
				showSuccess(`${response.succeeded.length} package(s) ${action}d successfully.`);
			}
			selectedIds.clear();
			await loadPackages(currentPage);
		} catch (e) {
			showError(e instanceof Error ? e.message : `Failed to ${action} packages`);
		} finally {
			batchSubmitting = false;
		}
	}

	const hasActiveFilters = $derived(filterHasUpdate != null || filterCategory != null || filterSearch !== '');

	function openPromoteModal(pkg: HostPackageResponse) {
		promoteModal = { pkg };
		promoteName = pkg.name;
		promoteExistingId = '';
		promoteShowAdvanced = false;
	}

	async function executePromote() {
		if (!promoteModal) return;
		promoting = true;
		try {
			const result = await promoteHostPackage(id, promoteModal.pkg.id, {
				name: promoteName.trim() || undefined,
				software_item_id: promoteExistingId.trim() || undefined
			});
			promoteModal = null;
			showSuccess(`Package promoted to software item "${result.name}". View it at /software/${result.id}.`);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to promote package');
		} finally {
			promoting = false;
		}
	}
</script>

<svelte:window onclick={handleWindowClick} />

{#if getUser()}
	<div class="mb-4">
		<a href="/hosts/{id}" class="text-sm text-surface-500 hover:underline">&larr; Back to Host</a>
	</div>

	{#if loading}
		<p>Loading...</p>
	{:else if error}
		<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
			<p>{error}</p>
			<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadData()}>Retry</button>
		</aside>
	{:else}
		<div class="mb-6 flex flex-wrap items-start justify-between gap-4">
			<div>
				<h1 class="h1">Packages — {host?.friendly_name ?? 'Host'}</h1>
				<p class="mt-1 text-sm text-surface-500">{total} package{total === 1 ? '' : 's'} tracked</p>
			</div>
		</div>

		<!-- Filters -->
		<div class="mb-4 flex flex-wrap items-end gap-3">
			<label class="label text-sm">
				<span>Search</span>
				<input
					class="input input-sm"
					type="text"
					placeholder="Package name..."
					bind:value={filterSearch}
					onkeydown={(e) => {
						if (e.key === 'Enter') applyFilters();
					}}
				/>
			</label>
			<label class="label text-sm">
				<span>Has Update</span>
				<select class="select select-sm" bind:value={filterHasUpdate} onchange={applyFilters}>
					<option value={undefined}>All</option>
					<option value={true}>Yes</option>
				</select>
			</label>
			<label class="label text-sm">
				<span>Category</span>
				<select class="select select-sm" bind:value={filterCategory} onchange={applyFilters}>
					<option value={undefined}>All</option>
					<option value="security">Security</option>
					<option value="standard">Standard</option>
					<option value="unknown">Unknown</option>
				</select>
			</label>
			<button class="btn btn-sm preset-tonal-surface" onclick={applyFilters}>Filter</button>
			{#if hasActiveFilters}
				<button class="btn btn-sm preset-tonal" onclick={clearFilters}>Clear</button>
			{/if}
		</div>

		<!-- Table -->
		<div class="table-wrap">
			<table class="table">
				<thead>
					<tr>
						{#if canManageSoftware}
							<th class="w-10">
								<input
									type="checkbox"
									class="checkbox"
									checked={packages.length > 0 && selectedIds.size === packages.length}
									indeterminate={selectedIds.size > 0 && selectedIds.size < packages.length}
									onchange={toggleSelectAll}
									aria-label="Select all"
								/>
							</th>
						{/if}
						<th>Name</th>
						<th>Installed</th>
						<th>Latest</th>
						<th>Category</th>
						<th>Status</th>
						<th>Enabled</th>
						<th>Last Checked</th>
						{#if canManageSoftware}<th class="w-20"></th>{/if}
					</tr>
				</thead>
				<tbody>
					{#each packages as pkg (pkg.id)}
						<tr>
							{#if canManageSoftware}
								<td>
									<input
										type="checkbox"
										class="checkbox"
										checked={selectedIds.has(pkg.id)}
										onchange={() => toggleSelect(pkg.id)}
										aria-label="Select {pkg.name}"
									/>
								</td>
							{/if}
							<td class="font-medium">{pkg.name}</td>
							<td title={pkg.installed_version ?? undefined}>{formatVersion(pkg.installed_version)}</td>
							<td title={pkg.latest_version ?? undefined}>{formatVersion(pkg.latest_version)}</td>
							<td>
								<span class="badge {pkg.update_category === 'security' ? 'preset-filled-error-500' : 'preset-tonal'}">
									{pkg.update_category}
								</span>
							</td>
							<td>
								{#if pkg.has_update}
									<span class="badge preset-filled-warning-500">Update Available</span>
								{:else}
									<span class="badge preset-filled-success-500">Up-to-date</span>
								{/if}
							</td>
							<td>
								{#if canManageSoftware}
									<button
										class="btn btn-sm {pkg.enabled ? 'preset-filled-success-500' : 'preset-tonal'}"
										disabled={togglingIds.has(pkg.id)}
										onclick={() => toggleEnabled(pkg)}
									>
										{pkg.enabled ? 'Yes' : 'No'}
									</button>
								{:else}
									{pkg.enabled ? 'Yes' : 'No'}
								{/if}
							</td>
							<td>{formatDate(pkg.last_checked_at)}</td>
							{#if canManageSoftware}
								<td>
									<div class="actions-menu">
										<button
											class="btn btn-sm preset-tonal"
											aria-label="Actions for {pkg.name}"
											onclick={(e) => {
												e.stopPropagation();
												toggleMenu(pkg.id, e.currentTarget);
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
							<td colspan={canManageSoftware ? 10 : 7} class="text-center py-8">
								<p class="text-lg font-medium">
									{#if hasActiveFilters}
										No packages match the current filters
									{:else}
										No packages discovered yet
									{/if}
								</p>
								<p class="mt-1 text-sm text-surface-500">
									{#if hasActiveFilters}
										Try adjusting or clearing the filters.
									{:else}
										Packages appear here when a package manager plugin discovers them on this host.
									{/if}
								</p>
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>

		<Pagination {currentPage} {totalPages} {total} onPageChange={handlePageChange} />

		{#if canManageSoftware && selectedIds.size > 0}
			<BatchActionBar
				selectedCount={selectedIds.size}
				actions={batchActions}
				onaction={requestBatchAction}
				oncancel={() => selectedIds.clear()}
			/>
		{/if}

		{#if batchConfirmAction}
			<ConfirmDialog
				title="Batch {batchConfirmAction}"
				messagePrefix="Are you sure you want to {batchConfirmAction}"
				entityName="{selectedIds.size} package(s)"
				confirmLabel={batchSubmitting
					? 'Processing...'
					: batchConfirmAction === 'enable'
						? 'Enable'
						: batchConfirmAction === 'disable'
							? 'Disable'
							: batchConfirmAction === 'ignore'
								? 'Delete & Ignore'
								: 'Delete'}
				confirmClass={batchConfirmAction === 'delete' || batchConfirmAction === 'ignore'
					? 'preset-filled-error-500'
					: 'preset-filled-primary-500'}
				confirmDisabled={batchSubmitting}
				onconfirm={executeBatchAction}
				oncancel={() => (batchConfirmAction = null)}
			/>
		{/if}

		{#if batchResult}
			<BatchResultDialog title="Batch Action Results" response={batchResult} onclose={() => (batchResult = null)} />
		{/if}
	{/if}
{/if}

{#if openMenuId}
	{@const pkg = packages.find((p) => p.id === openMenuId)}
	{#if pkg}
		<ContextMenu top={menuPos.top} left={menuPos.left} onclose={closeMenu}>
			<li>
				<button
					class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800"
					role="menuitem"
					tabindex="-1"
					onclick={() => {
						closeMenu();
						openPromoteModal(pkg);
					}}
				>
					Promote to Software Item
				</button>
			</li>
			<li>
				<button
					class="w-full rounded-md px-3 py-2 text-left text-sm text-error-500 hover:bg-surface-200 dark:hover:bg-surface-800"
					role="menuitem"
					tabindex="-1"
					onclick={() => {
						closeMenu();
						requestDelete(pkg, false);
					}}
				>
					Delete
				</button>
			</li>
			<li>
				<button
					class="w-full rounded-md px-3 py-2 text-left text-sm text-error-500 hover:bg-surface-200 dark:hover:bg-surface-800"
					role="menuitem"
					tabindex="-1"
					onclick={() => {
						closeMenu();
						requestDelete(pkg, true);
					}}
				>
					Delete &amp; Ignore
				</button>
			</li>
		</ContextMenu>
	{/if}
{/if}

{#if confirmDelete}
	<ConfirmDialog
		title={confirmDelete.ignore ? 'Delete & Ignore Package' : 'Delete Package'}
		messagePrefix={confirmDelete.ignore ? 'Delete and create ignore rule for' : 'Delete package'}
		entityName={confirmDelete.pkg.name}
		confirmLabel={confirmDelete.ignore ? 'Delete & Ignore' : 'Delete'}
		confirmClass="preset-filled-error-500"
		onconfirm={executeDelete}
		oncancel={() => (confirmDelete = null)}
	/>
{/if}

{#if promoteModal}
	<Modal title="Promote to Software Item" onclose={() => (promoteModal = null)}>
		<p class="text-sm text-surface-500">
			Creates a tracked software item for <strong>{promoteModal.pkg.name}</strong> alongside the existing host package. The
			operation is idempotent — if a matching item already exists it will be returned.
		</p>

		<label class="label">
			<span>Software Item Name</span>
			<input class="input" type="text" placeholder={promoteModal.pkg.name} bind:value={promoteName} />
		</label>

		<button
			class="btn btn-sm preset-tonal text-xs"
			type="button"
			onclick={() => (promoteShowAdvanced = !promoteShowAdvanced)}
		>
			{promoteShowAdvanced ? 'Hide advanced' : 'Show advanced'}
		</button>

		{#if promoteShowAdvanced}
			<label class="label">
				<span>Link to existing software item UUID (optional)</span>
				<input
					class="input"
					type="text"
					placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
					bind:value={promoteExistingId}
				/>
			</label>
		{/if}

		{#snippet footer()}
			<button class="btn preset-tonal-surface" onclick={() => (promoteModal = null)} disabled={promoting}>Cancel</button
			>
			<button class="btn preset-filled-primary-500" onclick={executePromote} disabled={promoting}>
				{promoting ? 'Promoting…' : 'Promote'}
			</button>
		{/snippet}
	</Modal>
{/if}
