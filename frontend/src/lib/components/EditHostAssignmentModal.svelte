<script lang="ts">
	import { onMount } from 'svelte';
	import Modal from './Modal.svelte';
	import { getPluginConfigs, updateHostAssignment, deletePluginAssignment, listPluginTypes } from '$lib/api';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import type {
		FormField,
		HostPluginRoleSummary,
		PluginConfigResponse,
		PluginTypeInfo,
		SelectOption,
		SoftwareItemDetailResponse,
		UpdateHostAssignmentRequest
	} from '$lib/types';
	import { PluginCapability } from '$lib/types';

	// ---------------------------------------------------------------------------
	// Types
	// ---------------------------------------------------------------------------

	type StandardRoleKey = 'detect_version' | 'fetch_releases' | 'execute_update';
	type HookRoleKey = 'pre_update_hook' | 'post_update_hook';
	type RoleKey = StandardRoleKey | HookRoleKey;

	const STANDARD_ROLES: StandardRoleKey[] = ['detect_version', 'fetch_releases', 'execute_update'];
	const HOOK_ROLES: HookRoleKey[] = ['pre_update_hook', 'post_update_hook'];

	type RenderItem = { kind: 'standard'; role: StandardRoleKey } | { kind: 'hook'; role: HookRoleKey };
	const RENDER_ORDER: RenderItem[] = [
		{ kind: 'standard', role: 'detect_version' },
		{ kind: 'standard', role: 'fetch_releases' },
		{ kind: 'hook', role: 'pre_update_hook' },
		{ kind: 'standard', role: 'execute_update' },
		{ kind: 'hook', role: 'post_update_hook' }
	];

	const ROLE_LABELS: Record<RoleKey, string> = {
		detect_version: 'Detect Version',
		fetch_releases: 'Fetch Releases',
		execute_update: 'Execute Update',
		pre_update_hook: 'Pre-Update Hook',
		post_update_hook: 'Post-Update Hook'
	};

	const ROLE_DESCRIPTIONS: Record<RoleKey, string> = {
		detect_version: 'Checks what version is currently installed on the host.',
		fetch_releases: 'Queries upstream for the latest available version.',
		execute_update: 'Performs the actual software update on the host.',
		pre_update_hook: 'Runs before the update (e.g. stop a service).',
		post_update_hook: 'Runs after the update (e.g. restart a service).'
	};

	interface StandardRoleState {
		plugin_type: string;
		plugin_config_id: string;
		package_identifier: string;
		execution_site: string;
		config_override_text: string;
		config_override_error: string | null;
		overrideFormValues: Record<string, string>;
		overrideShowJson: boolean;
	}

	/** One hook entry in the list. */
	interface HookEntry {
		/** Local stable key for Svelte reactivity. */
		localKey: string;
		/**
		 * Ordinal that was loaded from the server (null = new entry added in
		 * this editing session).
		 */
		origOrdinal: number | null;
		plugin_type: string;
		plugin_config_id: string;
		config_override_text: string;
		config_override_error: string | null;
		overrideFormValues: Record<string, string>;
		overrideShowJson: boolean;
	}

	// ---------------------------------------------------------------------------
	// Props
	// ---------------------------------------------------------------------------

	let {
		softwareItemId,
		hostId,
		hostName,
		softwareItemName,
		existingPlugins,
		onclose,
		onsuccess
	}: {
		softwareItemId: string;
		hostId: string;
		hostName: string;
		softwareItemName: string;
		existingPlugins: HostPluginRoleSummary[];
		onclose: () => void;
		onsuccess: (result: SoftwareItemDetailResponse) => void;
	} = $props();

	// ---------------------------------------------------------------------------
	// Reactive state
	// ---------------------------------------------------------------------------

	let pluginConfigs: PluginConfigResponse[] = $state([]);
	let pluginTypes: PluginTypeInfo[] = $state([]);
	let loading: boolean = $state(true);
	let loadError: string | null = $state(null);
	let submitting: boolean = $state(false);

	/** State for the three standard roles. */
	let standardStates: Record<StandardRoleKey, StandardRoleState> = $state(makeInitialStandardStates());

	/** Lists of hook entries per hook role. */
	let hookLists: Record<HookRoleKey, HookEntry[]> = $state({ pre_update_hook: [], post_update_hook: [] });

	/** Ordinals present in the original server response (for computing deletes). */
	let origOrdinalsByRole: Record<HookRoleKey, number[]> = { pre_update_hook: [], post_update_hook: [] };

	let localKeyCounter = 0;
	function nextKey() {
		return `hook-${++localKeyCounter}`;
	}

	// ---------------------------------------------------------------------------
	// Initializers
	// ---------------------------------------------------------------------------

	function makeInitialStandardStates(): Record<StandardRoleKey, StandardRoleState> {
		const result = {} as Record<StandardRoleKey, StandardRoleState>;
		for (const role of STANDARD_ROLES) {
			const existing = existingPlugins.find((p) => p.role === role);
			result[role] = existing
				? {
						plugin_type: existing.plugin_type,
						plugin_config_id: existing.plugin_config_id ?? '',
						package_identifier: existing.package_identifier,
						execution_site: existing.execution_site || 'auto',
						config_override_text: existing.config_override ? JSON.stringify(existing.config_override, null, 2) : '',
						config_override_error: null,
						overrideFormValues: {},
						overrideShowJson: false
					}
				: {
						plugin_type: '',
						plugin_config_id: '',
						package_identifier: '',
						execution_site: 'auto',
						config_override_text: '',
						config_override_error: null,
						overrideFormValues: {},
						overrideShowJson: false
					};
		}
		return result;
	}

	function makeInitialHookLists(): Record<HookRoleKey, HookEntry[]> {
		const result: Record<HookRoleKey, HookEntry[]> = { pre_update_hook: [], post_update_hook: [] };
		for (const role of HOOK_ROLES) {
			const hooks = existingPlugins.filter((p) => p.role === role).sort((a, b) => a.ordinal - b.ordinal);
			origOrdinalsByRole[role] = hooks.map((h) => h.ordinal);
			result[role] = hooks.map((h) => ({
				localKey: nextKey(),
				origOrdinal: h.ordinal,
				plugin_type: h.plugin_type,
				plugin_config_id: h.plugin_config_id ?? '',
				config_override_text: h.config_override ? JSON.stringify(h.config_override, null, 2) : '',
				config_override_error: null,
				overrideFormValues: {},
				overrideShowJson: false
			}));
		}
		return result;
	}

	// ---------------------------------------------------------------------------
	// Lifecycle
	// ---------------------------------------------------------------------------

	onMount(async () => {
		try {
			const [configsResult, typesResult] = await Promise.all([getPluginConfigs(1, 500), listPluginTypes()]);
			pluginConfigs = configsResult.items;
			pluginTypes = typesResult;

			// Initialise hook lists from existing plugins.
			hookLists = makeInitialHookLists();

			// Initialise override form values for standard roles.
			for (const role of STANDARD_ROLES) {
				const existing = existingPlugins.find((p) => p.role === role);
				if (existing?.plugin_config_id) {
					const config = pluginConfigs.find((c) => c.id === existing.plugin_config_id);
					if (config) {
						const fields = getFormFields(config.plugin_type);
						standardStates[role].overrideFormValues = flattenConfig(
							(existing.config_override as Record<string, unknown>) ?? {},
							fields
						);
					}
				} else if (existing?.plugin_type) {
					const pt = pluginTypes.find((t) => t.plugin_type === existing.plugin_type);
					if (pt) {
						const fields = pt.config_form_fields ?? [];
						standardStates[role].overrideFormValues = flattenConfig(
							(existing.config_override as Record<string, unknown>) ?? pt.sample_config,
							fields
						);
					}
				}
			}

			// Initialise override form values for hook entries.
			for (const role of HOOK_ROLES) {
				const hooks = existingPlugins.filter((p) => p.role === role).sort((a, b) => a.ordinal - b.ordinal);
				for (let i = 0; i < hookLists[role].length; i++) {
					const existing = hooks[i];
					if (existing?.plugin_config_id) {
						const config = pluginConfigs.find((c) => c.id === existing.plugin_config_id);
						if (config) {
							const fields = getFormFields(config.plugin_type);
							hookLists[role][i].overrideFormValues = flattenConfig(
								(existing.config_override as Record<string, unknown>) ?? {},
								fields
							);
						}
					} else if (existing?.plugin_type) {
						const pt = pluginTypes.find((t) => t.plugin_type === existing.plugin_type);
						if (pt) {
							const fields = pt.config_form_fields ?? [];
							hookLists[role][i].overrideFormValues = flattenConfig(
								(existing.config_override as Record<string, unknown>) ?? pt.sample_config,
								fields
							);
						}
					}
				}
			}
		} catch (e) {
			loadError = e instanceof Error ? e.message : 'Failed to load plugin configs.';
		} finally {
			loading = false;
		}
	});

	// ---------------------------------------------------------------------------
	// Plugin-type helpers
	// ---------------------------------------------------------------------------

	function pluginTypesForRole(role: StandardRoleKey): PluginTypeInfo[] {
		const capMap: Record<StandardRoleKey, string> = {
			detect_version: 'version_detection',
			fetch_releases: 'release_fetching',
			execute_update: 'update_execution'
		};
		const cap = capMap[role];
		return pluginTypes.filter((t) => t.capabilities.includes(cap as PluginCapability));
	}

	function hookPluginTypes(): PluginTypeInfo[] {
		return pluginTypes.filter((t) => t.capabilities.includes(PluginCapability.UpdateLifecycle));
	}

	function savedConfigsForType(pluginType: string): PluginConfigResponse[] {
		return pluginConfigs.filter((c) => c.plugin_type === pluginType);
	}

	function pluginSelection(s: { plugin_type: string; plugin_config_id: string }): string {
		if (s.plugin_config_id) return `cfg:${s.plugin_config_id}`;
		if (s.plugin_type) return `type:${s.plugin_type}`;
		return '';
	}

	function applySelection(
		target: {
			plugin_type: string;
			plugin_config_id: string;
			overrideFormValues: Record<string, string>;
			overrideShowJson: boolean;
			config_override_text: string;
			config_override_error: string | null;
		},
		value: string
	): void {
		if (value.startsWith('cfg:')) {
			const id = value.slice(4);
			target.plugin_config_id = id;
			const config = pluginConfigs.find((c) => c.id === id);
			if (config) {
				target.plugin_type = config.plugin_type;
				const fields = getFormFields(config.plugin_type);
				target.overrideFormValues = flattenConfig({}, fields);
			}
		} else if (value.startsWith('type:')) {
			const ptStr = value.slice(5);
			target.plugin_type = ptStr;
			target.plugin_config_id = '';
			const pt = pluginTypes.find((t) => t.plugin_type === ptStr);
			const fields = pt?.config_form_fields ?? [];
			target.overrideFormValues = flattenConfig(pt?.sample_config ?? {}, fields);
		} else {
			target.plugin_type = '';
			target.plugin_config_id = '';
			target.overrideFormValues = {};
		}
		target.overrideShowJson = false;
		target.config_override_text = '';
		target.config_override_error = null;
	}

	function getFormFields(pluginType: string): FormField[] {
		const t = pluginTypes.find((pt) => pt.plugin_type === pluginType);
		return t?.config_form_fields ?? [];
	}

	function resolvedOptions(field: FormField): SelectOption[] {
		return field.options ?? [];
	}

	// ---------------------------------------------------------------------------
	// Config override helpers (shared between standard and hook entries)
	// ---------------------------------------------------------------------------

	/** Flatten a nested config object into dot-path form values. */
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

	/** Unflatten dot-path form values into a nested config object. */
	function unflattenConfig(formValues: Record<string, string>, fields: FormField[]): Record<string, unknown> {
		const result: Record<string, unknown> = {};
		for (const field of fields) {
			const raw = formValues[field.key] ?? '';
			if (raw === '' && field.field_type !== 'toggle') continue;
			let value: unknown;
			if (field.field_type === 'toggle') {
				value = raw === 'true';
				if (value === false) continue;
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
		// Clean up empty nested objects.
		for (const key of Object.keys(result)) {
			const val = result[key];
			if (typeof val === 'object' && val !== null && !Array.isArray(val)) {
				if (!Object.values(val as Record<string, unknown>).some((v) => v !== '' && v !== undefined)) delete result[key];
			}
		}
		return result;
	}

	// ---------------------------------------------------------------------------
	// Standard-role override helpers
	// ---------------------------------------------------------------------------

	function getStdFormFields(role: StandardRoleKey): FormField[] {
		const s = standardStates[role];
		if (!s.plugin_type) return [];
		if (s.plugin_config_id) {
			const config = pluginConfigs.find((c) => c.id === s.plugin_config_id);
			return config ? getFormFields(config.plugin_type) : [];
		}
		return getFormFields(s.plugin_type);
	}

	function isStdOverrideFieldVisible(field: FormField, role: StandardRoleKey): boolean {
		if (!field.visible_when) return true;
		const controlValue = standardStates[role].overrideFormValues[field.visible_when.field] ?? '';
		return field.visible_when.values.includes(controlValue);
	}

	function hasStdOverride(role: StandardRoleKey): boolean {
		const fields = getStdFormFields(role);
		if (fields.length > 0 && !standardStates[role].overrideShowJson) {
			return Object.keys(unflattenConfig(standardStates[role].overrideFormValues, fields)).length > 0;
		}
		return !!standardStates[role].config_override_text.trim();
	}

	function validateStdOverride(role: StandardRoleKey): boolean {
		const fields = getStdFormFields(role);
		if (fields.length > 0 && !standardStates[role].overrideShowJson) {
			standardStates[role].config_override_error = null;
			return true;
		}
		const text = standardStates[role].config_override_text.trim();
		if (!text) {
			standardStates[role].config_override_error = null;
			return true;
		}
		try {
			const parsed: unknown = JSON.parse(text);
			if (typeof parsed !== 'object' || Array.isArray(parsed) || parsed === null) {
				standardStates[role].config_override_error = 'Must be a JSON object, not an array or primitive.';
				return false;
			}
			standardStates[role].config_override_error = null;
			return true;
		} catch {
			standardStates[role].config_override_error = 'Invalid JSON.';
			return false;
		}
	}

	// ---------------------------------------------------------------------------
	// Hook-entry override helpers
	// ---------------------------------------------------------------------------

	function getHookFormFields(entry: HookEntry): FormField[] {
		if (!entry.plugin_type) return [];
		return getFormFields(entry.plugin_type);
	}

	function isHookOverrideFieldVisible(field: FormField, entry: HookEntry): boolean {
		if (!field.visible_when) return true;
		const controlValue = entry.overrideFormValues[field.visible_when.field] ?? '';
		return field.visible_when.values.includes(controlValue);
	}

	function hasHookOverride(entry: HookEntry): boolean {
		const fields = getHookFormFields(entry);
		if (fields.length > 0 && !entry.overrideShowJson) {
			return Object.keys(unflattenConfig(entry.overrideFormValues, fields)).length > 0;
		}
		return !!entry.config_override_text.trim();
	}

	function validateHookOverride(entry: HookEntry): boolean {
		const fields = getHookFormFields(entry);
		if (fields.length > 0 && !entry.overrideShowJson) {
			entry.config_override_error = null;
			return true;
		}
		const text = entry.config_override_text.trim();
		if (!text) {
			entry.config_override_error = null;
			return true;
		}
		try {
			const parsed: unknown = JSON.parse(text);
			if (typeof parsed !== 'object' || Array.isArray(parsed) || parsed === null) {
				entry.config_override_error = 'Must be a JSON object, not an array or primitive.';
				return false;
			}
			entry.config_override_error = null;
			return true;
		} catch {
			entry.config_override_error = 'Invalid JSON.';
			return false;
		}
	}

	// ---------------------------------------------------------------------------
	// Hook list mutations
	// ---------------------------------------------------------------------------

	function addHook(role: HookRoleKey) {
		hookLists[role] = [
			...hookLists[role],
			{
				localKey: nextKey(),
				origOrdinal: null,
				plugin_type: '',
				plugin_config_id: '',
				config_override_text: '',
				config_override_error: null,
				overrideFormValues: {},
				overrideShowJson: false
			}
		];
	}

	function removeHook(role: HookRoleKey, localKey: string) {
		hookLists[role] = hookLists[role].filter((e) => e.localKey !== localKey);
	}

	// ---------------------------------------------------------------------------
	// Save
	// ---------------------------------------------------------------------------

	async function save() {
		if (submitting) return;

		// Validate all overrides.
		let allValid = true;
		for (const role of STANDARD_ROLES) {
			if (!validateStdOverride(role)) allValid = false;
		}
		for (const role of HOOK_ROLES) {
			for (const entry of hookLists[role]) {
				if (!validateHookOverride(entry)) allValid = false;
			}
		}
		if (!allValid) return;

		const standardToUpdate = STANDARD_ROLES.filter((r) => standardStates[r].plugin_type);
		const hasHooks = HOOK_ROLES.some((r) => hookLists[r].length > 0);

		if (standardToUpdate.length === 0 && !hasHooks) {
			showError('Select at least one plugin config to save.');
			return;
		}

		submitting = true;
		try {
			let lastResult: SoftwareItemDetailResponse | undefined;

			// --- Save standard roles ---
			for (const role of standardToUpdate) {
				const s = standardStates[role];
				const fields = getStdFormFields(role);
				const hasFields = fields.length > 0;

				let configOverride: Record<string, unknown> | null = null;
				if (hasFields && !s.overrideShowJson) {
					const obj = unflattenConfig(s.overrideFormValues, fields);
					configOverride = Object.keys(obj).length > 0 ? obj : null;
				} else {
					const text = s.config_override_text.trim();
					configOverride = text ? (JSON.parse(text) as Record<string, unknown>) : null;
				}

				const req: UpdateHostAssignmentRequest = {
					role,
					ordinal: 0,
					...(s.plugin_config_id
						? { plugin_config_id: s.plugin_config_id, config_override: configOverride }
						: {
								plugin_type: s.plugin_type,
								config_override: (() => {
									if (hasFields && !s.overrideShowJson) {
										const obj = unflattenConfig(s.overrideFormValues, fields) as Record<string, unknown>;
										return Object.keys(obj).length > 0 ? obj : null;
									}
									const text = s.config_override_text.trim();
									return text ? (JSON.parse(text) as Record<string, unknown>) : null;
								})()
							}),
					package_identifier: s.package_identifier.trim() || undefined,
					execution_site: s.execution_site
				};
				lastResult = await updateHostAssignment(softwareItemId, hostId, req);
			}

			// --- Reconcile hooks ---
			// Assign sequential ordinals (0, 1, 2…) based on list position.
			for (const role of HOOK_ROLES) {
				const desiredOrdinals = new Set(hookLists[role].map((_, i) => i));
				const origOrdinals = new Set(origOrdinalsByRole[role]);

				// Upsert desired entries (in order, so ordinal = list index).
				for (let i = 0; i < hookLists[role].length; i++) {
					const entry = hookLists[role][i];
					if (!entry.plugin_type) continue;

					const fields = getHookFormFields(entry);
					const hasFields = fields.length > 0;

					let configOverride: Record<string, unknown> | null = null;
					if (hasFields && !entry.overrideShowJson) {
						const obj = unflattenConfig(entry.overrideFormValues, fields);
						configOverride = Object.keys(obj).length > 0 ? obj : null;
					} else {
						const text = entry.config_override_text.trim();
						configOverride = text ? (JSON.parse(text) as Record<string, unknown>) : null;
					}

					const req: UpdateHostAssignmentRequest = {
						role,
						ordinal: i,
						...(entry.plugin_config_id
							? { plugin_config_id: entry.plugin_config_id, config_override: configOverride }
							: {
									plugin_type: entry.plugin_type,
									config_override: (() => {
										if (hasFields && !entry.overrideShowJson) {
											const obj = unflattenConfig(entry.overrideFormValues, fields) as Record<string, unknown>;
											return Object.keys(obj).length > 0 ? obj : null;
										}
										const text = entry.config_override_text.trim();
										return text ? (JSON.parse(text) as Record<string, unknown>) : null;
									})()
								})
					};
					lastResult = await updateHostAssignment(softwareItemId, hostId, req);
				}

				// Delete ordinals that no longer appear in the desired list.
				for (const ordinal of origOrdinals) {
					if (!desiredOrdinals.has(ordinal)) {
						lastResult = await deletePluginAssignment(softwareItemId, hostId, role, ordinal);
					}
				}
			}

			showSuccess('Plugin assignments saved.');
			onsuccess(lastResult!);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to save plugin assignments.');
		} finally {
			submitting = false;
		}
	}
</script>

<Modal title="Configure Plugins" {onclose} maxWidth="max-w-2xl max-h-[90vh] flex flex-col">
	<div class="space-y-1 text-sm text-surface-500">
		<p>
			Editing plugin assignments for <strong class="text-surface-900 dark:text-surface-100">{softwareItemName}</strong>
			on <strong class="text-surface-900 dark:text-surface-100">{hostName}</strong>.
		</p>
		<p class="text-xs">To remove a role entirely, unassign this host and reassign without that role.</p>
	</div>

	{#if loading}
		<p class="text-surface-500">Loading plugin configs…</p>
	{:else if loadError}
		<aside class="rounded-lg p-4 preset-filled-error-500 text-sm">{loadError}</aside>
	{:else}
		<div class="space-y-3">
			{#each RENDER_ORDER as item (item.kind + '_' + item.role)}
				{#if item.kind === 'standard'}
					{@const role = item.role}
					{@const s = standardStates[role]}
					{@const roleFields = getStdFormFields(role)}
					{@const hasFormFields = roleFields.length > 0}
					{@const typeOpts = pluginTypesForRole(role)}
					{@const savedRoleOpts = typeOpts.flatMap((pt) => savedConfigsForType(pt.plugin_type))}
					<div class="rounded-lg border border-surface-200 p-4 space-y-3 dark:border-surface-700">
						<div class="flex items-start gap-2">
							<span class="badge preset-tonal shrink-0 text-xs">{ROLE_LABELS[role]}</span>
							<span class="text-xs text-surface-500 leading-5">{ROLE_DESCRIPTIONS[role]}</span>
						</div>

						<!-- Plugin Config -->
						<div class="grid grid-cols-[9rem_1fr] items-center gap-3">
							<label class="text-sm font-medium" for="cfg-{role}">Plugin Config</label>
							<select
								id="cfg-{role}"
								class="select text-sm"
								value={pluginSelection(s)}
								onchange={(e) => applySelection(standardStates[role], (e.target as HTMLSelectElement).value)}
							>
								<option value="">— not configured —</option>
								{#if savedRoleOpts.length > 0}
									<optgroup label="Saved">
										{#each savedRoleOpts as cfg (cfg.id)}
											<option value="cfg:{cfg.id}">{cfg.name}</option>
										{/each}
									</optgroup>
									<optgroup label="Inline">
										{#each typeOpts as pt (pt.plugin_type)}
											<option value="type:{pt.plugin_type}">{pt.display_name}</option>
										{/each}
									</optgroup>
								{:else}
									{#each typeOpts as pt (pt.plugin_type)}
										<option value="type:{pt.plugin_type}">{pt.display_name}</option>
									{/each}
								{/if}
							</select>
						</div>

						{#if s.plugin_type}
							{@const isInline = !s.plugin_config_id}

							<!-- Package Identifier -->
							<div class="grid grid-cols-[9rem_1fr] items-center gap-3">
								<label class="text-sm font-medium" for="pkg-{role}">Package ID</label>
								<input
									id="pkg-{role}"
									class="input text-sm"
									type="text"
									placeholder="e.g. owner/repo"
									bind:value={standardStates[role].package_identifier}
								/>
							</div>

							{#if role === 'fetch_releases'}
								<div class="grid grid-cols-[9rem_1fr] items-center gap-3">
									<label class="text-sm font-medium" for="site-{role}">Execution Site</label>
									<select id="site-{role}" class="select text-sm" bind:value={standardStates[role].execution_site}>
										<option value="auto">Auto (recommended)</option>
										<option value="agent">Agent</option>
										<option value="controller">Controller</option>
									</select>
								</div>
							{/if}

							{#if isInline}
								<!-- Inline config form — not collapsible, required -->
								{#if hasFormFields && !standardStates[role].overrideShowJson}
									<div class="space-y-2">
										{#each roleFields as field (field.key)}
											{#if isStdOverrideFieldVisible(field, role)}
												<div>
													<label for="ovr-{role}-{field.key}" class="mb-1 block text-xs font-medium"
														>{field.label}</label
													>
													{#if field.field_type === 'textarea'}
														<textarea
															id="ovr-{role}-{field.key}"
															bind:value={standardStates[role].overrideFormValues[field.key]}
															placeholder={field.placeholder ?? ''}
															class="textarea font-mono text-xs w-full"
															rows="3"
														></textarea>
													{:else if field.field_type === 'select'}
														<select
															id="ovr-{role}-{field.key}"
															bind:value={standardStates[role].overrideFormValues[field.key]}
															class="select text-xs w-full"
														>
															<option value=""></option>
															{#each resolvedOptions(field) as opt (opt.value)}
																<option value={opt.value}>{opt.label}</option>
															{/each}
														</select>
													{:else if field.field_type === 'toggle'}
														<label class="flex items-center gap-2">
															<input
																type="checkbox"
																id="ovr-{role}-{field.key}"
																checked={standardStates[role].overrideFormValues[field.key] === 'true'}
																onchange={(e) => {
																	standardStates[role].overrideFormValues[field.key] = String(
																		(e.target as HTMLInputElement).checked
																	);
																}}
																class="checkbox"
															/>
															<span class="text-xs">{field.help_text ?? ''}</span>
														</label>
													{:else}
														<input
															id="ovr-{role}-{field.key}"
															type={field.field_type === 'password' ? 'password' : 'text'}
															bind:value={standardStates[role].overrideFormValues[field.key]}
															placeholder={field.placeholder ?? ''}
															class="input text-xs w-full"
														/>
													{/if}
													{#if field.help_text && field.field_type !== 'toggle'}
														<p class="mt-0.5 text-xs text-surface-400">{field.help_text}</p>
													{/if}
												</div>
											{/if}
										{/each}
										<button
											type="button"
											class="btn btn-sm preset-tonal text-xs"
											onclick={() => {
												const obj = unflattenConfig(standardStates[role].overrideFormValues, roleFields);
												standardStates[role].config_override_text =
													Object.keys(obj).length > 0 ? JSON.stringify(obj, null, 2) : '';
												standardStates[role].overrideShowJson = true;
											}}>Edit as JSON</button
										>
									</div>
								{:else if hasFormFields && standardStates[role].overrideShowJson}
									<div class="mt-2 space-y-1">
										<textarea
											class="textarea font-mono text-xs"
											rows={4}
											placeholder={'{\n  "example_field": "value"\n}'}
											bind:value={standardStates[role].config_override_text}
											onblur={() => validateStdOverride(role)}
										></textarea>
										{#if s.config_override_error}
											<p class="text-xs rounded px-2 py-1 preset-filled-error-500">{s.config_override_error}</p>
										{/if}
										<p class="text-xs text-surface-400">Full plugin configuration as JSON.</p>
										<button
											type="button"
											class="btn btn-sm preset-tonal text-xs"
											onclick={() => {
												try {
													const parsed = standardStates[role].config_override_text.trim()
														? (JSON.parse(standardStates[role].config_override_text) as Record<string, unknown>)
														: {};
													standardStates[role].overrideFormValues = flattenConfig(parsed, roleFields);
													standardStates[role].overrideShowJson = false;
													standardStates[role].config_override_error = null;
												} catch {
													showError('Config must be valid JSON to switch back to form view.');
												}
											}}>Back to Form</button
										>
									</div>
								{:else}
									<!-- No form fields: plain JSON textarea -->
									<div class="mt-2 space-y-1">
										<p class="text-xs font-medium">Config</p>
										<textarea
											class="textarea font-mono text-xs"
											rows={4}
											placeholder={'{\n  "example_field": "value"\n}'}
											bind:value={standardStates[role].config_override_text}
											onblur={() => validateStdOverride(role)}
										></textarea>
										{#if s.config_override_error}
											<p class="text-xs rounded px-2 py-1 preset-filled-error-500">{s.config_override_error}</p>
										{/if}
										<p class="text-xs text-surface-400">Full plugin configuration as JSON.</p>
									</div>
								{/if}
							{:else}
								<!-- Saved config: collapsible Config Override (advanced) -->
								<details>
									<summary class="cursor-pointer select-none text-xs text-surface-500 hover:text-surface-700">
										Config Override <span class="opacity-60">(advanced)</span>
										{#if hasStdOverride(role)}
											<span class="ml-1 badge preset-tonal-warning text-xs">set</span>
										{/if}
									</summary>

									{#if hasFormFields && !standardStates[role].overrideShowJson}
										<div class="mt-2 space-y-2">
											{#each roleFields as field (field.key)}
												{#if isStdOverrideFieldVisible(field, role)}
													<div>
														<label for="ovr-{role}-{field.key}" class="mb-1 block text-xs font-medium"
															>{field.label}</label
														>
														{#if field.field_type === 'textarea'}
															<textarea
																id="ovr-{role}-{field.key}"
																bind:value={standardStates[role].overrideFormValues[field.key]}
																placeholder={field.placeholder}
																class="textarea font-mono text-xs w-full"
																rows="3"
															></textarea>
														{:else if field.field_type === 'select'}
															<select
																id="ovr-{role}-{field.key}"
																bind:value={standardStates[role].overrideFormValues[field.key]}
																class="select text-xs w-full"
															>
																<option value="">— keep base config —</option>
																{#each resolvedOptions(field) as opt (opt.value)}
																	<option value={opt.value}>{opt.label}</option>
																{/each}
															</select>
														{:else if field.field_type === 'toggle'}
															<label class="flex items-center gap-2">
																<input
																	type="checkbox"
																	id="ovr-{role}-{field.key}"
																	checked={standardStates[role].overrideFormValues[field.key] === 'true'}
																	onchange={(e) => {
																		standardStates[role].overrideFormValues[field.key] = String(
																			(e.target as HTMLInputElement).checked
																		);
																	}}
																	class="checkbox"
																/>
																<span class="text-xs">{field.help_text ?? ''}</span>
															</label>
														{:else}
															<input
																id="ovr-{role}-{field.key}"
																type={field.field_type === 'password' ? 'password' : 'text'}
																bind:value={standardStates[role].overrideFormValues[field.key]}
																placeholder={field.placeholder ?? 'Leave blank to keep base config value'}
																class="input text-xs w-full"
															/>
														{/if}
														{#if field.help_text && field.field_type !== 'toggle'}
															<p class="mt-0.5 text-xs text-surface-400">{field.help_text}</p>
														{/if}
													</div>
												{/if}
											{/each}
											<p class="text-xs text-surface-400">Leave fields blank to use the base plugin config value.</p>
											<button
												type="button"
												class="btn btn-sm preset-tonal text-xs"
												onclick={() => {
													const obj = unflattenConfig(standardStates[role].overrideFormValues, roleFields);
													standardStates[role].config_override_text =
														Object.keys(obj).length > 0 ? JSON.stringify(obj, null, 2) : '';
													standardStates[role].overrideShowJson = true;
												}}>Advanced: Edit as JSON</button
											>
										</div>
									{:else if hasFormFields && standardStates[role].overrideShowJson}
										<div class="mt-2 space-y-1">
											<textarea
												class="textarea font-mono text-xs"
												rows={4}
												placeholder={'{\n  "example_field": "value"\n}'}
												bind:value={standardStates[role].config_override_text}
												onblur={() => validateStdOverride(role)}
											></textarea>
											{#if s.config_override_error}
												<p class="text-xs rounded px-2 py-1 preset-filled-error-500">{s.config_override_error}</p>
											{/if}
											<p class="text-xs text-surface-400">
												JSON object merged on top of the plugin config. Leave empty to clear.
											</p>
											<button
												type="button"
												class="btn btn-sm preset-tonal text-xs"
												onclick={() => {
													try {
														const parsed = standardStates[role].config_override_text.trim()
															? (JSON.parse(standardStates[role].config_override_text) as Record<string, unknown>)
															: {};
														standardStates[role].overrideFormValues = flattenConfig(parsed, roleFields);
														standardStates[role].overrideShowJson = false;
														standardStates[role].config_override_error = null;
													} catch {
														showError('Config must be valid JSON to switch back to form view.');
													}
												}}>Back to Form</button
											>
										</div>
									{:else}
										<div class="mt-2 space-y-1">
											<textarea
												class="textarea font-mono text-xs"
												rows={4}
												placeholder={'{\n  "example_field": "value"\n}'}
												bind:value={standardStates[role].config_override_text}
												onblur={() => validateStdOverride(role)}
											></textarea>
											{#if s.config_override_error}
												<p class="text-xs rounded px-2 py-1 preset-filled-error-500">{s.config_override_error}</p>
											{/if}
											<p class="text-xs text-surface-400">
												JSON object merged on top of the plugin config. Leave empty to clear.
											</p>
										</div>
									{/if}
								</details>
							{/if}
						{/if}
					</div>
				{:else}
					{@const hookRole = item.role}
					<div class="rounded-lg border border-surface-200 p-4 space-y-3 dark:border-surface-700">
						<div class="flex items-center justify-between">
							<div class="flex items-start gap-2">
								<span class="badge preset-tonal shrink-0 text-xs">{ROLE_LABELS[hookRole]}s</span>
								<span class="text-xs text-surface-500 leading-5">{ROLE_DESCRIPTIONS[hookRole]}</span>
							</div>
							<button
								type="button"
								class="btn btn-sm preset-tonal-primary text-xs shrink-0"
								onclick={() => addHook(hookRole)}
							>
								+ Add
							</button>
						</div>

						{#if hookLists[hookRole].length === 0}
							<p class="text-xs text-surface-400 italic">No {ROLE_LABELS[hookRole].toLowerCase()}s configured.</p>
						{:else}
							<div class="space-y-2">
								{#each hookLists[hookRole] as entry, idx (entry.localKey)}
									{@const hookFields = getHookFormFields(entry)}
									{@const hasHookFormFields = hookFields.length > 0}
									{@const hookTypeOpts = hookPluginTypes()}
									{@const savedHookOpts = hookTypeOpts.flatMap((pt) => savedConfigsForType(pt.plugin_type))}
									<div class="rounded-md border border-surface-300 p-3 space-y-2 dark:border-surface-600">
										<div class="flex items-center justify-between gap-2">
											<span class="text-xs font-medium text-surface-500">Hook #{idx + 1}</span>
											<button
												type="button"
												class="btn btn-sm preset-tonal-error text-xs"
												onclick={() => removeHook(hookRole, entry.localKey)}
											>
												Remove
											</button>
										</div>

										<!-- Plugin Config (saved + inline in one select) -->
										<div class="grid grid-cols-[7rem_1fr] items-center gap-2">
											<label class="text-sm font-medium" for="hook-cfg-{entry.localKey}">Plugin Config</label>
											<select
												id="hook-cfg-{entry.localKey}"
												class="select text-sm"
												value={pluginSelection(entry)}
												onchange={(e) => applySelection(entry, (e.target as HTMLSelectElement).value)}
											>
												<option value="">— not configured —</option>
												{#if savedHookOpts.length > 0}
													<optgroup label="Saved">
														{#each savedHookOpts as cfg (cfg.id)}
															<option value="cfg:{cfg.id}">{cfg.name}</option>
														{/each}
													</optgroup>
													<optgroup label="Inline">
														{#each hookTypeOpts as pt (pt.plugin_type)}
															<option value="type:{pt.plugin_type}">{pt.display_name}</option>
														{/each}
													</optgroup>
												{:else}
													{#each hookTypeOpts as pt (pt.plugin_type)}
														<option value="type:{pt.plugin_type}">{pt.display_name}</option>
													{/each}
												{/if}
											</select>
										</div>

										{#if entry.plugin_type}
											{@const isHookInline = !entry.plugin_config_id}

											{#if isHookInline}
												<!-- Inline config form — not collapsible, required -->
												{#if hasHookFormFields && !entry.overrideShowJson}
													<div class="space-y-2">
														{#each hookFields as field (field.key)}
															{#if isHookOverrideFieldVisible(field, entry)}
																<div>
																	<label
																		for="hook-ovr-{entry.localKey}-{field.key}"
																		class="mb-1 block text-xs font-medium">{field.label}</label
																	>
																	{#if field.field_type === 'textarea'}
																		<textarea
																			id="hook-ovr-{entry.localKey}-{field.key}"
																			bind:value={entry.overrideFormValues[field.key]}
																			placeholder={field.placeholder ?? ''}
																			class="textarea font-mono text-xs w-full"
																			rows="3"
																		></textarea>
																	{:else if field.field_type === 'select'}
																		<select
																			id="hook-ovr-{entry.localKey}-{field.key}"
																			bind:value={entry.overrideFormValues[field.key]}
																			class="select text-xs w-full"
																		>
																			<option value=""></option>
																			{#each resolvedOptions(field) as opt (opt.value)}
																				<option value={opt.value}>{opt.label}</option>
																			{/each}
																		</select>
																	{:else if field.field_type === 'toggle'}
																		<label class="flex items-center gap-2">
																			<input
																				type="checkbox"
																				id="hook-ovr-{entry.localKey}-{field.key}"
																				checked={entry.overrideFormValues[field.key] === 'true'}
																				onchange={(e) => {
																					entry.overrideFormValues[field.key] = String(
																						(e.target as HTMLInputElement).checked
																					);
																				}}
																				class="checkbox"
																			/>
																			<span class="text-xs">{field.help_text ?? ''}</span>
																		</label>
																	{:else}
																		<input
																			id="hook-ovr-{entry.localKey}-{field.key}"
																			type={field.field_type === 'password' ? 'password' : 'text'}
																			bind:value={entry.overrideFormValues[field.key]}
																			placeholder={field.placeholder ?? ''}
																			class="input text-xs w-full"
																		/>
																	{/if}
																	{#if field.help_text && field.field_type !== 'toggle'}
																		<p class="mt-0.5 text-xs text-surface-400">{field.help_text}</p>
																	{/if}
																</div>
															{/if}
														{/each}
														<button
															type="button"
															class="btn btn-sm preset-tonal text-xs"
															onclick={() => {
																const obj = unflattenConfig(entry.overrideFormValues, hookFields);
																entry.config_override_text =
																	Object.keys(obj).length > 0 ? JSON.stringify(obj, null, 2) : '';
																entry.overrideShowJson = true;
															}}>Edit as JSON</button
														>
													</div>
												{:else if hasHookFormFields && entry.overrideShowJson}
													<div class="mt-2 space-y-1">
														<textarea
															class="textarea font-mono text-xs"
															rows={4}
															placeholder={'{\n  "example_field": "value"\n}'}
															bind:value={entry.config_override_text}
															onblur={() => validateHookOverride(entry)}
														></textarea>
														{#if entry.config_override_error}
															<p class="text-xs rounded px-2 py-1 preset-filled-error-500">
																{entry.config_override_error}
															</p>
														{/if}
														<p class="text-xs text-surface-400">Full plugin configuration as JSON.</p>
														<button
															type="button"
															class="btn btn-sm preset-tonal text-xs"
															onclick={() => {
																try {
																	const parsed = entry.config_override_text.trim()
																		? (JSON.parse(entry.config_override_text) as Record<string, unknown>)
																		: {};
																	entry.overrideFormValues = flattenConfig(parsed, hookFields);
																	entry.overrideShowJson = false;
																	entry.config_override_error = null;
																} catch {
																	showError('Config must be valid JSON to switch back to form view.');
																}
															}}>Back to Form</button
														>
													</div>
												{:else}
													<!-- No form fields: plain JSON textarea -->
													<div class="mt-2 space-y-1">
														<p class="text-xs font-medium">Config</p>
														<textarea
															class="textarea font-mono text-xs"
															rows={4}
															placeholder={'{\n  "example_field": "value"\n}'}
															bind:value={entry.config_override_text}
															onblur={() => validateHookOverride(entry)}
														></textarea>
														{#if entry.config_override_error}
															<p class="text-xs rounded px-2 py-1 preset-filled-error-500">
																{entry.config_override_error}
															</p>
														{/if}
														<p class="text-xs text-surface-400">Full plugin configuration as JSON.</p>
													</div>
												{/if}
											{:else}
												<!-- Saved config: collapsible Config Override (advanced) -->
												<details>
													<summary class="cursor-pointer select-none text-xs text-surface-500 hover:text-surface-700">
														Config Override <span class="opacity-60">(advanced)</span>
														{#if hasHookOverride(entry)}
															<span class="ml-1 badge preset-tonal-warning text-xs">set</span>
														{/if}
													</summary>

													{#if hasHookFormFields && !entry.overrideShowJson}
														<div class="mt-2 space-y-2">
															{#each hookFields as field (field.key)}
																{#if isHookOverrideFieldVisible(field, entry)}
																	<div>
																		<label
																			for="hook-ovr-{entry.localKey}-{field.key}"
																			class="mb-1 block text-xs font-medium">{field.label}</label
																		>
																		{#if field.field_type === 'textarea'}
																			<textarea
																				id="hook-ovr-{entry.localKey}-{field.key}"
																				bind:value={entry.overrideFormValues[field.key]}
																				placeholder={field.placeholder}
																				class="textarea font-mono text-xs w-full"
																				rows="3"
																			></textarea>
																		{:else if field.field_type === 'select'}
																			<select
																				id="hook-ovr-{entry.localKey}-{field.key}"
																				bind:value={entry.overrideFormValues[field.key]}
																				class="select text-xs w-full"
																			>
																				<option value="">— keep base config —</option>
																				{#each resolvedOptions(field) as opt (opt.value)}
																					<option value={opt.value}>{opt.label}</option>
																				{/each}
																			</select>
																		{:else if field.field_type === 'toggle'}
																			<label class="flex items-center gap-2">
																				<input
																					type="checkbox"
																					id="hook-ovr-{entry.localKey}-{field.key}"
																					checked={entry.overrideFormValues[field.key] === 'true'}
																					onchange={(e) => {
																						entry.overrideFormValues[field.key] = String(
																							(e.target as HTMLInputElement).checked
																						);
																					}}
																					class="checkbox"
																				/>
																				<span class="text-xs">{field.help_text ?? ''}</span>
																			</label>
																		{:else}
																			<input
																				id="hook-ovr-{entry.localKey}-{field.key}"
																				type={field.field_type === 'password' ? 'password' : 'text'}
																				bind:value={entry.overrideFormValues[field.key]}
																				placeholder={field.placeholder ?? 'Leave blank to keep base config value'}
																				class="input text-xs w-full"
																			/>
																		{/if}
																		{#if field.help_text && field.field_type !== 'toggle'}
																			<p class="mt-0.5 text-xs text-surface-400">{field.help_text}</p>
																		{/if}
																	</div>
																{/if}
															{/each}
															<p class="text-xs text-surface-400">
																Leave fields blank to use the base plugin config value.
															</p>
															<button
																type="button"
																class="btn btn-sm preset-tonal text-xs"
																onclick={() => {
																	const obj = unflattenConfig(entry.overrideFormValues, hookFields);
																	entry.config_override_text =
																		Object.keys(obj).length > 0 ? JSON.stringify(obj, null, 2) : '';
																	entry.overrideShowJson = true;
																}}>Advanced: Edit as JSON</button
															>
														</div>
													{:else if hasHookFormFields && entry.overrideShowJson}
														<div class="mt-2 space-y-1">
															<textarea
																class="textarea font-mono text-xs"
																rows={4}
																placeholder={'{\n  "example_field": "value"\n}'}
																bind:value={entry.config_override_text}
																onblur={() => validateHookOverride(entry)}
															></textarea>
															{#if entry.config_override_error}
																<p class="text-xs rounded px-2 py-1 preset-filled-error-500">
																	{entry.config_override_error}
																</p>
															{/if}
															<p class="text-xs text-surface-400">
																JSON object merged on top of the plugin config. Leave empty to clear.
															</p>
															<button
																type="button"
																class="btn btn-sm preset-tonal text-xs"
																onclick={() => {
																	try {
																		const parsed = entry.config_override_text.trim()
																			? (JSON.parse(entry.config_override_text) as Record<string, unknown>)
																			: {};
																		entry.overrideFormValues = flattenConfig(parsed, hookFields);
																		entry.overrideShowJson = false;
																		entry.config_override_error = null;
																	} catch {
																		showError('Config must be valid JSON to switch back to form view.');
																	}
																}}>Back to Form</button
															>
														</div>
													{:else}
														<div class="mt-2 space-y-1">
															<textarea
																class="textarea font-mono text-xs"
																rows={4}
																placeholder={'{\n  "example_field": "value"\n}'}
																bind:value={entry.config_override_text}
																onblur={() => validateHookOverride(entry)}
															></textarea>
															{#if entry.config_override_error}
																<p class="text-xs rounded px-2 py-1 preset-filled-error-500">
																	{entry.config_override_error}
																</p>
															{/if}
															<p class="text-xs text-surface-400">
																JSON object merged on top of the plugin config. Leave empty to clear.
															</p>
														</div>
													{/if}
												</details>
											{/if}
										{/if}
									</div>
								{/each}
							</div>
						{/if}
					</div>
				{/if}
			{/each}
		</div>
	{/if}

	{#snippet footer()}
		<button class="btn preset-tonal-surface" onclick={onclose}>Cancel</button>
		<button class="btn preset-filled-primary-500" onclick={save} disabled={submitting || loading || !!loadError}>
			{submitting ? 'Saving…' : 'Save Changes'}
		</button>
	{/snippet}
</Modal>
