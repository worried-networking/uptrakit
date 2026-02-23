<script lang="ts">
	import { onMount } from 'svelte';
	import { getUser } from '$lib/auth.svelte';
	import { listApiTokens, createApiToken, revokeApiToken } from '$lib/api';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import ModalBackdrop from '$lib/components/ModalBackdrop.svelte';
	import { formatDate } from '$lib/utils';
	import type { ApiTokenResponse } from '$lib/types';

	let tokens: ApiTokenResponse[] = $state([]);
	let loading: boolean = $state(true);
	let showCreateModal: boolean = $state(false);
	let newTokenName: string = $state('');
	let creating: boolean = $state(false);
	let createdToken: string | null = $state(null);
	let revokeConfirm: { id: string; name: string } | null = $state(null);
	let revoking: boolean = $state(false);

	onMount(async () => {
		await loadTokens();
	});

	async function loadTokens() {
		loading = true;
		try {
			const res = await listApiTokens();
			tokens = res.tokens;
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load API tokens');
		} finally {
			loading = false;
		}
	}

	function openCreateModal() {
		newTokenName = '';
		createdToken = null;
		showCreateModal = true;
	}

	function closeCreateModal() {
		showCreateModal = false;
		createdToken = null;
		newTokenName = '';
	}

	async function handleCreate() {
		if (!newTokenName.trim() || creating) return;
		creating = true;
		try {
			const res = await createApiToken({ name: newTokenName.trim() });
			tokens = [
				...tokens,
				{ id: res.id, name: newTokenName.trim(), revoked_at: null, created_at: new Date().toISOString() }
			];
			createdToken = res.token;
			newTokenName = '';
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to create API token');
			closeCreateModal();
		} finally {
			creating = false;
		}
	}

	async function handleRevoke() {
		if (!revokeConfirm || revoking) return;
		const { id } = revokeConfirm;
		revokeConfirm = null;
		revoking = true;
		try {
			await revokeApiToken(id);
			tokens = tokens.filter((t) => t.id !== id);
			showSuccess('API token revoked.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to revoke API token');
		} finally {
			revoking = false;
		}
	}

	async function copyToken(token: string) {
		try {
			await navigator.clipboard.writeText(token);
			showSuccess('Token copied to clipboard.');
		} catch {
			showError('Failed to copy token. Please copy it manually.');
		}
	}
</script>

<svelte:window
	onkeydown={(e) => {
		if (e.key === 'Escape') {
			if (showCreateModal) closeCreateModal();
			else if (revokeConfirm) revokeConfirm = null;
		}
	}}
/>

{#if getUser()}
	<h1 class="h1 mb-6">Profile</h1>

	<!-- Profile info -->
	<div class="card mb-6 p-6">
		<h2 class="h3 mb-4">Account</h2>
		<dl class="space-y-2">
			<div class="flex gap-4">
				<dt class="w-32 font-medium text-surface-600 dark:text-surface-400">Name</dt>
				<dd>{getUser()?.first_name} {getUser()?.last_name}</dd>
			</div>
			<div class="flex gap-4">
				<dt class="w-32 font-medium text-surface-600 dark:text-surface-400">Email</dt>
				<dd>{getUser()?.email}</dd>
			</div>
		</dl>
	</div>

	<!-- API Tokens -->
	<div class="card p-6">
		<div class="mb-4 flex items-center justify-between">
			<h2 class="h3">API Tokens</h2>
			<button class="btn preset-filled-primary-500" onclick={openCreateModal}>New Token</button>
		</div>
		<p class="mb-4 text-surface-600 dark:text-surface-400">
			API tokens allow programmatic access to the Uptrakit API. Treat tokens like passwords — do not share them.
		</p>

		{#if loading}
			<p class="text-center py-4">Loading...</p>
		{:else if tokens.length === 0}
			<p class="py-4 text-center text-surface-600 dark:text-surface-400">No API tokens yet.</p>
		{:else}
			<div class="table-wrap">
				<table class="table">
					<thead>
						<tr>
							<th>Name</th>
							<th>Created</th>
							<th>Status</th>
							<th class="w-24"></th>
						</tr>
					</thead>
					<tbody>
						{#each tokens as token (token.id)}
							<tr>
								<td>{token.name}</td>
								<td>{formatDate(token.created_at)}</td>
								<td>
									{#if token.revoked_at}
										<span class="badge preset-tonal">Revoked</span>
									{:else}
										<span class="badge preset-filled-success-500">Active</span>
									{/if}
								</td>
								<td>
									{#if !token.revoked_at}
										<button
											class="btn btn-sm preset-tonal-error"
											onclick={() => (revokeConfirm = { id: token.id, name: token.name })}
										>
											Revoke
										</button>
									{/if}
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}
	</div>
{/if}

{#if revokeConfirm}
	<ConfirmDialog
		title="Revoke API Token"
		messagePrefix="Are you sure you want to revoke"
		entityName={revokeConfirm.name}
		confirmLabel={revoking ? 'Revoking...' : 'Revoke'}
		confirmClass="preset-filled-error-500"
		confirmDisabled={revoking}
		onconfirm={handleRevoke}
		oncancel={() => (revokeConfirm = null)}
	/>
{/if}

{#if showCreateModal}
	<ModalBackdrop onclose={closeCreateModal}>
		<div
			class="card bg-surface-50 dark:bg-surface-900 w-full max-w-lg space-y-4 p-6 shadow-xl"
			role="dialog"
			aria-modal="true"
		>
			<h3 class="h3">New API Token</h3>

			{#if createdToken}
				<aside class="rounded-lg p-4 preset-filled-warning-500">
					<p class="mb-2 font-semibold">Save this token now — it will not be shown again.</p>
				</aside>
				<div class="relative">
					<pre
						class="rounded-md bg-surface-100 dark:bg-surface-800 p-3 font-mono text-sm break-all whitespace-pre-wrap">{createdToken}</pre>
				</div>
				<div class="flex justify-end gap-2">
					<button class="btn preset-tonal-surface" onclick={() => copyToken(createdToken!)}>Copy</button>
					<button class="btn preset-filled-primary-500" onclick={closeCreateModal}>Done</button>
				</div>
			{:else}
				<label class="label">
					<span>Token Name</span>
					<input
						class="input"
						type="text"
						placeholder="e.g. CI Pipeline"
						bind:value={newTokenName}
						onkeydown={(e) => {
							if (e.key === 'Enter') handleCreate();
						}}
					/>
				</label>
				<div class="flex justify-end gap-2">
					<button class="btn preset-tonal-surface" onclick={closeCreateModal}>Cancel</button>
					<button
						class="btn preset-filled-primary-500"
						onclick={handleCreate}
						disabled={!newTokenName.trim() || creating}
					>
						{creating ? 'Creating...' : 'Create'}
					</button>
				</div>
			{/if}
		</div>
	</ModalBackdrop>
{/if}
