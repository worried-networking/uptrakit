<script lang="ts">
	import { onMount } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import {
		getPluginConfigs,
		createPluginConfig,
		updatePluginConfig,
		deletePluginConfig,
		triggerPluginConfigDiscovery,
		discardPluginConfigDiscovered,
		listDiscoveryAllowlist,
		addDiscoveryAllowlistEntry,
		deleteDiscoveryAllowlistEntry,
		listPluginTypes,
		batchPluginConfigs
	} from '$lib/api';
	import { formatDate } from '$lib/utils';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import Modal from '$lib/components/Modal.svelte';
	import BatchActionBar from '$lib/components/BatchActionBar.svelte';
	import BatchResultDialog from '$lib/components/BatchResultDialog.svelte';
	import { getUser } from '$lib/auth.svelte';
	import { Permission, PluginCapability } from '$lib/types';
	import type {
		PluginConfigResponse,
		TenantDiscoveryAllowlistEntry,
		PluginTypeInfo,
		FieldDef,
		SelectOption,
		BatchActionResponse
	} from '$lib/types';

	const canView = $derived(getUser()?.permissions.includes(Permission.ViewSoftware) ?? false);
	const canManage = $derived(getUser()?.permissions.includes(Permission.ManageSoftware) ?? false);

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
	let showJsonEditor: boolean = $state(false);

	// Discovery allowlist state
	let allowlist: TenantDiscoveryAllowlistEntry[] = $state([]);
	let allowlistLoading: boolean = $state(true);
	let showAllowlistModal: boolean = $state(false);
	let allowlistForm = $state({ plugin_type: '' });
	let allowlistDeleteConfirm: { id: string; plugin_type: string } | null = $state(null);

	// Batch state — plugin configs
	let configSelectedIds = new SvelteSet<string>();
	let configBatchConfirmAction: string | null = $state(null);
	let configBatchResult: BatchActionResponse | null = $state(null);
	let configBatchSubmitting: boolean = $state(false);

	const configBatchActions: { id: string; label: string; destructive?: boolean }[] = [
		{ id: 'delete', label: 'Delete', destructive: true }
	];

	onMount(() => {
		if (canView) {
			loadPluginTypes();
			loadConfigs();
			loadAllowlist();
		}
	});

	async function loadPluginTypes() {
		try {
			pluginTypes = await listPluginTypes();
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load plugin types');
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

	function sampleConfigJson(pluginType: string): string {
		const t = pluginTypes.find((pt) => pt.plugin_type === pluginType);
		return t ? JSON.stringify(t.sample_config, null, 2) : '{}';
	}

	function getFormFields(pluginType: string): FieldDef[] {
		const t = pluginTypes.find((pt) => pt.plugin_type === pluginType);
		return t?.config_form_fields ?? [];
	}

	const currentFormFields = $derived(getFormFields(configForm.plugin_type));
	const hasFormFields = $derived(currentFormFields.length > 0);

	function flattenConfig(config: Record<string, unknown>, fields: FieldDef[]): Record<string, string> {
		const result: Record<string, string> = {};
		for (const field of fields) {
			const parts = field.key.split('.');
			let val: unknown = config;
			for (const part of parts) {
				if (val == null || typeof val !== 'object') {
					val = undefined;
					break;
				}
				const jsonKey = part.startsWith('_') ? part.slice(1) : part;
				val = (val as Record<string, unknown>)[jsonKey];
			}
			if (val === undefined || val === null) {
				result[field.key] = field.default_value ?? '';
			} else if (field.field_type === 'toggle') {
				result[field.key] = val ? 'true' : '';
			} else if (Array.isArray(val)) {
				result[field.key] = val.join('\n');
			} else {
				result[field.key] = String(val);
			}
		}
		return result;
	}

	function unflattenConfig(formValues: Record<string, string>, fields: FieldDef[]): Record<string, unknown> {
		const result: Record<string, unknown> = {};
		for (const field of fields) {
			const raw = formValues[field.key] ?? '';
			if (raw === '' && !field.required && field.field_type !== 'toggle') continue;
			let value: unknown;
			if (field.field_type === 'toggle') {
				value = raw === 'true';
			} else if (field.list) {
				value = raw
					.split('\n')
					.map((s) => s.trim())
					.filter((s) => s.length > 0);
				if ((value as string[]).length === 0) continue;
			} else {
				value = raw;
			}
			const parts = field.key.split('.');
			if (parts.length === 1) {
				result[parts[0]] = value;
			} else {
				let target = result;
				for (let i = 0; i < parts.length - 1; i++) {
					const key = parts[i].startsWith('_') ? parts[i].slice(1) : parts[i];
					if (target[key] == null || typeof target[key] !== 'object') target[key] = {};
					target = target[key] as Record<string, unknown>;
				}
				const lastKey = parts[parts.length - 1];
				const jsonKey = lastKey.startsWith('_') ? lastKey.slice(1) : lastKey;
				target[jsonKey] = value;
			}
		}
		for (const key of Object.keys(result)) {
			const val = result[key];
			if (typeof val === 'object' && val !== null && !Array.isArray(val)) {
				const obj = val as Record<string, unknown>;
				const nonEmpty = Object.values(obj).some((v) => v !== '' && v !== undefined);
				if (!nonEmpty) delete result[key];
			}
		}
		return result;
	}

	let formValues: Record<string, string> = $state({});

	function isFieldVisible(field: FieldDef): boolean {
		if (!field.visible_when) return true;
		const controlValue = formValues[field.visible_when.field] ?? '';
		return field.visible_when.values.includes(controlValue);
	}

	function resolvedOptions(field: FieldDef): SelectOption[] {
		return field.options ?? [];
	}

	function openCreateConfig() {
		editingConfig = null;
		const firstType = pluginTypes[0]?.plugin_type ?? '';
		const fields = getFormFields(firstType);
		configForm = { name: '', plugin_type: firstType, config: sampleConfigJson(firstType), enabled: true };
		formValues = flattenConfig({}, fields);
		showJsonEditor = false;
		showConfigModal = true;
	}

	function openEditConfig(config: PluginConfigResponse) {
		editingConfig = config;
		const fields = getFormFields(config.plugin_type);
		configForm = {
			name: config.name,
			plugin_type: config.plugin_type,
			config: JSON.stringify(config.config, null, 2),
			enabled: config.enabled
		};
		formValues = flattenConfig(config.config, fields);
		showJsonEditor = false;
		showConfigModal = true;
	}

	function closeConfigModal() {
		showConfigModal = false;
		editingConfig = null;
	}

	async function saveConfig() {
		let parsedConfig: Record<string, unknown>;
		if (hasFormFields && !showJsonEditor) {
			parsedConfig = unflattenConfig(formValues, currentFormFields);
		} else {
			try {
				parsedConfig = JSON.parse(configForm.config || '{}');
			} catch {
				showError('Config must be valid JSON');
				return;
			}
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

	function toggleConfigSelectAll() {
		if (configSelectedIds.size === configs.length) {
			configSelectedIds.clear();
		} else {
			configSelectedIds.clear();
			for (const c of configs) configSelectedIds.add(c.id);
		}
	}

	function toggleConfigSelect(id: string) {
		if (configSelectedIds.has(id)) {
			configSelectedIds.delete(id);
		} else {
			configSelectedIds.add(id);
		}
	}

	async function executeConfigBatchAction() {
		if (!configBatchConfirmAction || configBatchSubmitting) return;
		configBatchConfirmAction = null;
		configBatchSubmitting = true;
		try {
			const response = await batchPluginConfigs('delete', [...configSelectedIds]);
			if (response.failed.length > 0) {
				configBatchResult = response;
			} else {
				showSuccess(`${response.succeeded.length} plugin config(s) deleted.`);
			}
			configSelectedIds.clear();
			await loadConfigs();
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to delete plugin configs');
		} finally {
			configBatchSubmitting = false;
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
</script>

<!-- Plugin Configurations -->
<div class="mb-4 flex items-center justify-between">
	<h2 class="h4">Configurations</h2>
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
					{#if canManage}
						<th class="w-10">
							<input
								type="checkbox"
								class="checkbox"
								checked={configs.length > 0 && configSelectedIds.size === configs.length}
								indeterminate={configSelectedIds.size > 0 && configSelectedIds.size < configs.length}
								onchange={toggleConfigSelectAll}
								aria-label="Select all"
							/>
						</th>
					{/if}
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
						{#if canManage}
							<td>
								<input
									type="checkbox"
									class="checkbox"
									checked={configSelectedIds.has(config.id)}
									onchange={() => toggleConfigSelect(config.id)}
									aria-label="Select {config.name}"
								/>
							</td>
						{/if}
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
						<td colspan={canManage ? 7 : 4} class="py-8 text-center">
							<p class="text-lg font-medium">No plugin configs</p>
							<p class="mt-1 text-sm text-surface-500">Add a plugin configuration to enable version tracking.</p>
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}

{#if canManage && configSelectedIds.size > 0}
	<BatchActionBar
		selectedCount={configSelectedIds.size}
		actions={configBatchActions}
		onaction={() => (configBatchConfirmAction = 'delete')}
		oncancel={() => configSelectedIds.clear()}
	/>
{/if}

{#if configBatchConfirmAction}
	<ConfirmDialog
		title="Batch Delete Plugin Configs"
		messagePrefix="Are you sure you want to delete"
		entityName="{configSelectedIds.size} plugin config(s)"
		confirmLabel={configBatchSubmitting ? 'Deleting...' : 'Delete'}
		confirmClass="preset-filled-error-500"
		confirmDisabled={configBatchSubmitting}
		onconfirm={executeConfigBatchAction}
		oncancel={() => (configBatchConfirmAction = null)}
	/>
{/if}

{#if configBatchResult}
	<BatchResultDialog
		title="Batch Action Results"
		response={configBatchResult}
		onclose={() => (configBatchResult = null)}
	/>
{/if}

<!-- Discovery Allowlist -->
<div class="mt-10 mb-4 flex items-center justify-between">
	<h2 class="h4">Discovery Allowlist</h2>
	{#if canManage}
		<button class="btn preset-filled-primary-500" onclick={openAddAllowlistEntry}>Add Plugin Type</button>
	{/if}
</div>
<p class="text-sm text-surface-500 mb-4">
	When the allowlist is empty, all discovery plugins are active. Once you add at least one entry, only the listed plugin
	types will run discovery tenant-wide.
</p>

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

<!-- Plugin config modal -->
{#if showConfigModal}
	<Modal
		title={editingConfig ? 'Edit Plugin Config' : 'Add Plugin Config'}
		onclose={closeConfigModal}
		maxWidth="max-w-2xl max-h-[90vh] overflow-y-auto"
	>
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
					onchange={() => {
						configForm.config = sampleConfigJson(configForm.plugin_type);
						formValues = flattenConfig({}, getFormFields(configForm.plugin_type));
						showJsonEditor = false;
					}}
				>
					{#each pluginTypes as t (t.plugin_type)}
						<option value={t.plugin_type}>{t.display_name}</option>
					{/each}
				</select>
			</label>
		{/if}

		{#if hasFormFields && !showJsonEditor}
			<div class="space-y-3 mt-2">
				{#each currentFormFields as field (field.key)}
					{#if isFieldVisible(field)}
						<div>
							<label for="cfg-{field.key}" class="mb-1 block text-sm font-medium">
								{field.label}
								{#if field.required}<span class="text-error-500">*</span>{/if}
							</label>
							{#if field.field_type === 'textarea'}
								<textarea
									id="cfg-{field.key}"
									bind:value={formValues[field.key]}
									placeholder={field.placeholder}
									required={field.required}
									class="textarea font-mono text-sm w-full"
									rows="3"
								></textarea>
							{:else if field.field_type === 'select'}
								<select
									id="cfg-{field.key}"
									bind:value={formValues[field.key]}
									required={field.required}
									class="select w-full"
								>
									<option value="">— select —</option>
									{#each resolvedOptions(field) as opt (opt.value)}
										<option value={opt.value}>{opt.label}</option>
									{/each}
								</select>
							{:else if field.field_type === 'toggle'}
								<label class="flex items-center gap-2">
									<input
										type="checkbox"
										id="cfg-{field.key}"
										checked={formValues[field.key] === 'true'}
										onchange={(e) => {
											formValues[field.key] = String((e.target as HTMLInputElement).checked);
										}}
										class="checkbox"
									/>
									<span class="text-sm">{field.help_text ?? ''}</span>
								</label>
							{:else}
								<input
									id="cfg-{field.key}"
									type={field.field_type === 'password' ? 'password' : 'text'}
									bind:value={formValues[field.key]}
									placeholder={field.placeholder}
									required={field.required}
									class="input w-full"
								/>
							{/if}
							{#if field.help_text && field.field_type !== 'toggle'}
								<p class="mt-1 text-xs text-surface-500">{field.help_text}</p>
							{/if}
						</div>
					{/if}
				{/each}
			</div>
			<button
				type="button"
				class="btn btn-sm preset-tonal mt-3"
				onclick={() => {
					configForm.config = JSON.stringify(unflattenConfig(formValues, currentFormFields), null, 2);
					showJsonEditor = true;
				}}
			>
				Advanced: Edit as JSON
			</button>
		{:else if hasFormFields && showJsonEditor}
			<label class="label">
				<span>Config (JSON)</span>
				<textarea class="textarea font-mono text-sm" rows="8" bind:value={configForm.config}></textarea>
			</label>
			<button
				type="button"
				class="btn btn-sm preset-tonal mt-1"
				onclick={() => {
					try {
						const parsed = JSON.parse(configForm.config || '{}');
						formValues = flattenConfig(parsed, currentFormFields);
						showJsonEditor = false;
					} catch {
						showError('Config must be valid JSON to switch back to form view');
					}
				}}
			>
				Back to Form
			</button>
		{:else}
			<label class="label">
				<span>Config (JSON)</span>
				<textarea class="textarea font-mono text-sm" rows="6" bind:value={configForm.config}></textarea>
			</label>
		{/if}

		<label class="flex items-center gap-3">
			<input class="checkbox" type="checkbox" bind:checked={configForm.enabled} />
			<span>Enabled</span>
		</label>

		{#snippet footer()}
			<button class="btn preset-tonal-surface" onclick={closeConfigModal}>Cancel</button>
			<button class="btn preset-filled-primary-500" onclick={saveConfig}>
				{editingConfig ? 'Update' : 'Create'}
			</button>
		{/snippet}
	</Modal>
{/if}

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

{#if showAllowlistModal}
	<Modal title="Add Discovery Plugin Type" onclose={closeAllowlistModal}>
		<label class="label">
			<span>Plugin Type</span>
			<select class="select" bind:value={allowlistForm.plugin_type}>
				{#each discoveryPluginTypes as t (t.plugin_type)}
					<option value={t.plugin_type}>{t.display_name}</option>
				{/each}
			</select>
		</label>
		{#snippet footer()}
			<button class="btn preset-tonal-surface" onclick={closeAllowlistModal}>Cancel</button>
			<button class="btn preset-filled-primary-500" onclick={saveAllowlistEntry}>Add</button>
		{/snippet}
	</Modal>
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
