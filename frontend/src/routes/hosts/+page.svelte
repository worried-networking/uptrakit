<script lang="ts">
	import { onMount } from 'svelte';
	import { user } from '$lib/auth';
	import { getHosts, updateHost, deactivateHost } from '$lib/api';
	import type { HostResponse } from '$lib/types';
	import { Permission } from '$lib/types';
	import { formatDate } from '$lib/utils';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import ModalBackdrop from '$lib/components/ModalBackdrop.svelte';
	import Pagination from '$lib/components/Pagination.svelte';

	let hosts: HostResponse[] = $state([]);
	let error: string | null = $state(null);
	let openMenuId: string | null = $state(null);
	let menuPos: { top: number; left: number } = $state({ top: 0, left: 0 });
	let confirmAction: { hostId: string; action: 'deactivate'; name: string } | null = $state(null);
	let editHost: { id: string; friendlyName: string } | null = $state(null);
	let submitting: boolean = $state(false);
	let currentPage: number = $state(1);
	let totalPages: number = $state(1);

	onMount(() => loadHosts(1));

	async function loadHosts(page: number) {
		try {
			error = null;
			const result = await getHosts(page);
			hosts = result.items;
			currentPage = result.page;
			totalPages = result.total_pages;
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load hosts';
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

	function requestConfirm(hostId: string, action: 'deactivate', name: string) {
		closeMenu();
		confirmAction = { hostId, action, name };
	}

	function cancelConfirm() {
		confirmAction = null;
	}

	function openEditDialog(host: HostResponse) {
		closeMenu();
		editHost = { id: host.id, friendlyName: host.friendly_name };
	}

	function cancelEdit() {
		editHost = null;
	}

	async function executeEdit() {
		if (!editHost || submitting) return;
		submitting = true;
		try {
			error = null;
			const updated = await updateHost(editHost.id, { friendly_name: editHost.friendlyName });
			hosts = hosts.map((h) => (h.id === editHost?.id ? updated : h));
			editHost = null;
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to update host';
		} finally {
			submitting = false;
		}
	}

	async function executeConfirmed() {
		if (!confirmAction || submitting) return;
		const { hostId } = confirmAction;
		confirmAction = null;
		submitting = true;

		try {
			error = null;
			await deactivateHost(hostId);
			hosts = hosts.filter((h) => h.id !== hostId);
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to deactivate host';
		} finally {
			submitting = false;
		}
	}

	function handleWindowClick(event: MouseEvent) {
		if (openMenuId && !(event.target as HTMLElement).closest('.actions-menu')) {
			closeMenu();
		}
	}

	const canManage = $derived($user?.permissions.includes(Permission.ManageHosts) ?? false);
</script>

<svelte:window
	onclick={handleWindowClick}
	onkeydown={(e) => {
		if (e.key === 'Escape') {
			if (confirmAction) cancelConfirm();
			else if (editHost) cancelEdit();
		}
	}}
/>

{#if $user}
	<h1 class="h1 mb-6">Hosts</h1>

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
					<th>Hostname</th>
					<th>OS</th>
					<th>Architecture</th>
					<th>IP</th>
					<th>Agents</th>
					<th>Last Seen</th>
					{#if canManage}
						<th class="w-20"></th>
					{/if}
				</tr>
			</thead>
			<tbody>
				{#each hosts as host (host.id)}
					<tr>
						<td>{host.friendly_name}</td>
						<td>{host.hostname}</td>
						<td>{host.os_version ?? host.os_type ?? '\u2014'}</td>
						<td>{host.architecture ?? '\u2014'}</td>
						<td>{host.ip_address ?? '\u2014'}</td>
						<td>{host.agents.length}</td>
						<td>{formatDate(host.last_seen_at)}</td>
						{#if canManage}
							<td>
								<div class="actions-menu">
									<button
										class="btn btn-sm preset-tonal"
										aria-label="Actions for {host.friendly_name}"
										onclick={(e) => {
											e.stopPropagation();
											toggleMenu(host.id, e.currentTarget);
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
						<td colspan={canManage ? 8 : 7} class="text-center py-8">
							<p class="text-lg font-medium">No hosts discovered yet</p>
							<p class="mt-1 text-sm text-surface-500">
								Hosts appear here automatically when an approved agent reports from a new machine.
							</p>
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>

	<Pagination {currentPage} {totalPages} onPageChange={loadHosts} />

	{#if openMenuId}
		{@const host = hosts.find((h) => h.id === openMenuId)}
		{#if host}
			<ContextMenu top={menuPos.top} left={menuPos.left} onclose={closeMenu}>
				<li>
					<button
						class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800"
						role="menuitem"
						tabindex="-1"
						onclick={() => openEditDialog(host)}
					>
						Edit Name
					</button>
				</li>
				<li>
					<button
						class="w-full rounded-md px-3 py-2 text-left text-sm text-error-500 hover:bg-surface-200 dark:hover:bg-surface-800"
						role="menuitem"
						tabindex="-1"
						onclick={() => requestConfirm(host.id, 'deactivate', host.friendly_name)}
					>
						Deactivate
					</button>
				</li>
			</ContextMenu>
		{/if}
	{/if}

	{#if confirmAction}
		<ConfirmDialog
			title="Deactivate Host"
			messagePrefix="Are you sure you want to deactivate"
			entityName={confirmAction.name}
			confirmLabel={submitting ? 'Processing...' : 'Deactivate'}
			confirmDisabled={submitting}
			onconfirm={executeConfirmed}
			oncancel={cancelConfirm}
		/>
	{/if}

	{#if editHost}
		<ModalBackdrop onclose={cancelEdit}>
			<div
				class="card bg-surface-50 dark:bg-surface-900 w-full max-w-md space-y-4 p-6 shadow-xl"
				role="dialog"
				aria-modal="true"
			>
				<h3 class="h3">Edit Host Name</h3>
				<label class="label">
					<span>Friendly Name</span>
					<input class="input" type="text" bind:value={editHost.friendlyName} />
				</label>
				<div class="flex justify-end gap-2">
					<button class="btn preset-tonal-surface" onclick={cancelEdit}>Cancel</button>
					<button class="btn preset-filled-primary-500" disabled={submitting} onclick={executeEdit}>
						{submitting ? 'Saving...' : 'Save'}
					</button>
				</div>
			</div>
		</ModalBackdrop>
	{/if}
{/if}
