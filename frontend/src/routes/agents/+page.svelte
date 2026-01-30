<script lang="ts">
	import { user } from '$lib/auth';
	import { goto } from '$app/navigation';
	import { getAgents, approveAgent, rejectAgent, deleteAgent } from '$lib/api';
	import type { AgentResponse } from '$lib/types';

	let agents: AgentResponse[] = $state([]);
	let error: string | null = $state(null);
	let openMenuId: string | null = $state(null);
	let menuPos: { top: number; left: number } = $state({ top: 0, left: 0 });
	let confirmAction: { agentId: string; action: 'approve' | 'reject' | 'delete'; name: string } | null =
		$state(null);

	$effect(() => {
		if (!$user) {
			goto('/login');
		}
	});

	$effect(() => {
		if ($user) {
			loadAgents();
		}
	});

	async function loadAgents() {
		try {
			error = null;
			agents = await getAgents();
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load agents';
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

	function requestConfirm(agentId: string, action: 'approve' | 'reject' | 'delete', name: string) {
		closeMenu();
		confirmAction = { agentId, action, name };
	}

	function cancelConfirm() {
		confirmAction = null;
	}

	async function executeConfirmed() {
		if (!confirmAction) return;
		const { agentId, action } = confirmAction;
		confirmAction = null;

		try {
			error = null;
			if (action === 'approve') {
				const updated = await approveAgent(agentId);
				agents = agents.filter((a) => a.id !== agentId);
			} else if (action === 'reject') {
				await rejectAgent(agentId);
				agents = agents.filter((a) => a.id !== agentId);
			} else if (action === 'delete') {
				await deleteAgent(agentId);
				agents = agents.filter((a) => a.id !== agentId);
			}
		} catch (e) {
			error = e instanceof Error ? e.message : `Failed to ${action} agent`;
		}
	}

	function formatDate(date: string | null): string {
		if (!date) return '—';
		return new Date(date).toLocaleString();
	}

	function handleWindowClick(event: MouseEvent) {
		if (openMenuId && !(event.target as HTMLElement).closest('.actions-menu')) {
			closeMenu();
		}
	}

	const confirmLabels = {
		approve: { title: 'Approve Agent', verb: 'approve', btnClass: 'variant-filled-success' },
		reject: { title: 'Reject Agent', verb: 'reject', btnClass: 'variant-filled-error' },
		delete: { title: 'Delete Agent', verb: 'permanently delete', btnClass: 'variant-filled-error' }
	} as const;
</script>

<svelte:window onclick={handleWindowClick} />

{#if $user}
	<h1 class="h1 mb-6">Agents</h1>

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
					<th>IP</th>
					<th>Status</th>
					<th>Last Seen</th>
					<th class="w-20"></th>
				</tr>
			</thead>
			<tbody>
				{#each agents as agent (agent.id)}
					<tr>
						<td>{agent.friendly_name}</td>
						<td>{agent.hostname}</td>
						<td>{agent.ip_address ?? '—'}</td>
						<td>
							{#if agent.status === 'pending'}
								<span class="badge variant-filled-warning">Pending</span>
							{:else if agent.status === 'approved'}
								<span class="badge variant-filled-success">Approved</span>
							{:else}
								<span class="badge variant-filled-error">Rejected</span>
							{/if}
						</td>
						<td>{formatDate(agent.last_seen_at)}</td>
						<td>
							<div class="actions-menu">
								<button
									class="btn btn-sm variant-soft"
									onclick={(e) => {
										e.stopPropagation();
										toggleMenu(agent.id, e.currentTarget);
									}}
								>
									&#8943;
								</button>
							</div>
						</td>
					</tr>
				{:else}
					<tr>
						<td colspan="6" class="text-center">No agents found.</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>

	{#if openMenuId}
		{@const agent = agents.find((a) => a.id === openMenuId)}
		{#if agent}
			<div
				class="card fixed z-50 w-40 overflow-hidden p-0 shadow-xl"
				style="top: {menuPos.top}px; left: {menuPos.left}px;"
			>
				<nav class="list-nav">
					<ul>
						{#if agent.status === 'pending'}
							<li>
								<button
									class="w-full text-left text-success-500"
									onclick={() => requestConfirm(agent.id, 'approve', agent.friendly_name)}
								>
									Approve
								</button>
							</li>
							<li>
								<button
									class="w-full text-left text-error-500"
									onclick={() => requestConfirm(agent.id, 'reject', agent.friendly_name)}
								>
									Reject
								</button>
							</li>
						{:else}
							<li>
								<button
									class="w-full text-left text-error-500"
									onclick={() => requestConfirm(agent.id, 'delete', agent.friendly_name)}
								>
									Delete
								</button>
							</li>
						{/if}
					</ul>
				</nav>
			</div>
		{/if}
	{/if}

	{#if confirmAction}
		{@const labels = confirmLabels[confirmAction.action]}
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div
			class="fixed inset-0 z-50 flex items-center justify-center bg-surface-backdrop-token p-4"
			onkeydown={(e) => { if (e.key === 'Escape') cancelConfirm(); }}
		>
			<div class="card w-full max-w-md space-y-4 p-6 shadow-xl">
				<h3 class="h3">{labels.title}</h3>
				<p>
					Are you sure you want to {labels.verb}
					<strong>{confirmAction.name}</strong>?
				</p>
				<div class="flex justify-end gap-2">
					<button class="btn variant-ghost-surface" onclick={cancelConfirm}>Cancel</button>
					<button class="btn {labels.btnClass}" onclick={executeConfirmed}>
						{labels.title}
					</button>
				</div>
			</div>
		</div>
	{/if}
{/if}
