<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import {
		getProviderConfigs,
		createProviderConfig,
		updateProviderConfig,
		deleteProviderConfig,
		triggerProviderConfigDiscovery,
		discardProviderConfigDiscovered,
		getAutodiscoveryIgnores,
		createAutodiscoveryIgnore,
		deleteAutodiscoveryIgnore
	} from '$lib/api';
	import { formatDate, parseUrlParam, parseUrlPage } from '$lib/utils';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import ModalBackdrop from '$lib/components/ModalBackdrop.svelte';
	import Pagination from '$lib/components/Pagination.svelte';
	import { Permission } from '$lib/types';
	import type { ProviderConfigResponse, AutodiscoveryIgnoreResponse } from '$lib/types';

	type ActiveTab = 'configs' | 'ignores';
	const ACTIVE_TAB_VALUES = ['configs', 'ignores'] as const satisfies readonly ActiveTab[];

	let activeTab: ActiveTab = $state(parseUrlParam(page.url, 'tab', ACTIVE_TAB_VALUES, 'configs'));

	// Provider configs state
	let configs: ProviderConfigResponse[] = $state([]);
	let configsLoading: boolean = $state(true);
	let showConfigModal: boolean = $state(false);
	let editingConfig: ProviderConfigResponse | null = $state(null);
	let configForm = $state({ name: '', provider_type: 'github_releases', config: '{}', enabled: true });
	let configDeleteConfirm: { id: string; name: string } | null = $state(null);
	let discoveringId: string | null = $state(null);
	let discardingId: string | null = $state(null);

	// Ignore rules state
	let ignores: AutodiscoveryIgnoreResponse[] = $state([]);
	let ignoresLoading: boolean = $state(true);
	let ignoresPage: number = $state(page.url.searchParams.get('tab') === 'ignores' ? parseUrlPage(page.url) : 1);
	let ignoresTotalPages: number = $state(1);
	let showIgnoreModal: boolean = $state(false);
	let ignoreForm = $state({ provider_config_id: '', package_identifier: '' });
	let ignoreDeleteConfirm: { id: string; pkg: string } | null = $state(null);

	const canView = $derived(getUser()?.permissions.includes(Permission.ViewSoftware) ?? false);
	const canManage = $derived(getUser()?.permissions.includes(Permission.ManageSoftware) ?? false);

	$effect(() => {
		const parts: string[] = [];
		if (activeTab !== 'configs') parts.push(`tab=${activeTab}`);
		if (activeTab === 'ignores' && ignoresPage > 1) parts.push(`page=${ignoresPage}`);
		const search = parts.join('&');
		goto(search ? `${location.pathname}?${search}` : location.pathname, {
			replaceState: true,
			keepFocus: true,
			noScroll: true
		});
	});

	onMount(() => {
		if (canView) {
			loadConfigs();
			loadIgnores(ignoresPage);
		}
	});

	function switchTab(tab: ActiveTab) {
		if (activeTab === tab) return;
		ignoresPage = 1;
		activeTab = tab;
	}

	async function loadConfigs() {
		configsLoading = true;
		try {
			const res = await getProviderConfigs();
			configs = res.items;
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load provider configs');
		} finally {
			configsLoading = false;
		}
	}

	async function loadIgnores(page: number) {
		ignoresLoading = true;
		try {
			const res = await getAutodiscoveryIgnores(page);
			ignores = res.items;
			ignoresPage = res.page;
			ignoresTotalPages = res.total_pages;
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load ignore rules');
		} finally {
			ignoresLoading = false;
		}
	}

	function openCreateConfig() {
		editingConfig = null;
		configForm = { name: '', provider_type: 'github_releases', config: '{}', enabled: true };
		showConfigModal = true;
	}

	function openEditConfig(config: ProviderConfigResponse) {
		editingConfig = config;
		configForm = {
			name: config.name,
			provider_type: config.provider_type,
			config: JSON.stringify(config.config, null, 2),
			enabled: config.enabled
		};
		showConfigModal = true;
	}

	function closeConfigModal() {
		showConfigModal = false;
		editingConfig = null;
	}

	async function saveConfig() {
		let parsedConfig: Record<string, unknown>;
		try {
			parsedConfig = JSON.parse(configForm.config || '{}');
		} catch {
			showError('Config must be valid JSON');
			return;
		}

		try {
			if (editingConfig) {
				const updated = await updateProviderConfig(editingConfig.id, {
					name: configForm.name || undefined,
					config: parsedConfig,
					enabled: configForm.enabled
				});
				configs = configs.map((c) => (c.id === editingConfig!.id ? updated : c));
				showSuccess('Provider config updated.');
			} else {
				const created = await createProviderConfig({
					name: configForm.name,
					provider_type: configForm.provider_type,
					config: parsedConfig,
					enabled: configForm.enabled
				});
				configs = [...configs, created];
				showSuccess('Provider config created.');
			}
			closeConfigModal();
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to save provider config');
		}
	}

	async function executeDeleteConfig() {
		if (!configDeleteConfirm) return;
		const { id } = configDeleteConfirm;
		configDeleteConfirm = null;
		try {
			await deleteProviderConfig(id);
			configs = configs.filter((c) => c.id !== id);
			showSuccess('Provider config deleted.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to delete provider config');
		}
	}

	async function triggerDiscover(config: ProviderConfigResponse) {
		discoveringId = config.id;
		try {
			const res = await triggerProviderConfigDiscovery(config.id);
			showSuccess(`Discovery triggered — ${res.providers_queued} provider(s) queued`);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to trigger discovery');
		} finally {
			discoveringId = null;
		}
	}

	async function triggerDiscard(config: ProviderConfigResponse) {
		discardingId = config.id;
		try {
			const res = await discardProviderConfigDiscovered(config.id);
			showSuccess(`Discarded ${res.discarded_count} item(s).`);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to discard discovered items');
		} finally {
			discardingId = null;
		}
	}

	function openCreateIgnore() {
		ignoreForm = { provider_config_id: '', package_identifier: '' };
		showIgnoreModal = true;
	}

	function closeIgnoreModal() {
		showIgnoreModal = false;
	}

	async function saveIgnore() {
		if (!ignoreForm.provider_config_id || !ignoreForm.package_identifier.trim()) {
			showError('Provider config and package identifier are required');
			return;
		}
		try {
			await createAutodiscoveryIgnore({
				provider_config_id: ignoreForm.provider_config_id,
				package_identifier: ignoreForm.package_identifier.trim()
			});
			showSuccess('Ignore rule created.');
			closeIgnoreModal();
			loadIgnores(1);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to create ignore rule');
		}
	}

	async function executeDeleteIgnore() {
		if (!ignoreDeleteConfirm) return;
		const { id } = ignoreDeleteConfirm;
		ignoreDeleteConfirm = null;
		try {
			await deleteAutodiscoveryIgnore(id);
			ignores = ignores.filter((i) => i.id !== id);
			showSuccess('Ignore rule deleted.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to delete ignore rule');
		}
	}
