<script lang="ts">
	import { onMount } from 'svelte';
	import { KeyRound } from 'lucide-svelte';
	import { listOAuthClients, revokeOAuthClient, trustOAuthClient } from '$lib/api/oauth';
	import type { OAuthClient } from '$lib/api/oauth';
	import { getUser } from '$lib/auth.svelte';
	import { hasPermissionValue } from '$lib/types';
	import {
		Callout,
		DataTable,
		EmptyState,
		PageShell,
		SectionCard,
		StatusBadge,
		type DataTableColumn
	} from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';
	import RegisterClientDialog from './RegisterClientDialog.svelte';

	const user = getUser();
	const canManage = hasPermissionValue(user, 'manage_auth_settings');

	let clients: OAuthClient[] = $state([]);
	let loading: boolean = $state(true);
	let loadError: string | null = $state(null);
	let actionError: string | null = $state(null);
	let processing: string | null = $state(null);
	let showRegisterDialog: boolean = $state(false);

	const columns: DataTableColumn[] = [
		{ key: 'client_name', label: 'Name', mobileTitle: true },
		{ key: 'source', label: 'Source' },
		{ key: 'status', label: 'Status' },
		{ key: 'created', label: 'Created' },
		{ key: 'last_used', label: 'Last used' },
		{ key: 'actions', label: '' }
	];

	onMount(() => {
		if (canManage) void loadClients();
	});

	async function loadClients() {
		loading = true;
		loadError = null;
		try {
			clients = await listOAuthClients();
		} catch (e) {
			loadError = e instanceof Error ? e.message : 'Failed to load OAuth clients';
		} finally {
			loading = false;
		}
	}

	async function handleTrust(id: string) {
		processing = id;
		actionError = null;
		try {
			await trustOAuthClient(id);
			clients = clients.map((c) => (c.id === id ? { ...c, trusted_at: new Date().toISOString() } : c));
		} catch (e) {
			actionError = e instanceof Error ? e.message : 'Failed to trust client';
		} finally {
			processing = null;
		}
	}

	async function handleRevoke(id: string) {
		processing = id;
		actionError = null;
		try {
			await revokeOAuthClient(id);
			clients = clients.map((c) => (c.id === id ? { ...c, revoked_at: new Date().toISOString() } : c));
		} catch (e) {
			actionError = e instanceof Error ? e.message : 'Failed to revoke client';
		} finally {
			processing = null;
		}
	}

	function handleRegistered(client: OAuthClient) {
		clients = [client, ...clients];
	}

	function formatDate(iso: string): string {
		return new Date(iso).toLocaleDateString();
	}

	function clientStatus(client: OAuthClient): { tone: 'neutral' | 'danger' | 'warning'; label: string } {
		if (client.revoked_at !== null) return { tone: 'neutral', label: 'Revoked' };
		if (client.trusted_at === null) return { tone: 'danger', label: 'Unverified' };
		if (client.created_via === 'dcr') return { tone: 'warning', label: 'DCR' };
		if (client.created_via === 'cimd_cache') return { tone: 'neutral', label: 'CIMD' };
		return { tone: 'neutral', label: 'Manual' };
	}

	function sourceLabel(via: OAuthClient['created_via']): string {
		if (via === 'dcr') return 'Dynamic Registration';
		if (via === 'cimd_cache') return 'CIMD Cache';
		return 'Manual';
	}
</script>

<PageShell title="OAuth Clients">
	{#if !canManage}
		<Callout tone="danger" message="You don't have permission to manage OAuth clients." />
	{:else}
		{#if actionError}
			<Callout tone="danger" message={actionError} />
		{/if}

		<SectionCard title="Registered clients">
			{#snippet actions()}
				<Button
					variant="primary"
					onclick={() => {
						showRegisterDialog = true;
					}}
				>
					Register
				</Button>
			{/snippet}

			{#if loading}
				<p class="py-8 text-center text-sm text-[var(--text-secondary)]">Loading...</p>
			{:else if loadError}
				<Callout tone="danger" title="Unable to load data" message={loadError} />
			{:else if clients.length === 0}
				<EmptyState
					icon={KeyRound}
					title="No OAuth clients registered"
					description="Register a client manually or clients will appear here after Dynamic Client Registration."
				/>
			{:else}
				<DataTable
					{columns}
					rows={clients as unknown as Record<string, unknown>[]}
					rowKey={(row) => (row as unknown as OAuthClient).id}
				>
					{#snippet row(rowValue, _index)}
						{@const client = rowValue as unknown as OAuthClient}
						{@const status = clientStatus(client)}
						<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
							<td class="table-cell-pad text-[var(--text-primary)]">
								{#if client.client_uri}
									<a
										href={client.client_uri}
										target="_blank"
										rel="noopener noreferrer"
										class="underline hover:text-[var(--accent)]"
									>
										{client.client_name}
									</a>
								{:else}
									{client.client_name}
								{/if}
							</td>
							<td class="table-cell-pad text-[var(--text-secondary)]">{sourceLabel(client.created_via)}</td>
							<td class="table-cell-pad">
								<StatusBadge tone={status.tone} label={status.label} />
							</td>
							<td class="table-cell-pad text-[var(--text-primary)]">{formatDate(client.created_at)}</td>
							<td class="table-cell-pad text-[var(--text-primary)]">
								{client.last_used_at ? formatDate(client.last_used_at) : 'Never'}
							</td>
							<td class="table-cell-pad">
								<div class="flex flex-wrap gap-2">
									{#if client.trusted_at === null && client.revoked_at === null}
										<Button
											variant="secondary"
											size="sm"
											loading={processing === client.id}
											disabled={processing !== null}
											onclick={() => void handleTrust(client.id)}
										>
											Trust
										</Button>
									{/if}
									{#if client.revoked_at === null}
										<Button
											variant="danger"
											size="sm"
											loading={processing === client.id}
											disabled={processing !== null}
											onclick={() => void handleRevoke(client.id)}
										>
											Revoke
										</Button>
									{/if}
								</div>
							</td>
						</tr>
					{/snippet}
				</DataTable>
			{/if}
		</SectionCard>
	{/if}
</PageShell>

<RegisterClientDialog
	open={showRegisterDialog}
	onClose={() => {
		showRegisterDialog = false;
	}}
	onSuccess={(client) => {
		handleRegistered(client);
	}}
/>
