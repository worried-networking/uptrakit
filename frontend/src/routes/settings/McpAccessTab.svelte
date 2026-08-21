<script lang="ts">
	import { onMount } from 'svelte';
	import { KeyRound } from '@lucide/svelte';
	import { getOAuthSettings, updateOAuthSettings } from '$lib/api/oauth';
	import type { OAuthSettingsResponse } from '$lib/api/oauth';
	import { listClients, revokeClient, trustClient } from '$lib/api';
	import type { OAuthClientResponse } from '$lib/api';
	import { getUser } from '$lib/auth.svelte';
	import { hasAction, Actions } from '$lib/api';
	import {
		Callout,
		DataTable,
		EmptyState,
		ModalShell,
		SectionCard,
		StatusBadge,
		type DataTableColumn
	} from '$lib/components/ui';
	import { Checkbox } from '$lib/components/forms';
	import Button from '$lib/components/Button.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import { createFormDraft } from '$lib/forms/draft.svelte';
	import RegisterClientDialog from './RegisterClientDialog.svelte';

	const canManage = $derived(hasAction(getUser(), Actions.SETTINGS_AUTH_MANAGE));
	const canManageGlobalSettings = $derived(hasAction(getUser(), Actions.SYSTEM_SETTINGS_MANAGE));

	let clients: OAuthClientResponse[] = $state([]);
	let loading: boolean = $state(true);
	let loadError: string | null = $state(null);
	let actionError: string | null = $state(null);
	let processing: string | null = $state(null);
	let showRegisterDialog: boolean = $state(false);
	let detailClient = $state<OAuthClientResponse | null>(null);
	let confirmRevoke = $state<{ id: string; name: string } | null>(null);

	// OAuth settings state
	let oauthSettings = $state<OAuthSettingsResponse | null>(null);
	let settingsLoading = $state(false);
	let settingsError = $state<string | null>(null);
	let savingSettings = $state(false);
	let settingsLoaded = $state(false);

	type DraftOAuthSettings = {
		mcp_enabled: boolean;
		dcr_enabled: boolean;
		cimd_enabled: boolean;
	};

	const oauthDraft = createFormDraft<DraftOAuthSettings>({
		mcp_enabled: false,
		dcr_enabled: false,
		cimd_enabled: false
	});

	const columns: DataTableColumn[] = [
		{ key: 'client_name', label: 'Name', mobileTitle: true },
		{ key: 'source', label: 'Source' },
		{ key: 'status', label: 'Status' },
		{ key: 'created', label: 'Created' },
		{ key: 'actions', label: '' }
	];

	onMount(() => {
		if (canManage) void loadClients();
		if (canManageGlobalSettings) void loadOAuthSettings();
	});

	async function loadClients() {
		loading = true;
		loadError = null;
		try {
			// ponytail: fixed per_page 100, add pagination UI when a deployment exceeds it
			const { data } = await listClients({ query: { page: 1, per_page: 100 } });
			clients = data.items;
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
			await trustClient({ path: { client_id: id } });
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
			await revokeClient({ path: { client_id: id } });
			clients = clients.map((c) => (c.id === id ? { ...c, revoked_at: new Date().toISOString() } : c));
		} catch (e) {
			actionError = e instanceof Error ? e.message : 'Failed to revoke client';
		} finally {
			processing = null;
			confirmRevoke = null;
		}
	}

	function formatDate(iso: string): string {
		return new Date(iso).toLocaleDateString();
	}

	function clientStatus(client: OAuthClientResponse): { tone: 'neutral' | 'danger' | 'warning'; label: string } {
		if (client.revoked_at) return { tone: 'neutral', label: 'Revoked' };
		if (!client.trusted_at) return { tone: 'danger', label: 'Unverified' };
		if (client.created_via === 'dcr') return { tone: 'warning', label: 'DCR' };
		if (client.created_via === 'cimd_cache') return { tone: 'neutral', label: 'CIMD' };
		return { tone: 'neutral', label: 'Manual' };
	}

	function sourceLabel(via: string): string {
		if (via === 'dcr') return 'Dynamic Registration';
		if (via === 'cimd_cache') return 'CIMD Cache';
		return 'Manual';
	}

	function settingsToOAuthDraft(data: OAuthSettingsResponse): DraftOAuthSettings {
		return {
			mcp_enabled: data.mcp_enabled,
			dcr_enabled: data.dcr_enabled,
			cimd_enabled: data.cimd_enabled
		};
	}

	async function loadOAuthSettings() {
		settingsLoading = true;
		settingsError = null;
		try {
			const result = await getOAuthSettings();
			oauthSettings = result;
			oauthDraft.load(settingsToOAuthDraft(result));
			settingsLoaded = true;
		} catch (e) {
			settingsError = e instanceof Error ? e.message : 'Failed to load OAuth settings';
		} finally {
			settingsLoading = false;
		}
	}

	async function handleSettingsChange() {
		savingSettings = true;
		settingsError = null;
		try {
			const patch = oauthDraft.draft;
			const result = await updateOAuthSettings(patch);
			oauthSettings = result;
			oauthDraft.commit(settingsToOAuthDraft(result));
		} catch (e) {
			settingsError = e instanceof Error ? e.message : 'Failed to save OAuth settings';
		} finally {
			savingSettings = false;
		}
	}
