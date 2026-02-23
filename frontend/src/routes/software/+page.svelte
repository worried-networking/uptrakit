<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { getUser } from '$lib/auth.svelte';
	import { getSoftwareItems } from '$lib/api';
	import { formatDate } from '$lib/utils';
	import Pagination from '$lib/components/Pagination.svelte';
	import AddSoftwareModal from '$lib/components/AddSoftwareModal.svelte';
	import type { SoftwareItemResponse } from '$lib/types';
	import { Permission } from '$lib/types';

	let items: SoftwareItemResponse[] = $state([]);
	let error: string | null = $state(null);
	let currentPage: number = $state(1);
	let totalPages: number = $state(1);
	let loading: boolean = $state(false);
	let showAddModal: boolean = $state(false);

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
</script>

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
				<button class="btn preset-filled-primary-500" onclick={() => (showAddModal = true)}>
					Add Software
				</button>
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
					</tr>
				</thead>
				<tbody>
					{#if loading}
						<tr>
							<td colspan="6" class="py-6 text-center">Loading...</td>
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
							</tr>
						{:else}
							<tr>
								<td colspan="6" class="py-8 text-center">
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