</script>

<svelte:window
	onkeydown={(e) => {
		if (e.key === 'Escape') {
			if (showConfigModal) closeConfigModal();
			else if (showIgnoreModal) closeIgnoreModal();
			else if (configDeleteConfirm) configDeleteConfirm = null;
			else if (ignoreDeleteConfirm) ignoreDeleteConfirm = null;
		}
	}}
/>

{#if getUser()}
	<h1 class="h1 mb-4">Provider Configs</h1>

	{#if !canView}
		<aside class="rounded-lg p-4 preset-filled-error-500">
			<p>You do not have permission to view provider configurations.</p>
		</aside>
	{:else}
		<!-- Tabs -->
		<div class="mb-4 flex gap-1">
			<button
				class="btn btn-sm {activeTab === 'configs' ? 'preset-filled-primary-500' : 'preset-tonal'}"
				onclick={() => switchTab('configs')}
			>
				Configurations
			</button>
			<button
				class="btn btn-sm {activeTab === 'ignores' ? 'preset-filled-primary-500' : 'preset-tonal'}"
				onclick={() => switchTab('ignores')}
			>
				Ignore Rules
			</button>
		</div>

		{#if activeTab === 'configs'}
			<div class="mb-4 flex justify-end">
				{#if canManage}
					<button class="btn preset-filled-primary-500" onclick={openCreateConfig}>Add Config</button>
				{/if}
			</div>

			{#if configsLoading}
				<p class="text-center py-4">Loading...</p>
			{:else}
				<div class="table-wrap">
					<table class="table">
						<thead>
							<tr>
								<th>Name</th>
								<th>Type</th>
								<th>Status</th>
								<th>Created</th>
								{#if canManage}<th class="w-56">Actions</th>{/if}
							</tr>
						</thead>
						<tbody>
							{#each configs as config (config.id)}
								<tr>
									<td>{config.name}</td>
									<td><span class="badge preset-tonal">{config.provider_type}</span></td>
									<td>
										{#if config.enabled}
											<span class="badge preset-filled-success-500">Enabled</span>
										{:else}
											<span class="badge preset-tonal">Disabled</span>
										{/if}
									</td>
									<td>{formatDate(config.created_at)}</td>
									{#if canManage}
										<td>
											<div class="flex gap-1 flex-wrap">
												<button class="btn btn-sm preset-tonal" onclick={() => openEditConfig(config)}>Edit</button>
												<button
													class="btn btn-sm preset-tonal"
													disabled={discoveringId === config.id}
													onclick={() => triggerDiscover(config)}
												>
													{discoveringId === config.id ? '...' : 'Discover'}
												</button>
												<button
													class="btn btn-sm preset-tonal"
													disabled={discardingId === config.id}
													onclick={() => triggerDiscard(config)}
												>
													{discardingId === config.id ? '...' : 'Discard'}
												</button>
												<button
													class="btn btn-sm preset-tonal-error"
													onclick={() => (configDeleteConfirm = { id: config.id, name: config.name })}
												>
													Delete
												</button>
											</div>
										</td>
									{/if}
								</tr>
							{:else}
								<tr>
									<td colspan={canManage ? 5 : 4} class="py-8 text-center">
										<p class="text-lg font-medium">No provider configs</p>
										<p class="mt-1 text-sm text-surface-500">
											Add a provider configuration to enable version tracking.
										</p>
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		{:else}
			<!-- Ignore rules tab -->
			<div class="mb-4 flex justify-end">
				{#if canManage}
					<button class="btn preset-filled-primary-500" onclick={openCreateIgnore}>Add Ignore Rule</button>
				{/if}
			</div>

			{#if ignoresLoading}
				<p class="text-center py-4">Loading...</p>
			{:else}
				<div class="table-wrap">
					<table class="table">
						<thead>
							<tr>
								<th>Provider Config</th>
								<th>Package Identifier</th>
								<th>Created</th>
								{#if canManage}<th class="w-24">Actions</th>{/if}
							</tr>
						</thead>
						<tbody>
							{#each ignores as ignore (ignore.id)}
								<tr>
									<td>
										<span class="font-medium">{ignore.provider_config_name}</span>
										<span class="ml-2 badge preset-tonal text-xs">{ignore.provider_type}</span>
									</td>
									<td><code class="text-sm">{ignore.package_identifier}</code></td>
									<td>{formatDate(ignore.created_at)}</td>
									{#if canManage}
										<td>
											<button
												class="btn btn-sm preset-tonal-error"
												onclick={() => (ignoreDeleteConfirm = { id: ignore.id, pkg: ignore.package_identifier })}
											>
												Delete
											</button>
										</td>
									{/if}
								</tr>
							{:else}
								<tr>
									<td colspan={canManage ? 4 : 3} class="py-8 text-center">
										<p class="text-lg font-medium">No ignore rules</p>
										<p class="mt-1 text-sm text-surface-500">
											Add ignore rules to suppress specific packages from autodiscovery.
										</p>
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
				<Pagination currentPage={ignoresPage} totalPages={ignoresTotalPages} onPageChange={loadIgnores} />
			{/if}
		{/if}
	{/if}
{/if}

<!-- Provider config modal -->
{#if showConfigModal}
	<ModalBackdrop onclose={closeConfigModal}>
		<div
			class="card bg-surface-50 dark:bg-surface-900 w-full max-w-2xl space-y-4 max-h-[90vh] overflow-y-auto p-6 shadow-xl"
			role="dialog"
			aria-modal="true"
		>
			<h3 class="h3">{editingConfig ? 'Edit Provider Config' : 'Add Provider Config'}</h3>

			<label class="label">
				<span>Name</span>
				<input class="input" type="text" bind:value={configForm.name} />
			</label>

			{#if !editingConfig}
				<label class="label">
					<span>Provider Type</span>
					<select class="select" bind:value={configForm.provider_type}>
						<option value="github_releases">GitHub Releases</option>
						<option value="docker_registry">Docker Registry</option>
						<option value="homebrew">Homebrew</option>
						<option value="proxmox_helper_scripts">Proxmox Helper Scripts</option>
					</select>
				</label>
			{/if}

			<label class="label">
				<span>Config (JSON)</span>
				<textarea class="textarea font-mono text-sm" rows="6" bind:value={configForm.config}></textarea>
			</label>

			<label class="flex items-center gap-3">
				<input class="checkbox" type="checkbox" bind:checked={configForm.enabled} />
				<span>Enabled</span>
			</label>

			<div class="flex justify-end gap-2">
				<button class="btn preset-tonal-surface" onclick={closeConfigModal}>Cancel</button>
				<button class="btn preset-filled-primary-500" onclick={saveConfig}>
					{editingConfig ? 'Update' : 'Create'}
				</button>
			</div>
		</div>
	</ModalBackdrop>
{/if}

<!-- Ignore rule modal -->
{#if showIgnoreModal}
	<ModalBackdrop onclose={closeIgnoreModal}>
		<div
			class="card bg-surface-50 dark:bg-surface-900 w-full max-w-md space-y-4 p-6 shadow-xl"
			role="dialog"
			aria-modal="true"
		>
			<h3 class="h3">Add Ignore Rule</h3>

			<label class="label">
				<span>Provider Config</span>
				<select class="select" bind:value={ignoreForm.provider_config_id}>
					<option value="">— select —</option>
					{#each configs as config (config.id)}
						<option value={config.id}>{config.name} ({config.provider_type})</option>
					{/each}
				</select>
			</label>

			<label class="label">
				<span>Package Identifier</span>
				<input
					class="input"
					type="text"
					placeholder="e.g. owner/repo or image:tag"
					bind:value={ignoreForm.package_identifier}
				/>
			</label>

			<div class="flex justify-end gap-2">
				<button class="btn preset-tonal-surface" onclick={closeIgnoreModal}>Cancel</button>
				<button
					class="btn preset-filled-primary-500"
					onclick={saveIgnore}
					disabled={!ignoreForm.provider_config_id || !ignoreForm.package_identifier.trim()}
				>
					Create
				</button>
			</div>
		</div>
	</ModalBackdrop>
{/if}

<!-- Delete confirm dialogs -->
{#if configDeleteConfirm}
	<ConfirmDialog
		title="Delete Provider Config"
		messagePrefix="Are you sure you want to delete"
		entityName={configDeleteConfirm.name}
		confirmLabel="Delete"
		confirmClass="preset-filled-error-500"
		onconfirm={executeDeleteConfig}
		oncancel={() => (configDeleteConfirm = null)}
	/>
{/if}

{#if ignoreDeleteConfirm}
	<ConfirmDialog
		title="Delete Ignore Rule"
		messagePrefix="Are you sure you want to delete the ignore rule for"
		entityName={ignoreDeleteConfirm.pkg}
		confirmLabel="Delete"
		confirmClass="preset-filled-error-500"
		onconfirm={executeDeleteIgnore}
		oncancel={() => (ignoreDeleteConfirm = null)}
	/>
{/if}
