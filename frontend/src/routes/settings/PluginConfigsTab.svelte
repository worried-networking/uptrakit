<script lang="ts">
	import { onMount } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import {
		getPluginConfigs,
		createPluginConfig,
		updatePluginConfig,
		deletePluginConfig,
		triggerPluginConfigDiscovery,
		listDiscoveryAllowlist,
		addDiscoveryAllowlistEntry,
		deleteDiscoveryAllowlistEntry,
		listPluginTypes,
		batchPluginConfigs,
		listPluginTypeSettings,
		upsertPluginTypeSettings,
		deletePluginTypeSettings,
		testPluginConfig
	} from '$lib/api';
	import { formatDate } from '$lib/utils';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import BatchActionBar from '$lib/components/BatchActionBar.svelte';
	import BatchResultDialog from '$lib/components/BatchResultDialog.svelte';
	import { getUser } from '$lib/auth.svelte';
	import { Permission, PluginCapability, hasAnyPermission } from '$lib/types';
	import {
		Callout,
		DataTable,
		ModalShell,
		SectionCard,
		StatusBadge,
		TableFooterBar,
		type DataTableColumn
	} from '$lib/components/ui';
	import { FormFieldRow, Input, Textarea, Checkbox } from '$lib/components/forms';
	import Button from '$lib/components/Button.svelte';
	import type {
		PluginConfigResponse,
		TenantDiscoveryAllowlistEntry,
		PluginTypeInfo,
		PluginTypeSettingsResponse,
		FormField,
		SelectOption,
		BatchActionResponse,
		TestPluginConfigResponse
	} from '$lib/types';

	const canViewConfigs = $derived(getUser()?.permissions.includes(Permission.ViewSoftware) ?? false);
	const canManageConfigs = $derived(getUser()?.permissions.includes(Permission.ManageCommands) ?? false);
	const canTriggerDiscovery = $derived(getUser()?.permissions.includes(Permission.TriggerChecks) ?? false);
	const canManageAllowlist = $derived(getUser()?.permissions.includes(Permission.UpdateSoftware) ?? false);
	const canViewTypeSettings = $derived(
		hasAnyPermission(getUser(), Permission.ViewSettings, Permission.ManageGlobalSettings)
	);
	const canManageTypeSettings = $derived(getUser()?.permissions.includes(Permission.ManageGlobalSettings) ?? false);
	const canTest = $derived(getUser()?.permissions.includes(Permission.TestPluginConfigs) ?? false);

	// Plugin types
	let pluginTypes: PluginTypeInfo[] = $state([]);
	const configurablePluginTypes = $derived(pluginTypes.filter((t) => t.supports_plugin_configs));
	const discoveryPluginTypes = $derived(
		pluginTypes.filter((t) => t.capabilities.includes(PluginCapability.DiscoverLocalSoftware))
	);

	// Plugin configs state
	let configs: PluginConfigResponse[] = $state([]);
	let configsLoading: boolean = $state(true);
	let configsError: string | null = $state(null);
	let configsCurrentPage: number = $state(1);
	let configsTotalPages: number = $state(1);
	let configsTotal: number = $state(0);
	let showConfigModal: boolean = $state(false);
	let editingConfig: PluginConfigResponse | null = $state(null);
	let configForm = $state({ name: '', plugin_type: '', config: '{}', enabled: true });
	let configDeleteConfirm: { id: string; name: string } | null = $state(null);
	let discoveringId: string | null = $state(null);
	let showJsonEditor: boolean = $state(false);
	let configNameError: string = $state('');
	let configPluginTypeError: string = $state('');
	let configJsonError: string = $state('');
	let configFieldErrors: Record<string, string> = $state({});

	// Config test state
	let configTesting: boolean = $state(false);
	let configTestResult: TestPluginConfigResponse | null = $state(null);

	// Discovery allowlist state
	let allowlist: TenantDiscoveryAllowlistEntry[] = $state([]);
	let allowlistLoading: boolean = $state(true);
	let allowlistError: string | null = $state(null);
	let showAllowlistModal: boolean = $state(false);
	let allowlistForm = $state({ plugin_type: '' });
	let allowlistPluginTypeError: string = $state('');
	let allowlistDeleteConfirm: { id: string; plugin_type: string } | null = $state(null);

	// Plugin type settings state
	let typeSettings: PluginTypeSettingsResponse[] = $state([]);
	let typeSettingsLoading: boolean = $state(true);
	let typeSettingsError: string | null = $state(null);
	let editingTypeSettingsType: string | null = $state(null);
	let showTypeSettingsModal: boolean = $state(false);
	let typeSettingsFormValues: Record<string, string> = $state({});
	let typeSettingsFieldErrors: Record<string, string> = $state({});
	let typeSettingsResetConfirm: string | null = $state(null);

	const typeSettingsPluginTypes = $derived(pluginTypes.filter((t) => (t.type_settings_form_fields ?? []).length > 0));

	// Batch state — plugin configs
	let configSelectedIds = new SvelteSet<string>();
	let configBatchConfirmAction: string | null = $state(null);
	let configBatchResult: BatchActionResponse | null = $state(null);
	let configBatchSubmitting: boolean = $state(false);

	const configBatchActions: { id: string; label: string; destructive?: boolean }[] = [
		{ id: 'delete', label: 'Delete', destructive: true }
	];

	onMount(() => {
		if (canViewConfigs || canViewTypeSettings) {
			void loadPluginTypes();
		}
		if (canViewConfigs) {
			loadConfigs();
			loadAllowlist();
		}
		if (canViewTypeSettings) {
			loadTypeSettings();
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
		configsError = null;
		try {
			const res = await getPluginConfigs(configsCurrentPage);
			configs = res.items;
			configsCurrentPage = res.page;
			configsTotalPages = res.total_pages;
			configsTotal = res.total;
			syncConfigSelectionToVisibleRows(res.items);
		} catch (e) {
			configsError = e instanceof Error ? e.message : 'Failed to load plugin configs';
			showError(configsError);
		} finally {
			configsLoading = false;
		}
	}

	async function loadAllowlist() {
		allowlistLoading = true;
		allowlistError = null;
		try {
			allowlist = await listDiscoveryAllowlist();
		} catch (e) {
			allowlistError = e instanceof Error ? e.message : 'Failed to load discovery allowlist';
			showError(allowlistError);
		} finally {
			allowlistLoading = false;
		}
	}

	async function loadTypeSettings() {
		typeSettingsLoading = true;
		typeSettingsError = null;
		try {
			typeSettings = await listPluginTypeSettings();
		} catch (e) {
			typeSettingsError = e instanceof Error ? e.message : 'Failed to load plugin type settings';
			showError(typeSettingsError);
		} finally {
			typeSettingsLoading = false;
		}
	}

	function sampleConfigJson(pluginType: string): string {
		const t = pluginTypes.find((pt) => pt.plugin_type === pluginType);
		return t ? JSON.stringify(t.sample_config, null, 2) : '{}';
	}

	function getFormFields(pluginType: string): FormField[] {
		const t = pluginTypes.find((pt) => pt.plugin_type === pluginType);
		return t?.config_form_fields ?? [];
	}

	const currentFormFields = $derived(getFormFields(configForm.plugin_type));
	const hasFormFields = $derived(currentFormFields.length > 0);

	function flattenConfig(config: Record<string, unknown>, fields: FormField[]): Record<string, string> {
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

	function unflattenConfig(formValues: Record<string, string>, fields: FormField[]): Record<string, unknown> {
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

	function isFieldVisible(field: FormField, values: Record<string, string> = formValues): boolean {
		if (!field.visible_when) return true;
		const controlValue = values[field.visible_when.field] ?? '';
		return field.visible_when.values.includes(controlValue);
	}

	function resolvedOptions(field: FormField): SelectOption[] {
		return field.options ?? [];
	}

	function clearConfigFieldError(fieldKey: string) {
		if (!(fieldKey in configFieldErrors)) return;
		const next = { ...configFieldErrors };
		delete next[fieldKey];
		configFieldErrors = next;
	}

	function clearTypeSettingsFieldError(fieldKey: string) {
		if (!(fieldKey in typeSettingsFieldErrors)) return;
		const next = { ...typeSettingsFieldErrors };
		delete next[fieldKey];
		typeSettingsFieldErrors = next;
	}

	function clearConfigValidation() {
		configNameError = '';
		configPluginTypeError = '';
		configJsonError = '';
		configFieldErrors = {};
	}

	function requiredFieldErrors(fields: FormField[], values: Record<string, string>): Record<string, string> {
		const errors: Record<string, string> = {};
		for (const field of fields) {
			if (!field.required || !isFieldVisible(field, values)) continue;
			if (field.field_type === 'toggle') continue;
			const raw = values[field.key] ?? '';
			const hasValue = field.list
				? raw
						.split('\n')
						.map((s) => s.trim())
						.some((s) => s.length > 0)
				: raw.trim().length > 0;
			if (!hasValue) {
				errors[field.key] = `${field.label} is required.`;
			}
		}
		return errors;
	}

	function openCreateConfig() {
		editingConfig = null;
		const firstType = configurablePluginTypes[0]?.plugin_type ?? '';
		const fields = getFormFields(firstType);
		configForm = { name: '', plugin_type: firstType, config: sampleConfigJson(firstType), enabled: true };
		formValues = flattenConfig({}, fields);
		showJsonEditor = false;
		configTestResult = null;
		clearConfigValidation();
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
		configTestResult = null;
		clearConfigValidation();
		showConfigModal = true;
	}

	function closeConfigModal() {
		showConfigModal = false;
		editingConfig = null;
		clearConfigValidation();
	}

	function handleConfigPageChange(page: number) {
		if (page === configsCurrentPage) return;
		configSelectedIds.clear();
		configBatchConfirmAction = null;
		configsCurrentPage = page;
		void loadConfigs();
	}

	async function saveConfig() {
		clearConfigValidation();
		let hasBlockingErrors = false;
		if (!configForm.name.trim()) {
			configNameError = 'Name is required.';
			hasBlockingErrors = true;
		}
		if (!editingConfig && !configForm.plugin_type.trim()) {
			configPluginTypeError = 'Plugin type is required.';
			hasBlockingErrors = true;
		}

		let parsedConfig: Record<string, unknown> = {};
		if (hasFormFields && !showJsonEditor) {
			const fieldErrors = requiredFieldErrors(currentFormFields, formValues);
			if (Object.keys(fieldErrors).length > 0) {
				configFieldErrors = fieldErrors;
				hasBlockingErrors = true;
			}
		} else {
			try {
				parsedConfig = JSON.parse(configForm.config || '{}');
			} catch {
				configJsonError = 'Config must be valid JSON';
				hasBlockingErrors = true;
			}
			if (hasFormFields && !hasBlockingErrors) {
				const flattened = flattenConfig(parsedConfig, currentFormFields);
				const fieldErrors = requiredFieldErrors(currentFormFields, flattened);
				if (Object.keys(fieldErrors).length > 0) {
					configFieldErrors = fieldErrors;
					if (showJsonEditor) {
						const [firstError] = Object.values(fieldErrors);
						configJsonError = firstError ?? 'Required fields are missing.';
					}
					hasBlockingErrors = true;
				}
			}
		}

		if (hasBlockingErrors) {
			return;
		}

		if (hasFormFields && !showJsonEditor) {
			parsedConfig = unflattenConfig(formValues, currentFormFields);
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
			await loadConfigs();
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
			showSuccess('Plugin config deleted.');
			await loadConfigs();
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

	async function testCurrentConfig() {
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
		configTesting = true;
		configTestResult = null;
		try {
			configTestResult = await testPluginConfig({
				plugin_type: configForm.plugin_type,
				config: parsedConfig,
				plugin_config_id: editingConfig?.id
			});
			if (configTestResult.success) {
				showSuccess('Config test passed.');
			} else {
				showError(configTestResult.error ?? 'Config test failed.');
			}
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to test config');
		} finally {
			configTesting = false;
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

	function syncConfigSelectionToVisibleRows(visibleConfigs: PluginConfigResponse[]) {
		if (configSelectedIds.size === 0) return;
		const visibleIds = new Set(visibleConfigs.map((config) => config.id));
		for (const selectedId of [...configSelectedIds]) {
			if (!visibleIds.has(selectedId)) {
				configSelectedIds.delete(selectedId);
			}
		}
		if (configSelectedIds.size === 0 && configBatchConfirmAction) {
			configBatchConfirmAction = null;
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

	function getTypeSettingsFields(pluginType: string): FormField[] {
		const t = pluginTypes.find((pt) => pt.plugin_type === pluginType);
		return t?.type_settings_form_fields ?? [];
	}

	function getTypeSettingsConfig(pluginType: string): Record<string, unknown> | null {
		return typeSettings.find((s) => s.plugin_type === pluginType)?.config ?? null;
	}

	function openEditTypeSettings(pluginType: string) {
		editingTypeSettingsType = pluginType;
		const fields = getTypeSettingsFields(pluginType);
		const existing = getTypeSettingsConfig(pluginType);
		const t = pluginTypes.find((pt) => pt.plugin_type === pluginType);
		const sample = t?.type_settings_sample ?? {};
		typeSettingsFormValues = flattenConfig(existing ?? sample, fields);
		typeSettingsFieldErrors = {};
		showTypeSettingsModal = true;
	}

	function closeTypeSettingsModal() {
		showTypeSettingsModal = false;
		editingTypeSettingsType = null;
		typeSettingsFieldErrors = {};
	}

	async function saveTypeSettings() {
		if (!editingTypeSettingsType) return;
		const fields = getTypeSettingsFields(editingTypeSettingsType);
		typeSettingsFieldErrors = requiredFieldErrors(fields, typeSettingsFormValues);
		if (Object.keys(typeSettingsFieldErrors).length > 0) {
			return;
		}
		const config = unflattenConfig(typeSettingsFormValues, fields);
		try {
			const updated = await upsertPluginTypeSettings(editingTypeSettingsType, config);
			const idx = typeSettings.findIndex((s) => s.plugin_type === editingTypeSettingsType);
			if (idx >= 0) {
				typeSettings = typeSettings.map((s) => (s.plugin_type === editingTypeSettingsType ? updated : s));
			} else {
				typeSettings = [...typeSettings, updated];
			}
			showSuccess('Type settings saved.');
			closeTypeSettingsModal();
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to save type settings');
		}
	}

	async function executeResetTypeSettings() {
		if (!typeSettingsResetConfirm) return;
		const pluginType = typeSettingsResetConfirm;
		typeSettingsResetConfirm = null;
		try {
			await deletePluginTypeSettings(pluginType);
			typeSettings = typeSettings.filter((s) => s.plugin_type !== pluginType);
			showSuccess('Type settings reset to defaults.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to reset type settings');
		}
	}

	function openAddAllowlistEntry() {
		allowlistForm = { plugin_type: discoveryPluginTypes[0]?.plugin_type ?? '' };
		allowlistPluginTypeError = '';
		showAllowlistModal = true;
	}

	function closeAllowlistModal() {
		showAllowlistModal = false;
		allowlistPluginTypeError = '';
	}

	async function saveAllowlistEntry() {
		if (!allowlistForm.plugin_type.trim()) {
			allowlistPluginTypeError = 'Plugin type is required.';
			return;
		}
		allowlistPluginTypeError = '';
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

	const configTableColumns: DataTableColumn[] = [
		{ key: 'select', label: 'Select' },
		{ key: 'name', label: 'Name' },
		{ key: 'type', label: 'Type' },
		{ key: 'status', label: 'Status' },
		{ key: 'created', label: 'Created' },
		{ key: 'actions', label: 'Actions' }
	];

	const allowlistColumns: DataTableColumn[] = [
		{ key: 'plugin_type', label: 'Plugin Type' },
		{ key: 'created_at', label: 'Created' },
		{ key: 'actions', label: 'Actions' }
	];

	const typeSettingsColumns: DataTableColumn[] = [
		{ key: 'plugin_type', label: 'Plugin Type' },
		{ key: 'settings', label: 'Current Settings' },
		{ key: 'actions', label: 'Actions' }
	];

	const loadingSkeletonRows = [0, 1, 2, 3, 4];
</script>

{#if canViewConfigs}
	<!-- Plugin Configurations -->
	<SectionCard title="Configurations">
		<div class="mb-4 flex items-center justify-end">
			{#if canManageConfigs}
				<Button variant="primary" onclick={openCreateConfig}>Add Config</Button>
			{/if}
		</div>

		{#if configsLoading}
			<div
				class="overflow-hidden rounded-panel border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm"
				data-ui="known-shape-table-loading"
				aria-busy="true"
			>
				<div class="overflow-x-auto">
					<table class="min-w-full border-collapse text-table-body">
						<caption class="sr-only">Loading plugin configurations</caption>
						<thead>
							<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
								{#if canManageConfigs}
									<th
										class="w-10 table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
										>Select</th
									>
								{/if}
								<th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
									>Name</th
								>
								<th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
									>Type</th
								>
								<th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
									>Status</th
								>
								<th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
									>Created</th
								>
								{#if canManageConfigs || canTriggerDiscovery}
									<th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
										>Actions</th
									>
								{/if}
							</tr>
						</thead>
						<tbody>
							{#each loadingSkeletonRows as rowIndex (rowIndex)}
								<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
									{#if canManageConfigs}
										<td class="table-cell-pad">
											<div
												data-ui="loading-skeleton-cell"
												class="h-3 w-4 animate-pulse rounded-card bg-[var(--bg-raised)]"
											></div>
										</td>
									{/if}
									<td class="table-cell-pad"
										><div
											data-ui="loading-skeleton-cell"
											class="h-3 w-28 animate-pulse rounded-card bg-[var(--bg-raised)]"
										></div></td
									>
									<td class="table-cell-pad"
										><div
											data-ui="loading-skeleton-cell"
											class="h-3 w-24 animate-pulse rounded-card bg-[var(--bg-raised)]"
										></div></td
									>
									<td class="table-cell-pad"
										><div
											data-ui="loading-skeleton-cell"
											class="h-3 w-20 animate-pulse rounded-card bg-[var(--bg-raised)]"
										></div></td
									>
									<td class="table-cell-pad"
										><div
											data-ui="loading-skeleton-cell"
											class="h-3 w-28 animate-pulse rounded-card bg-[var(--bg-raised)]"
										></div></td
									>
									{#if canManageConfigs || canTriggerDiscovery}
										<td class="table-cell-pad">
											<div
												data-ui="loading-skeleton-cell"
												class="h-3 w-20 animate-pulse rounded-card bg-[var(--bg-raised)]"
											></div>
										</td>
									{/if}
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			</div>
		{:else}
			<DataTable
				columns={configTableColumns}
				rows={configs as unknown as Record<string, unknown>[]}
				loading={false}
				error={configsError}
				emptyTitle="No plugin configs"
				emptyDescription="Add a plugin configuration to enable version tracking."
				rowKey={(row) => (row as unknown as PluginConfigResponse).id}
			>
				{#snippet header()}
					<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
						{#if canManageConfigs}
							<th class="w-10 table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header">
								<Checkbox
									id="config-select-all"
									checked={configs.length > 0 && configSelectedIds.size === configs.length}
									indeterminate={configSelectedIds.size > 0 && configSelectedIds.size < configs.length}
									onchange={toggleConfigSelectAll}
									aria-label="Select all"
								/>
							</th>
						{/if}
						<th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							>Name</th
						>
						<th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							>Type</th
						>
						<th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							>Status</th
						>
						<th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							>Created</th
						>
						{#if canManageConfigs || canTriggerDiscovery}
							<th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								>Actions</th
							>
						{/if}
					</tr>
				{/snippet}
				{#snippet row(rowValue, _index)}
					{@const config = rowValue as unknown as PluginConfigResponse}
					<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
						{#if canManageConfigs}
							<td class="table-cell-pad">
								<Checkbox
									id="config-select-{config.id}"
									checked={configSelectedIds.has(config.id)}
									onchange={() => toggleConfigSelect(config.id)}
									aria-label="Select {config.name}"
								/>
							</td>
						{/if}
						<td class="table-cell-pad">{config.name}</td>
						<td class="table-cell-pad"><StatusBadge tone="neutral" label={config.plugin_type} /></td>
						<td class="table-cell-pad">
							{#if config.enabled}
								<StatusBadge tone="success" label="Enabled" />
							{:else}
								<StatusBadge tone="neutral" label="Disabled" />
							{/if}
						</td>
						<td class="table-cell-pad">{formatDate(config.created_at)}</td>
						{#if canManageConfigs || canTriggerDiscovery}
							<td class="table-cell-pad">
								<div class="flex flex-wrap gap-1">
									{#if canManageConfigs}
										<Button variant="secondary" size="sm" onclick={() => openEditConfig(config)}>Edit</Button>
									{/if}
									{#if canTriggerDiscovery && config.capabilities.includes(PluginCapability.DiscoverLocalSoftware)}
										<Button
											variant="secondary"
											size="sm"
											loading={discoveringId === config.id}
											onclick={() => triggerDiscover(config)}>Discover</Button
										>
									{/if}
									{#if canManageConfigs}
										<Button
											variant="danger"
											size="sm"
											onclick={() => (configDeleteConfirm = { id: config.id, name: config.name })}>Delete</Button
										>
									{/if}
								</div>
							</td>
						{/if}
					</tr>
				{/snippet}
				{#snippet footer()}
					{#if configsTotalPages > 1}
						<TableFooterBar
							total={configsTotal}
							currentPage={configsCurrentPage}
							totalPages={configsTotalPages}
							onPageChange={handleConfigPageChange}
						/>
					{/if}
				{/snippet}
			</DataTable>
		{/if}
	</SectionCard>
{/if}

{#if canManageConfigs && configSelectedIds.size > 0}
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

{#if canViewConfigs}
	<!-- Discovery Allowlist -->
	<SectionCard
		title="Discovery Allowlist"
		description="When the allowlist is empty, all discovery plugins are active. Once you add at least one entry, only listed plugin types run tenant-wide."
	>
		<div class="mb-4 flex items-center justify-end">
			{#if canManageAllowlist}
				<Button variant="primary" onclick={openAddAllowlistEntry}>Add Plugin Type</Button>
			{/if}
		</div>
		{#if allowlistLoading}
			<div
				class="overflow-hidden rounded-panel border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm"
				data-ui="known-shape-table-loading"
				aria-busy="true"
			>
				<div class="overflow-x-auto">
					<table class="min-w-full border-collapse text-table-body">
						<caption class="sr-only">Loading discovery allowlist</caption>
						<thead>
							<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
								<th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
									>Plugin Type</th
								>
								<th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
									>Created</th
								>
								{#if canManageAllowlist}
									<th
										class="w-24 table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
										>Actions</th
									>
								{/if}
							</tr>
						</thead>
						<tbody>
							{#each loadingSkeletonRows as rowIndex (rowIndex)}
								<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
									<td class="table-cell-pad"
										><div
											data-ui="loading-skeleton-cell"
											class="h-3 w-28 animate-pulse rounded-card bg-[var(--bg-raised)]"
										></div></td
									>
									<td class="table-cell-pad"
										><div
											data-ui="loading-skeleton-cell"
											class="h-3 w-28 animate-pulse rounded-card bg-[var(--bg-raised)]"
										></div></td
									>
									{#if canManageAllowlist}
										<td class="table-cell-pad">
											<div
												data-ui="loading-skeleton-cell"
												class="h-3 w-16 animate-pulse rounded-card bg-[var(--bg-raised)]"
											></div>
										</td>
									{/if}
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			</div>
		{:else}
			<DataTable
				columns={allowlistColumns}
				rows={allowlist as unknown as Record<string, unknown>[]}
				loading={false}
				error={allowlistError}
				emptyTitle="No restrictions — all discovery plugins are active."
				emptyDescription="Add a plugin type to restrict discovery to only the listed types."
				rowKey={(row) => (row as unknown as TenantDiscoveryAllowlistEntry).id}
			>
				{#snippet header()}
					<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
						<th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							>Plugin Type</th
						>
						<th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							>Created</th
						>
						{#if canManageAllowlist}
							<th class="w-24 table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								>Actions</th
							>
						{/if}
					</tr>
				{/snippet}
				{#snippet row(rowValue, _index)}
					{@const entry = rowValue as unknown as TenantDiscoveryAllowlistEntry}
					<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
						<td class="table-cell-pad"><StatusBadge tone="neutral" label={entry.plugin_type} /></td>
						<td class="table-cell-pad">{formatDate(entry.created_at)}</td>
						{#if canManageAllowlist}
							<td class="table-cell-pad">
								<Button
									variant="danger"
									size="sm"
									onclick={() => (allowlistDeleteConfirm = { id: entry.id, plugin_type: entry.plugin_type })}
									>Remove</Button
								>
							</td>
						{/if}
					</tr>
				{/snippet}
			</DataTable>
		{/if}
	</SectionCard>
{/if}

<!-- Type Defaults -->
{#if canViewTypeSettings}
	<SectionCard
		title="Type Defaults"
		description="Tenant-wide default settings for plugin types. Defaults apply unless overridden by a plugin config or host assignment."
	>
		{#if typeSettingsLoading}
			<div
				class="overflow-hidden rounded-panel border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm"
				data-ui="known-shape-table-loading"
				aria-busy="true"
			>
				<div class="overflow-x-auto">
					<table class="min-w-full border-collapse text-table-body">
						<caption class="sr-only">Loading plugin type defaults</caption>
						<thead>
							<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
								<th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
									>Plugin Type</th
								>
								<th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
									>Current Settings</th
								>
								{#if canManageTypeSettings}
									<th
										class="w-36 table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
										>Actions</th
									>
								{/if}
							</tr>
						</thead>
						<tbody>
							{#each loadingSkeletonRows as rowIndex (rowIndex)}
								<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
									<td class="table-cell-pad"
										><div
											data-ui="loading-skeleton-cell"
											class="h-3 w-24 animate-pulse rounded-card bg-[var(--bg-raised)]"
										></div></td
									>
									<td class="table-cell-pad"
										><div
											data-ui="loading-skeleton-cell"
											class="h-3 w-40 animate-pulse rounded-card bg-[var(--bg-raised)]"
										></div></td
									>
									{#if canManageTypeSettings}
										<td class="table-cell-pad">
											<div
												data-ui="loading-skeleton-cell"
												class="h-3 w-20 animate-pulse rounded-card bg-[var(--bg-raised)]"
											></div>
										</td>
									{/if}
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			</div>
		{:else}
			<DataTable
				columns={typeSettingsColumns}
				rows={typeSettingsPluginTypes as unknown as Record<string, unknown>[]}
				loading={false}
				error={typeSettingsError}
				emptyTitle="No type defaults available."
				emptyDescription="No plugin types expose tenant-wide defaults."
				rowKey={(row) => (row as unknown as PluginTypeInfo).plugin_type}
			>
				{#snippet header()}
					<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
						<th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							>Plugin Type</th
						>
						<th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							>Current Settings</th
						>
						{#if canManageTypeSettings}
							<th class="w-36 table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								>Actions</th
							>
						{/if}
					</tr>
				{/snippet}
				{#snippet row(rowValue, _index)}
					{@const t = rowValue as unknown as PluginTypeInfo}
					{@const existing = getTypeSettingsConfig(t.plugin_type)}
					<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
						<td class="table-cell-pad"><StatusBadge tone="neutral" label={t.plugin_type} /></td>
						<td class="table-cell-pad">
							{#if existing}
								<code class="text-xs">{JSON.stringify(existing)}</code>
							{:else}
								<span class="text-sm text-[var(--text-muted)]">Default</span>
							{/if}
						</td>
						{#if canManageTypeSettings}
							<td class="table-cell-pad">
								<div class="flex flex-wrap gap-1">
									<Button variant="secondary" size="sm" onclick={() => openEditTypeSettings(t.plugin_type)}>Edit</Button
									>
									{#if existing}
										<Button variant="danger" size="sm" onclick={() => (typeSettingsResetConfirm = t.plugin_type)}
											>Reset</Button
										>
									{/if}
								</div>
							</td>
						{/if}
					</tr>
				{/snippet}
			</DataTable>
		{/if}
	</SectionCard>
{/if}

<!-- Plugin config modal -->
{#if showConfigModal}
	<ModalShell
		title={editingConfig ? 'Edit Plugin Config' : 'Add Plugin Config'}
		onclose={closeConfigModal}
		maxWidth="max-w-2xl max-h-[90vh] overflow-y-auto"
	>
		<div class="space-y-4">
			<FormFieldRow label="Name" inputId="plugin-config-name" required error={configNameError || undefined}>
				<Input
					id="plugin-config-name"
					type="text"
					bind:value={configForm.name}
					error={configNameError || undefined}
					oninput={() => {
						configNameError = '';
					}}
				/>
			</FormFieldRow>

			{#if !editingConfig}
				<FormFieldRow
					label="Plugin Type"
					inputId="plugin-config-plugin-type"
					required
					error={configPluginTypeError || undefined}
				>
					<select
						id="plugin-config-plugin-type"
						class="select"
						bind:value={configForm.plugin_type}
						aria-invalid={configPluginTypeError ? 'true' : undefined}
						onchange={() => {
							configPluginTypeError = '';
							configForm.config = sampleConfigJson(configForm.plugin_type);
							formValues = flattenConfig({}, getFormFields(configForm.plugin_type));
							configFieldErrors = {};
							showJsonEditor = false;
						}}
					>
						{#each configurablePluginTypes as t (t.plugin_type)}
							<option value={t.plugin_type}>{t.display_name}</option>
						{/each}
					</select>
				</FormFieldRow>
			{/if}

			{#if hasFormFields && !showJsonEditor}
				<div class="mt-2 space-y-4">
					{#each currentFormFields as field (field.key)}
						{#if isFieldVisible(field)}
							<FormFieldRow
								label={field.label}
								inputId={'cfg-' + field.key}
								required={field.required}
								hint={field.field_type === 'toggle' ? undefined : field.help_text}
								error={configFieldErrors[field.key] || undefined}
							>
								{#if field.field_type === 'textarea'}
									<Textarea
										id="cfg-{field.key}"
										bind:value={formValues[field.key]}
										placeholder={field.placeholder}
										required={field.required}
										oninput={() => clearConfigFieldError(field.key)}
										variant="mono"
										rows={3}
									/>
								{:else if field.field_type === 'select'}
									<select
										id="cfg-{field.key}"
										bind:value={formValues[field.key]}
										required={field.required}
										aria-invalid={configFieldErrors[field.key] ? 'true' : undefined}
										onchange={() => clearConfigFieldError(field.key)}
										class="select w-full"
									>
										<option value="">— select —</option>
										{#each resolvedOptions(field) as opt (opt.value)}
											<option value={opt.value}>{opt.label}</option>
										{/each}
									</select>
								{:else if field.field_type === 'toggle'}
									<label class="flex items-center gap-2">
										<Checkbox
											id="cfg-{field.key}"
											checked={formValues[field.key] === 'true'}
											onchange={(e) => {
												formValues[field.key] = String((e.target as HTMLInputElement).checked);
												clearConfigFieldError(field.key);
											}}
										/>
										<span class="text-sm">{field.help_text ?? ''}</span>
									</label>
								{:else}
									<Input
										id="cfg-{field.key}"
										type={field.field_type === 'password' ? 'password' : 'text'}
										bind:value={formValues[field.key]}
										placeholder={field.placeholder}
										required={field.required}
										error={configFieldErrors[field.key] || undefined}
										oninput={() => clearConfigFieldError(field.key)}
									/>
								{/if}
							</FormFieldRow>
						{/if}
					{/each}
					<FormFieldRow label="Editor Mode" hint="Switch to raw JSON editing for advanced cases.">
						<Button
							variant="secondary"
							size="sm"
							onclick={() => {
								configForm.config = JSON.stringify(unflattenConfig(formValues, currentFormFields), null, 2);
								configJsonError = '';
								showJsonEditor = true;
							}}>Advanced: Edit as JSON</Button
						>
					</FormFieldRow>
				</div>
			{:else if hasFormFields && showJsonEditor}
				<FormFieldRow
					label="Config (JSON)"
					inputId="plugin-config-json-editor"
					error={configJsonError || undefined}
					hint="Provide a JSON object matching this plugin type."
				>
					<Textarea
						id="plugin-config-json-editor"
						variant="mono"
						rows={8}
						bind:value={configForm.config}
						oninput={() => {
							configJsonError = '';
						}}
					/>
				</FormFieldRow>
				<FormFieldRow label="Editor Mode" hint="Return to schema-driven field editing.">
					<Button
						variant="secondary"
						size="sm"
						onclick={() => {
							try {
								const parsed = JSON.parse(configForm.config || '{}');
								formValues = flattenConfig(parsed, currentFormFields);
								configFieldErrors = {};
								configJsonError = '';
								showJsonEditor = false;
							} catch {
								configJsonError = 'Config must be valid JSON to switch back to form view';
							}
						}}>Back to Form</Button
					>
				</FormFieldRow>
			{:else}
				<FormFieldRow
					label="Config (JSON)"
					inputId="plugin-config-json"
					error={configJsonError || undefined}
					hint="Provide a JSON object for this plugin configuration."
				>
					<Textarea
						id="plugin-config-json"
						variant="mono"
						rows={6}
						bind:value={configForm.config}
						oninput={() => {
							configJsonError = '';
						}}
					/>
				</FormFieldRow>
			{/if}

			<FormFieldRow label="Enabled" inputId="plugin-config-enabled">
				<label class="flex items-center gap-3">
					<Checkbox id="plugin-config-enabled" bind:checked={configForm.enabled} />
					<span>Enable this configuration</span>
				</label>
			</FormFieldRow>

			{#if configTestResult}
				<Callout
					tone={configTestResult.success ? 'success' : 'danger'}
					title={configTestResult.success ? 'Test Passed' : 'Test Failed'}
				>
					<div class="flex items-center gap-2 mb-1">
						<span class="text-xs text-[var(--text-muted)]"
							>{configTestResult.test_kind} &mdash; {configTestResult.duration_ms}ms</span
						>
					</div>
					{#if configTestResult.detected_version}
						<p class="text-sm">Detected version: <code>{configTestResult.detected_version}</code></p>
					{/if}
					{#if configTestResult.output}
						<pre
							class="mt-1 text-xs font-mono whitespace-pre-wrap max-h-32 overflow-y-auto">{configTestResult.output}</pre>
					{/if}
					{#if configTestResult.error}
						<p class="mt-1 text-sm text-[var(--color-danger)]">{configTestResult.error}</p>
					{/if}
				</Callout>
			{/if}
		</div>

		{#snippet footer()}
			<Button variant="secondary" onclick={closeConfigModal}>Cancel</Button>
			{#if canTest}
				<Button
					variant="secondary"
					loading={configTesting}
					disabled={!configForm.plugin_type}
					onclick={testCurrentConfig}>Test</Button
				>
			{/if}
			<Button variant="primary" onclick={saveConfig}>
				{editingConfig ? 'Update' : 'Create'}
			</Button>
		{/snippet}
	</ModalShell>
{/if}

{#if configDeleteConfirm}
	<ConfirmDialog
		title="Delete Plugin Config"
		messagePrefix="Are you sure you want to delete"
		entityName={configDeleteConfirm.name}
		confirmLabel="Delete"
		onconfirm={executeDeleteConfig}
		oncancel={() => (configDeleteConfirm = null)}
	/>
{/if}

{#if showAllowlistModal}
	<ModalShell title="Add Discovery Plugin Type" onclose={closeAllowlistModal}>
		<FormFieldRow
			label="Plugin Type"
			inputId="allowlist-plugin-type"
			required
			error={allowlistPluginTypeError || undefined}
		>
			<select
				id="allowlist-plugin-type"
				class="select"
				bind:value={allowlistForm.plugin_type}
				aria-invalid={allowlistPluginTypeError ? 'true' : undefined}
				onchange={() => {
					allowlistPluginTypeError = '';
				}}
			>
				<option value="">— select —</option>
				{#each discoveryPluginTypes as t (t.plugin_type)}
					<option value={t.plugin_type}>{t.display_name}</option>
				{/each}
			</select>
		</FormFieldRow>
		{#snippet footer()}
			<Button variant="secondary" onclick={closeAllowlistModal}>Cancel</Button>
			<Button variant="primary" onclick={saveAllowlistEntry}>Add</Button>
		{/snippet}
	</ModalShell>
{/if}

{#if allowlistDeleteConfirm}
	<ConfirmDialog
		title="Remove Allowlist Entry"
		messagePrefix="Remove discovery plugin type"
		entityName={allowlistDeleteConfirm.plugin_type}
		confirmLabel="Remove"
		onconfirm={executeDeleteAllowlistEntry}
		oncancel={() => (allowlistDeleteConfirm = null)}
	/>
{/if}

<!-- Type settings modal -->
{#if showTypeSettingsModal && editingTypeSettingsType}
	{@const tsFields = getTypeSettingsFields(editingTypeSettingsType)}
	{@const tsType = pluginTypes.find((t) => t.plugin_type === editingTypeSettingsType)}
	<ModalShell
		title="Edit Type Defaults — {tsType?.display_name ?? editingTypeSettingsType}"
		onclose={closeTypeSettingsModal}
		maxWidth="max-w-2xl max-h-[90vh] overflow-y-auto"
	>
		<div class="space-y-4">
			{#each tsFields as field (field.key)}
				{#if isFieldVisible(field, typeSettingsFormValues)}
					<FormFieldRow
						label={field.label}
						inputId={'ts-' + field.key}
						required={field.required}
						hint={field.field_type === 'toggle' ? undefined : field.help_text}
						error={typeSettingsFieldErrors[field.key] || undefined}
					>
						{#if field.field_type === 'textarea'}
							<Textarea
								id="ts-{field.key}"
								bind:value={typeSettingsFormValues[field.key]}
								placeholder={field.placeholder}
								required={field.required}
								oninput={() => clearTypeSettingsFieldError(field.key)}
								variant="mono"
								rows={3}
							/>
						{:else if field.field_type === 'select'}
							<select
								id="ts-{field.key}"
								bind:value={typeSettingsFormValues[field.key]}
								required={field.required}
								aria-invalid={typeSettingsFieldErrors[field.key] ? 'true' : undefined}
								onchange={() => clearTypeSettingsFieldError(field.key)}
								class="select w-full"
							>
								<option value="">— select —</option>
								{#each resolvedOptions(field) as opt (opt.value)}
									<option value={opt.value}>{opt.label}</option>
								{/each}
							</select>
						{:else if field.field_type === 'toggle'}
							<label class="flex items-center gap-2">
								<Checkbox
									id="ts-{field.key}"
									checked={typeSettingsFormValues[field.key] === 'true'}
									onchange={(e) => {
										typeSettingsFormValues[field.key] = String((e.target as HTMLInputElement).checked);
										clearTypeSettingsFieldError(field.key);
									}}
								/>
								<span class="text-sm">{field.help_text ?? ''}</span>
							</label>
						{:else}
							<Input
								id="ts-{field.key}"
								type={field.field_type === 'password' ? 'password' : 'text'}
								bind:value={typeSettingsFormValues[field.key]}
								placeholder={field.placeholder}
								required={field.required}
								error={typeSettingsFieldErrors[field.key] || undefined}
								oninput={() => clearTypeSettingsFieldError(field.key)}
							/>
						{/if}
					</FormFieldRow>
				{/if}
			{/each}
		</div>
		{#snippet footer()}
			<Button variant="secondary" onclick={closeTypeSettingsModal}>Cancel</Button>
			<Button variant="primary" onclick={saveTypeSettings}>Save</Button>
		{/snippet}
	</ModalShell>
{/if}

{#if typeSettingsResetConfirm}
	{@const tsType = pluginTypes.find((t) => t.plugin_type === typeSettingsResetConfirm)}
	<ConfirmDialog
		title="Reset Type Defaults"
		messagePrefix="Reset type settings for"
		entityName={tsType?.display_name ?? typeSettingsResetConfirm}
		confirmLabel="Reset"
		onconfirm={executeResetTypeSettings}
		oncancel={() => (typeSettingsResetConfirm = null)}
	/>
{/if}