</script>

{#if !canManage}
	<Callout tone="danger" message="You don't have permission to manage OAuth clients." />
{:else}
	{#if actionError}
		<Callout tone="danger" message={actionError} />
	{/if}

	<SectionCard title="Registered Clients">
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
				rowKey={(row) => (row as unknown as OAuthClientResponse).id}
			>
				{#snippet row(rowValue, _index)}
					{@const client = rowValue as unknown as OAuthClientResponse}
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
						<td class="table-cell-pad">
							<div class="flex flex-wrap gap-2">
								<Button
									variant="ghost"
									size="sm"
									onclick={() => {
										detailClient = client;
									}}>View details</Button
								>
								{#if !client.trusted_at && !client.revoked_at}
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
								{#if !client.revoked_at}
									<Button
										variant="danger"
										size="sm"
										loading={processing === client.id}
										disabled={processing !== null}
										onclick={() => {
											confirmRevoke = { id: client.id, name: client.client_name };
										}}
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

	{#if canManageGlobalSettings}
		<SectionCard title="OAuth Settings">
			{#if settingsLoading}
				<p class="py-4 text-center text-sm text-[var(--text-secondary)]">Loading…</p>
			{:else if settingsLoaded && oauthSettings !== null}
				<div class="space-y-4">
					<label class="flex items-center gap-3" for="mcp_enabled">
						<Checkbox id="mcp_enabled" bind:checked={oauthDraft.draft.mcp_enabled} disabled={savingSettings} />
						<span class="text-sm text-[var(--text-primary)]">Enable MCP OAuth (master switch)</span>
					</label>

					<p class="text-sm text-[var(--text-secondary)]">
						Canonical host is configured under Global Settings → Canonical Host.
					</p>

					{#if oauthDraft.draft.mcp_enabled && !oauthSettings?.canonical_host}
						<Callout
							tone="warning"
							message="oauth.canonical_host must be set before enabling MCP OAuth. Tokens minted without a canonical host will be invalid."
						/>
					{/if}

					<label class="flex items-center gap-3" for="dcr_enabled">
						<Checkbox
							id="dcr_enabled"
							bind:checked={oauthDraft.draft.dcr_enabled}
							disabled={savingSettings || !oauthDraft.draft.mcp_enabled}
						/>
						<span class="text-sm text-[var(--text-primary)]">Enable Dynamic Client Registration (DCR)</span>
					</label>

					<label class="flex items-center gap-3" for="cimd_enabled">
						<Checkbox
							id="cimd_enabled"
							bind:checked={oauthDraft.draft.cimd_enabled}
							disabled={savingSettings || !oauthDraft.draft.mcp_enabled}
						/>
						<span class="text-sm text-[var(--text-primary)]">Enable Client-Initiated Metadata Discovery (CIMD)</span>
					</label>

					{#if oauthSettings.restart_required}
						<Callout tone="info" message="Settings saved. Changes take effect after the controller is restarted." />
					{/if}

					{#if settingsError}
						<Callout tone="danger" message={settingsError} />
					{/if}

					<div class="flex gap-2 justify-end">
						<Button
							variant="primary"
							disabled={!oauthDraft.isDirty || savingSettings}
							loading={savingSettings}
							onclick={() => void handleSettingsChange()}
						>
							Save
						</Button>
						{#if oauthDraft.isDirty}
							<Button variant="ghost" disabled={savingSettings} onclick={() => oauthDraft.discard()}>Discard</Button>
						{/if}
					</div>
				</div>
			{:else if settingsError}
				<Callout tone="danger" message={settingsError} />
			{/if}
		</SectionCard>
	{/if}
{/if}

{#if detailClient !== null}
	{@const dc = detailClient}
	<ModalShell
		title="Client Details"
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
	onSuccess={() => void loadClients()}
/>

{#if confirmRevoke}
	<ConfirmDialog
		title="Revoke OAuth Client"
		messagePrefix="Are you sure you want to revoke"
		entityName={confirmRevoke.name}
		confirmLabel="Revoke"
		confirmDisabled={processing !== null}
		onconfirm={() => {
			if (confirmRevoke) void handleRevoke(confirmRevoke.id);
		}}
		oncancel={() => {
			confirmRevoke = null;
		}}
	/>
{/if}
