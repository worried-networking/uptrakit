<script lang="ts">
	import { browser } from '$app/environment';
	import { onMount } from 'svelte';
	import { KeyRound } from 'lucide-svelte';
	import { listOAuthClients, revokeOAuthClient, trustOAuthClient } from '$lib/api/oauth';
	import type { OAuthClient } from '$lib/api/oauth';
	import { getUser } from '$lib/auth.svelte';
	import { hasPermissionValue, Permission } from '$lib/types';
	import {
		Callout,
		DataTable,
		EmptyState,
		ModalShell,
		PageShell,
		SectionCard,
		StatusBadge,
		type DataTableColumn
	} from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';
	import RegisterClientDialog from './RegisterClientDialog.svelte';

	const user = getUser();
	const canManage = hasPermissionValue(user, Permission.ManageAuthSettings);
	const canManageGlobalSettings = hasPermissionValue(user, Permission.ManageGlobalSettings);

	let clients: OAuthClient[] = $state([]);
	let loading: boolean = $state(true);
	let loadError: string | null = $state(null);
	let actionError: string | null = $state(null);
	let processing: string | null = $state(null);
	let showRegisterDialog: boolean = $state(false);
	let detailClient = $state<OAuthClient | null>(null);

	// OAuth settings state
	let mcpEnabled = $state(false);
	let dcrEnabled = $state(false);
	let cimdEnabled = $state(false);
	let settingsError = $state<string | null>(null);
	let savingSettings = $state(false);

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

	async function handleMcpToggle() {
		// Pre-flight: warn when enabling (canonical_host check is deferred to Plan D)
		// For now just reflect the toggle — actual save deferred until Plan D ships
		settingsError = null;
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
									<Button
										variant="ghost"
										size="sm"
										onclick={() => {
											detailClient = client;
										}}>View details</Button
									>
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

		{#if browser && import.meta.env.VITE_OAUTH_TOGGLE_PREVIEW}
			{#if canManageGlobalSettings}
				<SectionCard title="OAuth settings">
					<div class="space-y-4">
						<label class="flex items-center gap-3">
							<input
								type="checkbox"
								bind:checked={mcpEnabled}
								disabled={savingSettings}
								class="h-4 w-4"
								onchange={() => handleMcpToggle()}
							/>
							<span class="text-sm text-[var(--text-primary)]">Enable MCP OAuth (master switch)</span>
						</label>
						{#if mcpEnabled}
							<Callout
								tone="warning"
								message="Ensure oauth.canonical_host is configured before enabling MCP OAuth. Tokens minted without a canonical host will be invalid."
							/>
						{/if}
						<label class="flex items-center gap-3">
							<input
								type="checkbox"
								bind:checked={dcrEnabled}
								disabled={savingSettings || !mcpEnabled}
								class="h-4 w-4"
							/>
							<span class="text-sm text-[var(--text-primary)]">Enable Dynamic Client Registration (DCR)</span>
						</label>
						<label class="flex items-center gap-3">
							<input
								type="checkbox"
								bind:checked={cimdEnabled}
								disabled={savingSettings || !mcpEnabled}
								class="h-4 w-4"
							/>
							<span class="text-sm text-[var(--text-primary)]">Enable Client Initiated Metadata Discovery (CIMD)</span>
						</label>
						{#if settingsError}
							<Callout tone="danger" message={settingsError} />
						{/if}
					</div>
				</SectionCard>
			{/if}
		{/if}
	{/if}
</PageShell>

{#if detailClient !== null}
	{@const dc = detailClient}
	<ModalShell
		title="Client details"
		onclose={() => {
			detailClient = null;
		}}
		maxWidth="max-w-lg"
	>
		<div class="space-y-3 text-sm">
			<div>
				<span class="text-[var(--text-muted)]">Client ID</span>
				<p class="font-mono text-xs text-[var(--text-primary)] break-all">{dc.id}</p>
			</div>
			<div>
				<span class="text-[var(--text-muted)]">Redirect URIs</span>
				<ul class="mt-1 space-y-1">
					{#each dc.redirect_uris as uri (uri)}
						<li class="font-mono text-xs text-[var(--text-primary)] break-all">{uri}</li>
					{/each}
				</ul>
			</div>
			{#if dc.client_uri}
				<div>
					<span class="text-[var(--text-muted)]">Client URI</span>
					<p class="text-[var(--text-primary)]">{dc.client_uri}</p>
				</div>
			{/if}
			<div>
				<span class="text-[var(--text-muted)]">Created</span>
				<p class="text-[var(--text-primary)]">{new Date(dc.created_at).toLocaleString()}</p>
			</div>
		</div>
	</ModalShell>
{/if}

<RegisterClientDialog
	open={showRegisterDialog}
	onClose={() => {
		showRegisterDialog = false;
	}}
	onSuccess={(client) => {
		handleRegistered(client);
	}}
/>
