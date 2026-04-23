<script lang="ts">
	import { onMount } from 'svelte';
	import { getUser } from '$lib/auth.svelte';
	import { listApiTokens, createApiToken, revokeApiToken } from '$lib/api';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import { formatDate } from '$lib/utils';
	import type { ApiTokenResponse } from '$lib/types';
	import {
		Callout,
		DataTable,
		FormFieldRow,
		ModalShell,
		PageShell,
		SectionCard,
		StatusBadge
	} from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';
	import Input from '$lib/components/Input.svelte';

	const user = $derived(getUser());

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

{#if user}
	<PageShell title="Profile" description="Manage your account information and API access tokens.">
		<SectionCard title="Account">
			<div class="grid gap-3 sm:grid-cols-2" data-ui="profile-account-details">
				<div class="rounded-panel border border-[var(--border-subtle)] bg-[var(--bg-raised)] px-4 py-3">
					<p class="text-xs font-semibold uppercase tracking-table-header text-[var(--text-muted)]">Name</p>
					<p class="mt-1 text-sm font-medium text-[var(--text-primary)]">{user.first_name} {user.last_name}</p>
				</div>
				<div class="rounded-panel border border-[var(--border-subtle)] bg-[var(--bg-raised)] px-4 py-3">
					<p class="text-xs font-semibold uppercase tracking-table-header text-[var(--text-muted)]">Email</p>
					<p class="mt-1 text-sm font-medium text-[var(--text-primary)]">{user.email}</p>
				</div>
			</div>
		</SectionCard>

		<SectionCard
			title="API Tokens"
			description="API tokens allow programmatic access to Uptrakit. Treat tokens like passwords and rotate them regularly."
		>
			{#snippet actions()}
				<Button variant="primary" onclick={openCreateModal}>New Token</Button>
			{/snippet}

			<DataTable
				columns={[]}
				rows={tokens as unknown as Record<string, unknown>[]}
				{loading}
				emptyTitle="No API tokens yet."
				rowKey={(row) => (row as unknown as ApiTokenResponse).id}
			>
				{#snippet header()}
					<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
						<th class="px-4 py-3 text-left text-table-header font-semibold uppercase tracking-table-header" scope="col"
							>Name</th
						>
						<th class="px-4 py-3 text-left text-table-header font-semibold uppercase tracking-table-header" scope="col"
							>Created</th
						>
						<th class="px-4 py-3 text-left text-table-header font-semibold uppercase tracking-table-header" scope="col"
							>Status</th
						>
						<th
							class="w-24 px-4 py-3 text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col"
						></th>
					</tr>
				{/snippet}
				{#snippet row(rowValue)}
					{@const token = rowValue as unknown as ApiTokenResponse}
					<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
						<td class="px-4 py-3 text-[var(--text-primary)]">{token.name}</td>
						<td class="px-4 py-3 text-[var(--text-primary)]">{formatDate(token.created_at)}</td>
						<td class="px-4 py-3 text-[var(--text-primary)]">
							{#if token.revoked_at}
								<StatusBadge tone="neutral" label="Revoked" />
							{:else}
								<StatusBadge tone="success" label="Active" />
							{/if}
						</td>
						<td class="px-4 py-3 text-[var(--text-primary)]">
							{#if !token.revoked_at}
								<Button variant="danger" size="sm" onclick={() => (revokeConfirm = { id: token.id, name: token.name })}>
									Revoke
								</Button>
							{/if}
						</td>
					</tr>
				{/snippet}
			</DataTable>
		</SectionCard>
	</PageShell>
{/if}

{#if revokeConfirm}
	<ConfirmDialog
		title="Revoke API Token"
		messagePrefix="Are you sure you want to revoke"
		entityName={revokeConfirm.name}
		confirmLabel={revoking ? 'Revoking...' : 'Revoke'}
		confirmDisabled={revoking}
		onconfirm={handleRevoke}
		oncancel={() => (revokeConfirm = null)}
	/>
{/if}

{#if showCreateModal}
	<ModalShell title="New API Token" onclose={closeCreateModal} maxWidth="max-w-lg">
		{#if createdToken}
			<Callout
				tone="warning"
				title="Save this token now"
				message="It will not be shown again after you close this dialog."
			/>
			<div class="relative">
				<pre
					class="rounded-panel bg-[var(--bg-raised)] p-3 font-mono text-sm break-all whitespace-pre-wrap">{createdToken}</pre>
			</div>
		{:else}
			<FormFieldRow label="Token Name">
				<Input
					id="new-token-name"
					type="text"
					placeholder="e.g. CI Pipeline"
					bind:value={newTokenName}
					onkeydown={(e) => {
						if (e.key === 'Enter') handleCreate();
					}}
				/>
			</FormFieldRow>
		{/if}
		{#snippet footer()}
			<div class="contents" data-ui="profile-token-modal-footer">
				{#if createdToken}
					<Button variant="secondary" onclick={() => copyToken(createdToken!)}>Copy</Button>
					<Button variant="primary" onclick={closeCreateModal}>Done</Button>
				{:else}
					<Button variant="secondary" onclick={closeCreateModal}>Cancel</Button>
					<Button variant="primary" onclick={handleCreate} disabled={!newTokenName.trim()} loading={creating}>
						Create
					</Button>
				{/if}
			</div>
		{/snippet}
	</ModalShell>
{/if}
