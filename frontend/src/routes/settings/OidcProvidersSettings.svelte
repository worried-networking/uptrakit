<script lang="ts">
	import {
		getOidcProviders,
		createOidcProvider,
		updateOidcProvider,
		deleteOidcProvider,
		activateOidcProvider,
		deactivateOidcProvider
	} from '$lib/api';
	import type { OidcProviderResponse, CreateOidcProviderRequest, UpdateOidcProviderRequest } from '$lib/types';
	import { isValidLogoUrl } from '$lib/utils';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import Modal from '$lib/components/Modal.svelte';
	import { getIsOnline } from '$lib/stores/network.svelte';
	import { Callout, DataTable, FormFieldRow, SectionCard, StatusBadge, type DataTableColumn } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';
	import Checkbox from '$lib/components/Checkbox.svelte';
	import Input from '$lib/components/Input.svelte';
	import Textarea from '$lib/components/Textarea.svelte';

	let {
		providers,
		multiTenancyEnabled,
		onSuccess,
		onError
	}: {
		providers: OidcProviderResponse[] | undefined;
		multiTenancyEnabled: boolean;
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let oidcProviders: OidcProviderResponse[] = $state([]);
	let showOidcModal: boolean = $state(false);
	let editingProvider: OidcProviderResponse | null = $state(null);
	let oidcForm = $state({
		name: '',
		slug: '',
		logo_url: '',
		issuer_url: '',
		client_id: '',
		client_secret: '',
		scopes: 'openid email profile groups',
		auto_create_users: true,
		allow_private_network_issuers: true,
		role_claim_path: '',
		role_mapping_json: '{}'
	});
	let slugTouched: boolean = $state(false);
	let deleteConfirm: { id: string; name: string } | null = $state(null);
	let saving: boolean = $state(false);
	let togglingProviderId: string | null = $state(null);

	$effect(() => {
		if (providers !== undefined) {
			oidcProviders = providers;
		}
	});

	function slugify(text: string): string {
		return text
			.toLowerCase()
			.trim()
			.replace(/[^a-z0-9]+/g, '-')
			.replace(/^-+|-+$/g, '');
	}

	function onOidcNameInput() {
		if (!editingProvider && !slugTouched) {
			oidcForm.slug = slugify(oidcForm.name);
		}
	}

	function openCreateOidc() {
		editingProvider = null;
		slugTouched = false;
		oidcForm = {
			name: '',
			slug: '',
			logo_url: '',
			issuer_url: '',
			client_id: '',
			client_secret: '',
			scopes: 'openid email profile groups',
			auto_create_users: true,
			allow_private_network_issuers: !multiTenancyEnabled,
			role_claim_path: '',
			role_mapping_json: '{}'
		};
		showOidcModal = true;
	}

	function openEditOidc(provider: OidcProviderResponse) {
		editingProvider = provider;
		oidcForm = {
			name: provider.name,
			slug: provider.slug,
			logo_url: provider.logo_url ?? '',
			issuer_url: provider.issuer_url,
			client_id: provider.client_id,
			client_secret: '',
			scopes: provider.scopes,
			auto_create_users: provider.auto_create_users,
			allow_private_network_issuers: provider.allow_private_network_issuers,
			role_claim_path: provider.role_claim_path ?? '',
			role_mapping_json: JSON.stringify(provider.role_mapping, null, 2)
		};
		showOidcModal = true;
	}

	function closeOidcModal() {
		showOidcModal = false;
		editingProvider = null;
	}

	async function saveOidcProvider() {
		let roleMapping: Record<string, string>;
		try {
			roleMapping = JSON.parse(oidcForm.role_mapping_json);
		} catch {
			onError('Role mapping must be valid JSON (e.g. {"oidc_value": "local_role"})');
			return;
		}

		saving = true;
		try {
			if (editingProvider) {
				const data: UpdateOidcProviderRequest = {
					name: oidcForm.name,
					slug: oidcForm.slug,
					issuer_url: oidcForm.issuer_url,
					client_id: oidcForm.client_id,
					scopes: oidcForm.scopes,
					auto_create_users: oidcForm.auto_create_users,
					allow_private_network_issuers: oidcForm.allow_private_network_issuers,
					role_mapping: roleMapping
				};
				if (oidcForm.logo_url) data.logo_url = oidcForm.logo_url;
				if (oidcForm.client_secret) data.client_secret = oidcForm.client_secret;
				if (oidcForm.role_claim_path) data.role_claim_path = oidcForm.role_claim_path;

				const updated = await updateOidcProvider(editingProvider.id, data);
				oidcProviders = oidcProviders.map((p) => (p.id === updated.id ? updated : p));
				onSuccess('OIDC provider updated.');
			} else {
				const data: CreateOidcProviderRequest = {
					name: oidcForm.name,
					slug: oidcForm.slug,
					issuer_url: oidcForm.issuer_url,
					client_id: oidcForm.client_id,
					client_secret: oidcForm.client_secret,
					scopes: oidcForm.scopes,
					auto_create_users: oidcForm.auto_create_users,
					allow_private_network_issuers: oidcForm.allow_private_network_issuers,
					role_mapping: roleMapping
				};
				if (oidcForm.logo_url) data.logo_url = oidcForm.logo_url;
				if (oidcForm.role_claim_path) data.role_claim_path = oidcForm.role_claim_path;

				const created = await createOidcProvider(data);
				oidcProviders = [...oidcProviders, created];
				onSuccess('OIDC provider created.');
			}
			closeOidcModal();
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save OIDC provider');
		} finally {
			saving = false;
		}
	}

	function requestDeleteOidc(provider: OidcProviderResponse) {
		deleteConfirm = { id: provider.id, name: provider.name };
	}

	async function executeDeleteOidc() {
		if (!deleteConfirm) return;
		const { id } = deleteConfirm;
		deleteConfirm = null;
		try {
			await deleteOidcProvider(id);
			oidcProviders = oidcProviders.filter((p) => p.id !== id);
			onSuccess('OIDC provider deleted.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to delete OIDC provider');
		}
	}

	async function toggleOidcActive(provider: OidcProviderResponse) {
		togglingProviderId = provider.id;
		try {
			let updated: OidcProviderResponse;
			if (provider.is_active) {
				updated = await deactivateOidcProvider(provider.id);
			} else {
				updated = await activateOidcProvider(provider.id);
			}
			// Activation may deactivate others, so reload all
			oidcProviders = await getOidcProviders();
			onSuccess(updated.is_active ? `${updated.name} activated.` : `${updated.name} deactivated.`);
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to update provider status');
		} finally {
			togglingProviderId = null;
		}
	}

	const oidcColumns: DataTableColumn[] = [
		{ key: 'name', label: 'Name' },
		{ key: 'slug', label: 'Slug' },
		{ key: 'status', label: 'Status' },
		{ key: 'actions', label: 'Actions', align: 'right' }
	];
</script>

<SectionCard title="OIDC Providers">
	<div class="mb-4 flex items-center justify-between">
		<Button variant="primary" onclick={openCreateOidc}>Add Provider</Button>
	</div>

	<DataTable
		columns={oidcColumns}
		rows={oidcProviders as unknown as Record<string, unknown>[]}
		loading={false}
		emptyTitle="No OIDC providers configured."
		rowKey={(row) => (row as unknown as OidcProviderResponse).id}
	>
		{#snippet row(rowValue)}
			{@const provider = rowValue as unknown as OidcProviderResponse}
			<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
				<td class="table-cell-pad">{provider.name}</td>
				<td class="table-cell-pad">{provider.slug}</td>
				<td class="table-cell-pad">
					{#if provider.is_active}
						<StatusBadge tone="success" label="Active" />
					{:else}
						<StatusBadge tone="neutral" label="Inactive" />
					{/if}
				</td>
				<td class="table-cell-pad text-right">
					<div class="flex justify-end gap-1">
						<Button variant="secondary" size="sm" onclick={() => openEditOidc(provider)}>Edit</Button>
						{#if provider.is_active}
							<Button
								variant="secondary"
								size="sm"
								loading={togglingProviderId === provider.id}
								onclick={() => void toggleOidcActive(provider)}>Deactivate</Button
							>
						{:else}
							<Button
								variant="secondary"
								size="sm"
								loading={togglingProviderId === provider.id}
								onclick={() => void toggleOidcActive(provider)}>Activate</Button
							>
						{/if}
						<Button variant="danger" size="sm" onclick={() => requestDeleteOidc(provider)}>Delete</Button>
					</div>
				</td>
			</tr>
		{/snippet}
	</DataTable>
</SectionCard>

{#if deleteConfirm}
	<ConfirmDialog
		title="Delete OIDC Provider"
		messagePrefix="Are you sure you want to delete"
		entityName={deleteConfirm.name}
		confirmLabel="Delete"
		onconfirm={executeDeleteOidc}
		oncancel={() => {
			deleteConfirm = null;
		}}
	/>
{/if}

{#if showOidcModal}
	<Modal
		title={editingProvider ? 'Edit OIDC Provider' : 'Add OIDC Provider'}
		onclose={closeOidcModal}
		maxWidth="max-w-2xl max-h-[90vh] overflow-y-auto"
	>
		<div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
			<FormFieldRow label="Name" inputId="oidc-name">
				<Input id="oidc-name" type="text" bind:value={oidcForm.name} oninput={onOidcNameInput} />
			</FormFieldRow>
			<FormFieldRow label="Slug" inputId="oidc-slug">
				<Input
					id="oidc-slug"
					type="text"
					bind:value={oidcForm.slug}
					oninput={() => {
						slugTouched = true;
					}}
				/>
			</FormFieldRow>
		</div>

		<FormFieldRow label="Logo URL" inputId="oidc-logo-url">
			<Input id="oidc-logo-url" type="url" placeholder="https://..." bind:value={oidcForm.logo_url} />
			{#if oidcForm.logo_url && !isValidLogoUrl(oidcForm.logo_url)}
				<small class="text-[var(--color-danger)]">Logo URL must use HTTPS</small>
			{/if}
		</FormFieldRow>

		<FormFieldRow label="Issuer URL" inputId="oidc-issuer-url">
			<Input id="oidc-issuer-url" type="text" bind:value={oidcForm.issuer_url} />
		</FormFieldRow>

		<div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
			<FormFieldRow label="Client ID" inputId="oidc-client-id">
				<Input id="oidc-client-id" type="text" bind:value={oidcForm.client_id} />
			</FormFieldRow>
			<FormFieldRow label="Client Secret" inputId="oidc-client-secret">
				<Input
					id="oidc-client-secret"
					type="password"
					placeholder={editingProvider ? 'Leave blank to keep current' : ''}
					bind:value={oidcForm.client_secret}
				/>
			</FormFieldRow>
		</div>

		<FormFieldRow label="Scopes" inputId="oidc-scopes">
			<Input id="oidc-scopes" type="text" bind:value={oidcForm.scopes} />
		</FormFieldRow>

		<label class="flex items-center gap-3">
			<Checkbox id="oidc-auto-create-users" bind:checked={oidcForm.auto_create_users} />
			<span>Auto-create users on first login</span>
		</label>

		{#if multiTenancyEnabled}
			<Callout tone="info">
				Private-network OIDC issuers are disabled in multi-tenant mode and cannot be changed.
			</Callout>
		{:else}
			<label class="flex items-start gap-3">
				<Checkbox
					id="oidc-allow-private-network-issuers"
					bind:checked={oidcForm.allow_private_network_issuers}
					class="mt-1"
				/>
				<span>
					Allow private-network issuers
					<small class="block text-[var(--text-secondary)]">
						Permit issuer hostnames that resolve to LAN, loopback, or other non-public addresses.
					</small>
				</span>
			</label>
		{/if}

		<FormFieldRow label="Role Claim Path" inputId="oidc-role-claim-path">
			<Input id="oidc-role-claim-path" type="text" placeholder="e.g. groups" bind:value={oidcForm.role_claim_path} />
		</FormFieldRow>

		<FormFieldRow label="Role Mapping (JSON)" inputId="oidc-role-mapping-json">
			<Textarea
				id="oidc-role-mapping-json"
				rows={3}
				variant="mono"
				placeholder={'{"oidc_value": "local_role"}'}
				bind:value={oidcForm.role_mapping_json}
			/>
		</FormFieldRow>

		<div class="flex justify-end gap-2 items-center">
			{#if !getIsOnline()}<span class="text-[var(--color-warning)] text-sm mr-auto">Offline</span>{/if}
			<Button variant="secondary" onclick={closeOidcModal}>Cancel</Button>
			<Button variant="primary" loading={saving} disabled={!getIsOnline()} onclick={() => void saveOidcProvider()}
				>{editingProvider ? 'Update' : 'Create'}</Button
			>
		</div>
	</Modal>
{/if}
