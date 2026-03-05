<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import { getHost, listHostPackages, updateHostPackage, deleteHostPackage, promoteHostPackage } from '$lib/api';
	import { formatDate, formatVersion, parseUrlPage } from '$lib/utils';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import { Permission } from '$lib/types';
	import type { HostResponse, HostPackageResponse } from '$lib/types';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import Pagination from '$lib/components/Pagination.svelte';

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

	const canManageSoftware = $derived(getUser()?.permissions.includes(Permission.ManageSoftware) ?? false);

	let refreshInterval: ReturnType<typeof setInterval> | null = null;

	onMount(() => {
		loadData();
		refreshInterval = setInterval(() => {
			if (document.visibilityState === 'visible') loadPackages(currentPage, true);
		}, 30_000);
	});

	onDestroy(() => {
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

<svelte:window
	onkeydown={(e) => {
		if (e.key === 'Escape') {
			if (confirmDelete) confirmDelete = null;
			if (promoteModal) promoteModal = null;
		}
	}}
/>

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
						<th>Name</th>
						<th>Installed</th>
						<th>Latest</th>
						<th>Category</th>
						<th>Status</th>
						<th>Enabled</th>
						<th>Last Checked</th>
						{#if canManageSoftware}<th class="w-32">Actions</th>{/if}
					</tr>
				</thead>
				<tbody>
					{#each packages as pkg (pkg.id)}
						<tr>
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
									<div class="flex gap-1">
										<button
											class="btn btn-sm preset-tonal-primary"
											title="Promote to software item"
											onclick={() => openPromoteModal(pkg)}
										>
											Promote
										</button>
										<button class="btn btn-sm preset-tonal-error" onclick={() => requestDelete(pkg, false)}>
											Delete
										</button>
										<button
											class="btn btn-sm preset-tonal"
											title="Delete and ignore"
											onclick={() => requestDelete(pkg, true)}
										>
											Ignore
										</button>
									</div>
								</td>
							{/if}
						</tr>
					{:else}
						<tr>
							<td colspan={canManageSoftware ? 8 : 7} class="text-center py-8">
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

		<Pagination {currentPage} {totalPages} onPageChange={handlePageChange} />
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
	<!-- Promote modal backdrop -->
	<div
		class="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
		role="dialog"
		tabindex="-1"
		aria-modal="true"
		aria-labelledby="promote-modal-title"
		onkeydown={(e) => {
			if (e.key === 'Escape') promoteModal = null;
		}}
	>
		<div class="card w-full max-w-md p-6 space-y-4">
			<h2 id="promote-modal-title" class="h3">Promote to Software Item</h2>
			<p class="text-sm text-surface-500">
				Creates a tracked software item for <strong>{promoteModal.pkg.name}</strong> alongside the existing host package.
				The operation is idempotent — if a matching item already exists it will be returned.
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

			<div class="flex gap-2 justify-end pt-2">
				<button class="btn preset-tonal" onclick={() => (promoteModal = null)} disabled={promoting}> Cancel </button>
				<button class="btn preset-filled-primary-500" onclick={executePromote} disabled={promoting}>
					{promoting ? 'Promoting…' : 'Promote'}
				</button>
			</div>
		</div>
	</div>
{/if}
