<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import {
		getPluginConfigs,
		createPluginConfig,
		updatePluginConfig,
		deletePluginConfig,
		triggerPluginConfigDiscovery,
		discardPluginConfigDiscovered,
		getAutodiscoveryIgnores,
		createAutodiscoveryIgnore,
		deleteAutodiscoveryIgnore,
		listDiscoveryAllowlist,
		addDiscoveryAllowlistEntry,
		deleteDiscoveryAllowlistEntry,
		listPluginTypes
	} from '$lib/api';
	import { formatDate, parseUrlParam, parseUrlPage } from '$lib/utils';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import ModalBackdrop from '$lib/components/ModalBackdrop.svelte';
	import Pagination from '$lib/components/Pagination.svelte';
	import { Permission, PluginCapability } from '$lib/types';
	import type {
		PluginConfigResponse,
		AutodiscoveryIgnoreResponse,
		TenantDiscoveryAllowlistEntry,
		PluginTypeInfo
	} from '$lib/types';

	type ActiveTab = 'configs' | 'ignores' | 'allowlist';
	const ACTIVE_TAB_VALUES = ['configs', 'ignores', 'allowlist'] as const satisfies readonly ActiveTab[];

	let activeTab: ActiveTab = $state(parseUrlParam(page.url, 'tab', ACTIVE_TAB_VALUES, 'configs'));

	// Plugin types
	let pluginTypes: PluginTypeInfo[] = $state([]);
	const discoveryPluginTypes = $derived(
		pluginTypes.filter((t) => t.capabilities.includes(PluginCapability.DiscoverLocalSoftware))
	);

	// Plugin configs state
	let configs: PluginConfigResponse[] = $state([]);
	let configsLoading: boolean = $state(true);
	let showConfigModal: boolean = $state(false);
	let editingConfig: PluginConfigResponse | null = $state(null);
	let configForm = $state({ name: '', plugin_type: '', config: '{}', enabled: true });
	let configDeleteConfirm: { id: string; name: string } | null = $state(null);
	let discoveringId: string | null = $state(null);
	let discardingId: string | null = $state(null);

	// Ignore rules state
	let ignores: AutodiscoveryIgnoreResponse[] = $state([]);
	let ignoresLoading: boolean = $state(true);
	let ignoresPage: number = $state(page.url.searchParams.get('tab') === 'ignores' ? parseUrlPage(page.url) : 1);
	let ignoresTotalPages: number = $state(1);
	let showIgnoreModal: boolean = $state(false);
	let ignoreForm = $state({ plugin_config_id: '', package_identifier: '' });
	let ignoreDeleteConfirm: { id: string; pkg: string } | null = $state(null);

	// Discovery allowlist state
	let allowlist: TenantDiscoveryAllowlistEntry[] = $state([]);
	let allowlistLoading: boolean = $state(true);
	let showAllowlistModal: boolean = $state(false);
	let allowlistForm = $state({ plugin_type: '' });
	let allowlistDeleteConfirm: { id: string; plugin_type: string } | null = $state(null);

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
			loadPluginTypes();
			loadConfigs();
			loadIgnores(ignoresPage);
			loadAllowlist();
		}
	});

	function switchTab(tab: ActiveTab) {
		if (activeTab === tab) return;
		ignoresPage = 1;
		activeTab = tab;
	}

	async function loadPluginTypes() {
		try {
			pluginTypes = await listPluginTypes();
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load plugin types');
		}
	}

	async function loadAllowlist() {
		allowlistLoading = true;
		try {
			allowlist = await listDiscoveryAllowlist();
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load discovery allowlist');
		} finally {
			allowlistLoading = false;
		}
	}

	function openAddAllowlistEntry() {
		allowlistForm = { plugin_type: discoveryPluginTypes[0]?.plugin_type ?? '' };
		showAllowlistModal = true;
	}

	function closeAllowlistModal() {
		showAllowlistModal = false;
	}

	async function saveAllowlistEntry() {
		try {
			const created = await addDiscoveryAllowlistEntry({ plugin_type: allowlistForm.plugin_type });
			// If it already existed the server returns the existing entry — avoid duplicates.
			if (!allowlist.some((e) => e.id === created.id)) {
				allowlist = [...allowlist, created];
			}
			showSuccess('Allowlist entry added.');
			closeAllowlistModal();
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to add allowlist entry');
		}
	}

	async function executeDeleteAllowlistEntry() {
		if (!allowlistDeleteConfirm) return;
		const { id } = allowlistDeleteConfirm;
		allowlistDeleteConfirm = null;
		try {
			await deleteDiscoveryAllowlistEntry(id);
			allowlist = allowlist.filter((e) => e.id !== id);
			showSuccess('Allowlist entry removed.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to remove allowlist entry');
		}
	}

	async function loadConfigs() {
		configsLoading = true;
		try {
			const res = await getPluginConfigs();
			configs = res.items;
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load plugin configs');
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

	function sampleConfigJson(pluginType: string): string {
		const t = pluginTypes.find((pt) => pt.plugin_type === pluginType);
		return t ? JSON.stringify(t.sample_config, null, 2) : '{}';
	}

	function openCreateConfig() {
		editingConfig = null;
		const firstType = pluginTypes[0]?.plugin_type ?? '';
		configForm = { name: '', plugin_type: firstType, config: sampleConfigJson(firstType), enabled: true };
		showConfigModal = true;
	}

	function openEditConfig(config: PluginConfigResponse) {
		editingConfig = config;
		configForm = {
			name: config.name,
			plugin_type: config.plugin_type,
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
				const updated = await updatePluginConfig(editingConfig.id, {
					name: configForm.name || undefined,
					config: parsedConfig,
					enabled: configForm.enabled
				});
				configs = configs.map((c) => (c.id === editingConfig!.id ? updated : c));
				showSuccess('Plugin config updated.');
			} else {
				const created = await createPluginConfig({
					name: configForm.name,
					plugin_type: configForm.plugin_type,
					config: parsedConfig,
					enabled: configForm.enabled
				});
				configs = [...configs, created];
				showSuccess('Plugin config created.');
			}
			closeConfigModal();
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to save plugin config');
		}
	}

	async function executeDeleteConfig() {
		if (!configDeleteConfirm) return;
		const { id } = configDeleteConfirm;
		configDeleteConfirm = null;
		try {
			await deletePluginConfig(id);
			configs = configs.filter((c) => c.id !== id);
			showSuccess('Plugin config deleted.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to delete plugin config');
		}
	}

	async function triggerDiscover(config: PluginConfigResponse) {
		discoveringId = config.id;
		try {
			const res = await triggerPluginConfigDiscovery(config.id);
			showSuccess(`Discovery triggered — ${res.plugins_queued} plugin(s) queued`);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to trigger discovery');
		} finally {
			discoveringId = null;
		}
	}

	async function triggerDiscard(config: PluginConfigResponse) {
		discardingId = config.id;
		try {
			const res = await discardPluginConfigDiscovered(config.id);
			showSuccess(`Discarded ${res.discarded_count} item(s).`);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to discard discovered items');
		} finally {
			discardingId = null;
		}
	}

	function openCreateIgnore() {
		ignoreForm = { plugin_config_id: '', package_identifier: '' };
		showIgnoreModal = true;
	}

	function closeIgnoreModal() {
		showIgnoreModal = false;
	}

	async function saveIgnore() {
		if (!ignoreForm.plugin_config_id || !ignoreForm.package_identifier.trim()) {
			showError('Plugin config and package identifier are required');
			return;
		}
		try {
			await createAutodiscoveryIgnore({
				plugin_config_id: ignoreForm.plugin_config_id,
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
			else if (showAllowlistModal) closeAllowlistModal();
			else if (configDeleteConfirm) configDeleteConfirm = null;
			else if (ignoreDeleteConfirm) ignoreDeleteConfirm = null;
			else if (allowlistDeleteConfirm) allowlistDeleteConfirm = null;
		}
	}}
/>

{#if getUser()}
	<h1 class="h1 mb-4">Plugin Configs</h1>

	{#if !canView}
		<aside class="rounded-lg p-4 preset-filled-error-500">
			<p>You do not have permission to view plugin configurations.</p>
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
			<button
				class="btn btn-sm {activeTab === 'allowlist' ? 'preset-filled-primary-500' : 'preset-tonal'}"
				onclick={() => switchTab('allowlist')}
			>
				Discovery Allowlist
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
								{#if canManage}<th>Actions</th>{/if}
							</tr>
						</thead>
						<tbody>
							{#each configs as config (config.id)}
								<tr>
									<td>{config.name}</td>
									<td><span class="badge preset-tonal">{config.plugin_type}</span></td>
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
												{#if config.capabilities.includes(PluginCapability.DiscoverLocalSoftware)}
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
												{/if}
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
										<p class="text-lg font-medium">No plugin configs</p>
										<p class="mt-1 text-sm text-surface-500">Add a plugin configuration to enable version tracking.</p>
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		{:else if activeTab === 'ignores'}
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
								<th>Plugin Config</th>
								<th>Package Identifier</th>
								<th>Created</th>
								{#if canManage}<th class="w-24">Actions</th>{/if}
							</tr>
						</thead>
						<tbody>
							{#each ignores as ignore (ignore.id)}
								<tr>
									<td>
										<span class="font-medium">{ignore.plugin_config_name}</span>
										<span class="ml-2 badge preset-tonal text-xs">{ignore.plugin_type}</span>
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
		{:else}
			<!-- Discovery allowlist tab -->
			<div class="mb-4">
				<p class="text-sm text-surface-500 mb-3">
					When the allowlist is empty, all discovery plugins are active. Once you add at least one entry, only the
					listed plugin types will run discovery tenant-wide.
				</p>
				{#if canManage}
					<div class="flex justify-end">
						<button class="btn preset-filled-primary-500" onclick={openAddAllowlistEntry}> Add Plugin Type </button>
					</div>
				{/if}
			</div>

			{#if allowlistLoading}
				<p class="text-center py-4">Loading...</p>
			{:else if allowlist.length === 0}
				<aside class="rounded-lg p-4 preset-tonal-surface">
					<p class="font-medium">No restrictions — all discovery plugins are active.</p>
					<p class="mt-1 text-sm text-surface-500">Add a plugin type to restrict discovery to only the listed types.</p>
				</aside>
			{:else}
				<div class="table-wrap">
					<table class="table">
						<thead>
							<tr>
								<th>Plugin Type</th>
								<th>Created</th>
								{#if canManage}<th class="w-24">Actions</th>{/if}
							</tr>
						</thead>
						<tbody>
							{#each allowlist as entry (entry.id)}
								<tr>
									<td><span class="badge preset-tonal">{entry.plugin_type}</span></td>
									<td>{formatDate(entry.created_at)}</td>
									{#if canManage}
										<td>
											<button
												class="btn btn-sm preset-tonal-error"
												onclick={() => (allowlistDeleteConfirm = { id: entry.id, plugin_type: entry.plugin_type })}
											>
												Remove
											</button>
										</td>
									{/if}
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		{/if}
	{/if}
{/if}

<!-- Plugin config modal -->
{#if showConfigModal}
	<ModalBackdrop onclose={closeConfigModal}>
		<div
			class="card bg-surface-50 dark:bg-surface-900 w-full max-w-2xl space-y-4 max-h-[90vh] overflow-y-auto p-6 shadow-xl"
			role="dialog"
			aria-modal="true"
		>
			<h3 class="h3">{editingConfig ? 'Edit Plugin Config' : 'Add Plugin Config'}</h3>

			<label class="label">
				<span>Name</span>
				<input class="input" type="text" bind:value={configForm.name} />
			</label>

			{#if !editingConfig}
				<label class="label">
					<span>Plugin Type</span>
					<select
						class="select"
						bind:value={configForm.plugin_type}
						onchange={() => (configForm.config = sampleConfigJson(configForm.plugin_type))}
					>
						{#each pluginTypes as t (t.plugin_type)}
							<option value={t.plugin_type}>{t.display_name}</option>
						{/each}
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
				<span>Plugin Config</span>
				<select class="select" bind:value={ignoreForm.plugin_config_id}>
					<option value="">— select —</option>
					{#each configs as config (config.id)}
						<option value={config.id}>{config.name} ({config.plugin_type})</option>
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
					disabled={!ignoreForm.plugin_config_id || !ignoreForm.package_identifier.trim()}
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
		title="Delete Plugin Config"
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

<!-- Discovery allowlist modal -->
{#if showAllowlistModal}
	<ModalBackdrop onclose={closeAllowlistModal}>
		<div
			class="card bg-surface-50 dark:bg-surface-900 w-full max-w-md space-y-4 p-6 shadow-xl"
			role="dialog"
			aria-modal="true"
		>
			<h3 class="h3">Add Discovery Plugin Type</h3>

			<label class="label">
				<span>Plugin Type</span>
				<select class="select" bind:value={allowlistForm.plugin_type}>
					{#each discoveryPluginTypes as t (t.plugin_type)}
						<option value={t.plugin_type}>{t.display_name}</option>
					{/each}
				</select>
			</label>

			<div class="flex justify-end gap-2">
				<button class="btn preset-tonal-surface" onclick={closeAllowlistModal}>Cancel</button>
				<button class="btn preset-filled-primary-500" onclick={saveAllowlistEntry}>Add</button>
			</div>
		</div>
	</ModalBackdrop>
{/if}

{#if allowlistDeleteConfirm}
	<ConfirmDialog
		title="Remove Allowlist Entry"
		messagePrefix="Remove discovery plugin type"
		entityName={allowlistDeleteConfirm.plugin_type}
		confirmLabel="Remove"
		confirmClass="preset-filled-error-500"
		onconfirm={executeDeleteAllowlistEntry}
		oncancel={() => (allowlistDeleteConfirm = null)}
	/>
{/if}
