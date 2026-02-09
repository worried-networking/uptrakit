<script lang="ts">
	import { onMount } from 'svelte';
	import { user } from '$lib/auth';
	import { getAgents, approveAgent, rejectAgent, deleteAgent, mergeAgent } from '$lib/api';
	import type { ServiceResponse } from '$lib/types';
	import { formatDate } from '$lib/utils';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import ModalBackdrop from '$lib/components/ModalBackdrop.svelte';
	import Pagination from '$lib/components/Pagination.svelte';

	let agents: ServiceResponse[] = $state([]);
	let error: string | null = $state(null);
	let openMenuId: string | null = $state(null);
	let menuPos: { top: number; left: number } = $state({ top: 0, left: 0 });
	let confirmAction: { agentId: string; action: 'approve' | 'reject' | 'delete'; name: string } | null =
		$state(null);
	let mergeSource: { id: string; name: string } | null = $state(null);
	let mergeTargetId: string | null = $state(null);
	let submitting: boolean = $state(false);
	let currentPage: number = $state(1);
	let totalPages: number = $state(1);

	onMount(() => loadAgents(1));

	async function loadAgents(page: number) {
		try {
			error = null;
			const result = await getAgents(undefined, page);
			agents = result.items;
			currentPage = result.page;
			totalPages = result.total_pages;
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

	function openMergeDialog(agent: ServiceResponse) {
		closeMenu();
		mergeSource = { id: agent.id, name: agent.friendly_name };
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
			await mergeAgent(mergeTargetId, sourceId);
			agents = agents.filter((a) => a.id !== sourceId);
			mergeSource = null;
			mergeTargetId = null;
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to merge agent';
		} finally {
			submitting = false;
		}
	}

	async function executeConfirmed() {
		if (!confirmAction || submitting) return;
		const { agentId, action } = confirmAction;
		confirmAction = null;
		submitting = true;

		try {
			error = null;
			if (action === 'approve') {
				const updated = await approveAgent(agentId);
				agents = agents.map((a) => (a.id === agentId ? updated : a));
			} else if (action === 'reject') {
				await rejectAgent(agentId);
				agents = agents.filter((a) => a.id !== agentId);
			} else if (action === 'delete') {
				await deleteAgent(agentId);
				agents = agents.filter((a) => a.id !== agentId);
			}
		} catch (e) {
			error = e instanceof Error ? e.message : `Failed to ${action} agent`;
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
		approve: { title: 'Approve Agent', verb: 'approve', btnClass: 'preset-filled-success-500' },
		reject: { title: 'Reject Agent', verb: 'reject', btnClass: 'preset-filled-error-500' },
		delete: { title: 'Delete Agent', verb: 'permanently delete', btnClass: 'preset-filled-error-500' }
	} as const;
</script>

<svelte:window onclick={handleWindowClick} onkeydown={(e) => {
	if (e.key === 'Escape') {
		if (confirmAction) cancelConfirm();
		else if (mergeSource) cancelMerge();
	}
}} />

{#if $user}
	<h1 class="h1 mb-6">Agents</h1>

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
						<td>{agent.ip_address ?? '\u2014'}</td>
						<td>
							{#if agent.status === 'pending'}
								<span class="badge preset-filled-warning-500">Pending</span>
							{:else if agent.status === 'approved'}
								<span class="badge preset-filled-success-500">Approved</span>
							{:else if agent.status === 'deactivated'}
								<span class="badge preset-tonal">Deactivated</span>
							{:else}
								<span class="badge preset-filled-error-500">Rejected</span>
							{/if}
						</td>
						<td>{formatDate(agent.last_seen_at)}</td>
						<td>
							<div class="actions-menu">
								<button
									class="btn btn-sm preset-tonal"
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
						<td colspan="6" class="text-center py-8">
							<p class="text-lg font-medium">No agents registered yet</p>
							<p class="mt-1 text-sm text-surface-500">
								Agents connect automatically when you run the Uptrakit agent on a host using an enrollment token.
							</p>
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>

	<Pagination {currentPage} {totalPages} onPageChange={loadAgents} />

	{#if openMenuId}
		{@const agent = agents.find((a) => a.id === openMenuId)}
		{#if agent}
			<ContextMenu top={menuPos.top} left={menuPos.left}>
				{#if agent.status === 'pending'}
					<li>
						<button
							class="w-full rounded-md px-3 py-2 text-left text-sm hover:bg-surface-200 dark:hover:bg-surface-800"
							role="menuitem"
							onclick={() => openMergeDialog(agent)}
						>
							Merge Into&hellip;
						</button>
					</li>
					<li>
						<button
							class="w-full rounded-md px-3 py-2 text-left text-sm text-success-500 hover:bg-surface-200 dark:hover:bg-surface-800"
							role="menuitem"
							onclick={() => requestConfirm(agent.id, 'approve', agent.friendly_name)}
						>
							Approve
						</button>
					</li>
					<li>
						<button
							class="w-full rounded-md px-3 py-2 text-left text-sm text-error-500 hover:bg-surface-200 dark:hover:bg-surface-800"
							role="menuitem"
							onclick={() => requestConfirm(agent.id, 'reject', agent.friendly_name)}
						>
							Reject
						</button>
					</li>
				{:else}
					<li>
						<button
							class="w-full rounded-md px-3 py-2 text-left text-sm text-error-500 hover:bg-surface-200 dark:hover:bg-surface-800"
							role="menuitem"
							onclick={() => requestConfirm(agent.id, 'delete', agent.friendly_name)}
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
			message="Are you sure you want to {labels.verb} <strong>{confirmAction.name}</strong>?"
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
				<h3 class="h3">Merge Agent</h3>
				<p>
					Merge <strong>{mergeSource.name}</strong> into an existing agent.
					The source agent's enrollment will be transferred to the target, preserving the target's history.
				</p>
				<label class="label">
					<span>Select target agent</span>
					<select class="select" bind:value={mergeTargetId}>
						<option value={null}>-- Select an agent --</option>
						{#each agents.filter((a) => a.status === 'approved' && a.id !== mergeSource?.id) as target (target.id)}
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
