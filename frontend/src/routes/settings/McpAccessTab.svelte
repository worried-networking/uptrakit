<script lang="ts">
	import { onMount } from 'svelte';
	import { KeyRound } from 'lucide-svelte';
	import {
		listOAuthClients,
		revokeOAuthClient,
		trustOAuthClient,
		getOAuthSettings,
		updateOAuthSettings
	} from '$lib/api/oauth';
	import type { OAuthClient, OAuthSettingsResponse } from '$lib/api/oauth';
	import { getUser } from '$lib/auth.svelte';
	import { hasPermissionValue, Permission } from '$lib/types';
	import {
		Callout,
		DataTable,
		EmptyState,
		ModalShell,
		SectionCard,
		StatusBadge,
		type DataTableColumn
	} from '$lib/components/ui';
	import { Checkbox, FormFieldRow, Input } from '$lib/components/forms';
	import Button from '$lib/components/Button.svelte';
	import { createFormDraft } from '$lib/forms/draft.svelte';
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
	let oauthSettings = $state<OAuthSettingsResponse | null>(null);
	let oauthSettingsEtag = $state<string | null>(null);
	let settingsLoading = $state(false);
	let settingsError = $state<string | null>(null);
	let savingSettings = $state(false);
	let settingsLoaded = $state(false);

	type DraftOAuthSettings = {
		mcp_enabled: boolean;
		dcr_enabled: boolean;
		cimd_enabled: boolean;
		canonical_host: string | null;
	};

	const oauthDraft = createFormDraft<DraftOAuthSettings>({
		mcp_enabled: false,
		dcr_enabled: false,
		cimd_enabled: false,
		canonical_host: null
	});

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
		if (canManageGlobalSettings) void loadOAuthSettings();
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

	function settingsToOAuthDraft(data: OAuthSettingsResponse): DraftOAuthSettings {
		return {
			mcp_enabled: data.mcp_enabled,
			dcr_enabled: data.dcr_enabled,
			cimd_enabled: data.cimd_enabled,
			canonical_host: data.canonical_host ?? null
		};
	}

	async function loadOAuthSettings() {
		settingsLoading = true;
		settingsError = null;
		try {
			const result = await getOAuthSettings();
			oauthSettings = result.data;
			oauthSettingsEtag = result.etag;
			oauthDraft.load(settingsToOAuthDraft(result.data));
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
			const result = await updateOAuthSettings(
				{ ...patch, canonical_host: patch.canonical_host === '' ? undefined : (patch.canonical_host ?? undefined) },
				oauthSettingsEtag
			);
			oauthSettings = result.data;
			oauthSettingsEtag = result.etag;
			oauthDraft.commit(settingsToOAuthDraft(result.data));
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

	{#if canManageGlobalSettings}
		<SectionCard title="OAuth settings">
			{#if settingsLoading}
				<p class="py-4 text-center text-sm text-[var(--text-secondary)]">Loading…</p>
			{:else if settingsLoaded && oauthSettings !== null}
				<div class="space-y-4">
					<label class="flex items-center gap-3" for="mcp_enabled">
						<Checkbox id="mcp_enabled" bind:checked={oauthDraft.draft.mcp_enabled} disabled={savingSettings} />
						<span class="text-sm text-[var(--text-primary)]">Enable MCP OAuth (master switch)</span>
					</label>

					{#if oauthDraft.draft.mcp_enabled && !oauthDraft.draft.canonical_host}
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

					<FormFieldRow
						label="Canonical host"
						hint="Required when MCP OAuth is enabled (e.g. auth.example.com)"
						inputId="canonical_host"
					>
						<Input
							id="canonical_host"
							type="text"
							value={oauthDraft.draft.canonical_host ?? ''}
							placeholder="auth.example.com"
							disabled={savingSettings}
							oninput={(e) => {
								const v = (e.currentTarget as HTMLInputElement).value.trim();
								oauthDraft.update('canonical_host', v === '' ? null : v);
							}}
						/>
					</FormFieldRow>

					{#if oauthSettings.restart_required}
						<Callout tone="info" message="Settings saved. Changes take effect after the controller is restarted." />
					{/if}

					{#if settingsError}
						<Callout tone="danger" message={settingsError} />
					{/if}

					<div class="flex gap-2">
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
