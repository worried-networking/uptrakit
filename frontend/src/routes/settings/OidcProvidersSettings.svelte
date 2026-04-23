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
	import { FormFieldRow, SectionCard, StatusBadge } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';
	import Checkbox from '$lib/components/Checkbox.svelte';

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
</script>

<SectionCard title="OIDC Providers">
	<div class="mb-4 flex items-center justify-between">
		<Button variant="primary" onclick={openCreateOidc}>Add Provider</Button>
	</div>

	{#if oidcProviders.length === 0}
		<p class="py-4 text-center text-[var(--text-secondary)]">No OIDC providers configured.</p>
	{:else}
		<div class="table-wrap">
			<table class="table">
				<thead>
					<tr>
						<th>Name</th>
						<th>Slug</th>
						<th>Status</th>
						<th class="w-48">Actions</th>
					</tr>
				</thead>
				<tbody>
					{#each oidcProviders as provider (provider.id)}
						<tr>
							<td>{provider.name}</td>
							<td>{provider.slug}</td>
							<td>
								{#if provider.is_active}
									<StatusBadge tone="success" label="Active" />
								{:else}
									<StatusBadge tone="neutral" label="Inactive" />
								{/if}
							</td>
							<td>
								<div class="flex gap-1">
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
					{/each}
				</tbody>
			</table>
		</div>
	{/if}
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
				<input id="oidc-name" class="input" type="text" bind:value={oidcForm.name} oninput={onOidcNameInput} />
			</FormFieldRow>
			<FormFieldRow label="Slug" inputId="oidc-slug">
				<input
					id="oidc-slug"
					class="input"
					type="text"
					bind:value={oidcForm.slug}
					oninput={() => {
						slugTouched = true;
					}}
				/>
			</FormFieldRow>
		</div>

		<label class="label">
			<span>Logo URL</span>
			<input class="input" type="url" placeholder="https://..." bind:value={oidcForm.logo_url} />
			{#if oidcForm.logo_url && !isValidLogoUrl(oidcForm.logo_url)}
				<small class="text-[var(--color-error)]">Logo URL must use HTTPS</small>
			{/if}
		</label>

		<label class="label">
			<span>Issuer URL</span>
			<input class="input" type="text" bind:value={oidcForm.issuer_url} />
		</label>

		<div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
			<label class="label">
				<span>Client ID</span>
				<input class="input" type="text" bind:value={oidcForm.client_id} />
			</label>
			<label class="label">
				<span>Client Secret</span>
				<input
					class="input"
					type="password"
					placeholder={editingProvider ? 'Leave blank to keep current' : ''}
					bind:value={oidcForm.client_secret}
				/>
			</label>
		</div>

		<label class="label">
			<span>Scopes</span>
			<input class="input" type="text" bind:value={oidcForm.scopes} />
		</label>

		<label class="flex items-center gap-3">
			<Checkbox id="oidc-auto-create-users" bind:checked={oidcForm.auto_create_users} />
			<span>Auto-create users on first login</span>
		</label>

		{#if multiTenancyEnabled}
			<aside class="rounded-[3px] border border-[var(--border-default)] p-3 text-sm text-[var(--text-primary)]">
				Private-network OIDC issuers are disabled in multi-tenant mode and cannot be changed.
			</aside>
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

		<label class="label">
			<span>Role Claim Path</span>
			<input class="input" type="text" placeholder="e.g. groups" bind:value={oidcForm.role_claim_path} />
		</label>

		<label class="label">
			<span>Role Mapping (JSON)</span>
			<textarea
				class="textarea"
				rows="3"
				placeholder={'{"oidc_value": "local_role"}'}
				bind:value={oidcForm.role_mapping_json}
			></textarea>
		</label>

		<div class="flex justify-end gap-2 items-center">
			{#if !getIsOnline()}<span class="text-warning-500 text-sm mr-auto">Offline</span>{/if}
			<Button variant="secondary" onclick={closeOidcModal}>Cancel</Button>
			<Button variant="primary" loading={saving} disabled={!getIsOnline()} onclick={() => void saveOidcProvider()}
				>{editingProvider ? 'Update' : 'Create'}</Button
			>
		</div>
	</Modal>
{/if}
