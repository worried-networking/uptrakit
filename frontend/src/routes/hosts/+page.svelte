<script lang="ts">
	import { user } from '$lib/auth';
	import { goto } from '$app/navigation';
	import { getHosts, updateHost, deactivateHost } from '$lib/api';
	import type { HostResponse } from '$lib/types';
	import { Permission } from '$lib/types';

	let hosts: HostResponse[] = $state([]);
	let error: string | null = $state(null);
	let openMenuId: string | null = $state(null);
	let menuPos: { top: number; left: number } = $state({ top: 0, left: 0 });
	let confirmAction: { hostId: string; action: 'deactivate'; name: string } | null = $state(null);
	let editHost: { id: string; friendlyName: string } | null = $state(null);

	$effect(() => {
		if (!$user) {
			goto('/login');
		}
	});

	$effect(() => {
		if ($user) {
			loadHosts();
		}
	});

	async function loadHosts() {
		try {
			error = null;
			hosts = await getHosts();
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
		if (!editHost) return;
		try {
			error = null;
			const updated = await updateHost(editHost.id, { friendly_name: editHost.friendlyName });
			hosts = hosts.map((h) => (h.id === editHost?.id ? updated : h));
			editHost = null;
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to update host';
		}
	}

	async function executeConfirmed() {
		if (!confirmAction) return;
		const { hostId } = confirmAction;
		confirmAction = null;

		try {
			error = null;
			await deactivateHost(hostId);
			hosts = hosts.filter((h) => h.id !== hostId);
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to deactivate host';
		}
	}

	function formatDate(date: string | null): string {
		if (!date) return '\u2014';
		return new Date(date).toLocaleString();
	}

	function handleWindowClick(event: MouseEvent) {
		if (openMenuId && !(event.target as HTMLElement).closest('.actions-menu')) {
			closeMenu();
		}
	}

	const canManage = $derived($user?.permissions.includes(Permission.ManageAgents) ?? false);
</script>

<svelte:window onclick={handleWindowClick} />

{#if $user}
	<h1 class="h1 mb-6">Hosts</h1>

	{#if error}
		<aside class="alert variant-filled-error mb-4">
			<div class="alert-message">
				<p>{error}</p>
			</div>
		</aside>
	{/if}

	<div class="table-container">
		<table class="table table-hover">
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
										class="btn btn-sm variant-soft"
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
						<td colspan={canManage ? 8 : 7} class="text-center">No hosts found.</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>

	{#if openMenuId}
		{@const host = hosts.find((h) => h.id === openMenuId)}
		{#if host}
			<div
				class="card fixed z-50 w-40 overflow-hidden p-0 shadow-xl"
				style="top: {menuPos.top}px; left: {menuPos.left}px;"
			>
				<nav class="list-nav">
					<ul>
						<li>
							<button
								class="w-full text-left"
								onclick={() => openEditDialog(host)}
							>
								Edit Name
							</button>
						</li>
						<li>
							<button
								class="w-full text-left text-error-500"
								onclick={() => requestConfirm(host.id, 'deactivate', host.friendly_name)}
							>
								Deactivate
							</button>
						</li>
					</ul>
				</nav>
			</div>
		{/if}
	{/if}

	{#if confirmAction}
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div
			class="fixed inset-0 z-50 flex items-center justify-center bg-surface-backdrop-token p-4"
			onkeydown={(e) => { if (e.key === 'Escape') cancelConfirm(); }}
		>
			<div class="card w-full max-w-md space-y-4 p-6 shadow-xl">
				<h3 class="h3">Deactivate Host</h3>
				<p>
					Are you sure you want to deactivate
					<strong>{confirmAction.name}</strong>?
				</p>
				<div class="flex justify-end gap-2">
					<button class="btn variant-ghost-surface" onclick={cancelConfirm}>Cancel</button>
					<button class="btn variant-filled-error" onclick={executeConfirmed}>
						Deactivate
					</button>
				</div>
			</div>
		</div>
	{/if}

	{#if editHost}
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div
			class="fixed inset-0 z-50 flex items-center justify-center bg-surface-backdrop-token p-4"
			onkeydown={(e) => { if (e.key === 'Escape') cancelEdit(); }}
		>
			<div class="card w-full max-w-md space-y-4 p-6 shadow-xl">
				<h3 class="h3">Edit Host Name</h3>
				<label class="label">
					<span>Friendly Name</span>
					<input class="input" type="text" bind:value={editHost.friendlyName} />
				</label>
				<div class="flex justify-end gap-2">
					<button class="btn variant-ghost-surface" onclick={cancelEdit}>Cancel</button>
					<button class="btn variant-filled-primary" onclick={executeEdit}>
						Save
					</button>
				</div>
			</div>
		</div>
	{/if}
{/if}
