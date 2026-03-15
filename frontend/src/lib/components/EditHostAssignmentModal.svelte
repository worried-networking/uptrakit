<script lang="ts">
	import { onMount } from 'svelte';
	import Modal from './Modal.svelte';
	import { getPluginConfigs, updateHostAssignment, listPluginTypes } from '$lib/api';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import type {
		FieldDef,
		HostPluginRoleSummary,
		PluginConfigResponse,
		PluginTypeInfo,
		SelectOption,
		SoftwareItemDetailResponse,
		UpdateHostAssignmentRequest
	} from '$lib/types';

	type RoleKey = 'detect_version' | 'fetch_releases' | 'execute_update' | 'pre_update_hook' | 'post_update_hook';

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

	const ALL_ROLES: RoleKey[] = [
		'detect_version',
		'fetch_releases',
		'execute_update',
		'pre_update_hook',
		'post_update_hook'
	];

	interface RoleState {
		plugin_config_id: string;
		package_identifier: string;
		execution_site: string;
		config_override_text: string;
		config_override_error: string | null;
	}

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

	const HOOK_ROLES: RoleKey[] = ['pre_update_hook', 'post_update_hook'];

	function isHookPluginType(pluginType: string): boolean {
		const pt = pluginTypes.find((t) => t.plugin_type === pluginType);
		if (!pt) return false;
		return pt.capabilities.some((c) => c === 'pre_update_hook' || c === 'post_update_hook');
	}

	function isConfigCompatibleWithRole(config: PluginConfigResponse, role: RoleKey): boolean {
		const isHook = isHookPluginType(config.plugin_type);
		if (HOOK_ROLES.includes(role)) return isHook;
		return !isHook;
	}

	function makeInitialState(): Record<RoleKey, RoleState> {
		const result = {} as Record<RoleKey, RoleState>;
		for (const role of ALL_ROLES) {
			const existing = existingPlugins.find((p) => p.role === role);
			result[role] = existing
				? {
						plugin_config_id: existing.plugin_config_id ?? '',
						package_identifier: existing.package_identifier,
						execution_site: existing.execution_site || 'auto',
						config_override_text: existing.config_override ? JSON.stringify(existing.config_override, null, 2) : '',
						config_override_error: null
					}
				: {
						plugin_config_id: '',
						package_identifier: '',
						execution_site: 'auto',
						config_override_text: '',
						config_override_error: null
					};
		}
		return result;
	}

	let roleStates: Record<RoleKey, RoleState> = $state(makeInitialState());
	let pluginConfigs: PluginConfigResponse[] = $state([]);
	let pluginTypes: PluginTypeInfo[] = $state([]);
	let loading: boolean = $state(true);
	let loadError: string | null = $state(null);
	let submitting: boolean = $state(false);

	// Per-role override form state (flat dot-path values)
	let overrideFormValues: Record<RoleKey, Record<string, string>> = $state({
		detect_version: {},
		fetch_releases: {},
		execute_update: {},
		pre_update_hook: {},
		post_update_hook: {}
	});

	// Per-role toggle: true = JSON editor mode, false = form mode
	let overrideShowJson: Record<RoleKey, boolean> = $state({
		detect_version: false,
		fetch_releases: false,
		execute_update: false,
		pre_update_hook: false,
		post_update_hook: false
	});

	onMount(async () => {
		try {
			const [configsResult, typesResult] = await Promise.all([getPluginConfigs(1, 500), listPluginTypes()]);
			pluginConfigs = configsResult.items;
			pluginTypes = typesResult;

			// Initialise override form values from existing config_override
			for (const role of ALL_ROLES) {
				const existing = existingPlugins.find((p) => p.role === role);
				if (existing) {
					const config = pluginConfigs.find((c) => c.id === existing.plugin_config_id);
					if (config) {
						const fields = getFormFields(config.plugin_type);
						overrideFormValues[role] = flattenConfig(
							(existing.config_override as Record<string, unknown>) ?? {},
							fields
						);
					}
				}
			}
		} catch (e) {
			loadError = e instanceof Error ? e.message : 'Failed to load plugin configs.';
		} finally {
			loading = false;
		}
	});

	// --- Form field helpers (mirrors plugin-configs page logic) ---

	function getFormFields(pluginType: string): FieldDef[] {
		const t = pluginTypes.find((pt) => pt.plugin_type === pluginType);
		return t?.config_form_fields ?? [];
	}

	function getRoleFormFields(role: RoleKey): FieldDef[] {
		const configId = roleStates[role].plugin_config_id;
		if (!configId) return [];
		const config = pluginConfigs.find((c) => c.id === configId);
		if (!config) return [];
		return getFormFields(config.plugin_type);
	}

	function resolvedOptions(field: FieldDef): SelectOption[] {
		return field.options ?? [];
	}

	function isOverrideFieldVisible(field: FieldDef, role: RoleKey): boolean {
		if (!field.visible_when) return true;
		const controlValue = overrideFormValues[role][field.visible_when.field] ?? '';
		return field.visible_when.values.includes(controlValue);
	}

	/** Flatten a nested config JSON object into dot-path string values for form fields. */
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

	/** Unflatten dot-path form values into a nested JSON config object. */
	function unflattenConfig(formValues: Record<string, string>, fields: FieldDef[]): Record<string, unknown> {
		const result: Record<string, unknown> = {};

		for (const field of fields) {
			const raw = formValues[field.key] ?? '';

			if (raw === '' && field.field_type !== 'toggle') continue;

			let value: unknown;
			if (field.field_type === 'toggle') {
				value = raw === 'true';
				// In override context, skip false toggles (leave base config value in place)
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
					if (target[key] == null || typeof target[key] !== 'object') {
						target[key] = {};
					}
					target = target[key] as Record<string, unknown>;
				}
				const lastKey = parts[parts.length - 1];
				const jsonKey = lastKey.startsWith('_') ? lastKey.slice(1) : lastKey;
				target[jsonKey] = value;
			}
		}

		// Clean up empty nested objects
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

	/** Whether this role has any non-empty config override set. */
	function hasOverride(role: RoleKey): boolean {
		const fields = getRoleFormFields(role);
		if (fields.length > 0 && !overrideShowJson[role]) {
			const obj = unflattenConfig(overrideFormValues[role], fields);
			return Object.keys(obj).length > 0;
		}
		return !!roleStates[role].config_override_text.trim();
	}

	function validateOverride(role: RoleKey): boolean {
		const fields = getRoleFormFields(role);
		// In form mode, form inputs always produce valid values
		if (fields.length > 0 && !overrideShowJson[role]) {
			roleStates[role].config_override_error = null;
			return true;
		}

		const text = roleStates[role].config_override_text.trim();
		if (!text) {
			roleStates[role].config_override_error = null;
			return true;
		}
		try {
			const parsed: unknown = JSON.parse(text);
			if (typeof parsed !== 'object' || Array.isArray(parsed) || parsed === null) {
				roleStates[role].config_override_error = 'Must be a JSON object, not an array or primitive.';
				return false;
			}
			roleStates[role].config_override_error = null;
			return true;
		} catch {
			roleStates[role].config_override_error = 'Invalid JSON.';
			return false;
		}
	}

	async function save() {
		if (submitting) return;

		let allValid = true;
		for (const role of ALL_ROLES) {
			if (!validateOverride(role)) allValid = false;
		}
		if (!allValid) return;

		const toUpdate = ALL_ROLES.filter((role) => roleStates[role].plugin_config_id);
		if (toUpdate.length === 0) {
			showError('Select at least one plugin config to save.');
			return;
		}

		submitting = true;
		try {
			let lastResult: SoftwareItemDetailResponse | undefined;
			for (const role of toUpdate) {
				const s = roleStates[role];
				const fields = getRoleFormFields(role);
				const hasFields = fields.length > 0;

				let configOverride: Record<string, unknown> | null = null;
				if (hasFields && !overrideShowJson[role]) {
					const overrideObj = unflattenConfig(overrideFormValues[role], fields);
					configOverride = Object.keys(overrideObj).length > 0 ? overrideObj : null;
				} else {
					const overrideText = s.config_override_text.trim();
					configOverride = overrideText ? (JSON.parse(overrideText) as Record<string, unknown>) : null;
				}

				const req: UpdateHostAssignmentRequest = {
					role,
					plugin_config_id: s.plugin_config_id,
					package_identifier: s.package_identifier.trim() || undefined,
					execution_site: s.execution_site,
					config_override: configOverride
				};
				lastResult = await updateHostAssignment(softwareItemId, hostId, req);
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
			{#each ALL_ROLES as role (role)}
				{@const s = roleStates[role]}
				{@const roleFields = getRoleFormFields(role)}
				{@const hasFormFields = roleFields.length > 0}
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
							bind:value={roleStates[role].plugin_config_id}
							onchange={() => {
								const config = pluginConfigs.find((c) => c.id === roleStates[role].plugin_config_id);
								const fields = config ? getFormFields(config.plugin_type) : [];
								overrideFormValues[role] = flattenConfig({}, fields);
								overrideShowJson[role] = false;
								roleStates[role].config_override_text = '';
								roleStates[role].config_override_error = null;
							}}
						>
							<option value="">— not configured —</option>
							{#each pluginConfigs.filter((c) => isConfigCompatibleWithRole(c, role)) as cfg (cfg.id)}
								<option value={cfg.id}>{cfg.name}</option>
							{/each}
						</select>
					</div>

					{#if s.plugin_config_id}
						{#if !HOOK_ROLES.includes(role)}
							<!-- Package Identifier -->
							<div class="grid grid-cols-[9rem_1fr] items-center gap-3">
								<label class="text-sm font-medium" for="pkg-{role}">Package ID</label>
								<input
									id="pkg-{role}"
									class="input text-sm"
									type="text"
									placeholder="e.g. owner/repo"
									bind:value={roleStates[role].package_identifier}
								/>
							</div>

							<!-- Execution Site (only for fetch_releases) -->
							{#if role === 'fetch_releases'}
								<div class="grid grid-cols-[9rem_1fr] items-center gap-3">
									<label class="text-sm font-medium" for="site-{role}">Execution Site</label>
									<select id="site-{role}" class="select text-sm" bind:value={roleStates[role].execution_site}>
										<option value="auto">Auto (recommended)</option>
										<option value="agent">Agent</option>
										<option value="controller">Controller</option>
									</select>
								</div>
							{/if}
						{/if}

						<!-- Config Override (advanced, collapsible) -->
						<details>
							<summary class="cursor-pointer select-none text-xs text-surface-500 hover:text-surface-700">
								Config Override <span class="opacity-60">(advanced)</span>
								{#if hasOverride(role)}
									<span class="ml-1 badge preset-tonal-warning text-xs">set</span>
								{/if}
							</summary>

							{#if hasFormFields && !overrideShowJson[role]}
								<!-- Form mode -->
								<div class="mt-2 space-y-2">
									{#each roleFields as field (field.key)}
										{#if isOverrideFieldVisible(field, role)}
											<div>
												<label for="ovr-{role}-{field.key}" class="mb-1 block text-xs font-medium">
													{field.label}
												</label>

												{#if field.field_type === 'textarea'}
													<textarea
														id="ovr-{role}-{field.key}"
														bind:value={overrideFormValues[role][field.key]}
														placeholder={field.placeholder}
														class="textarea font-mono text-xs w-full"
														rows="3"
													></textarea>
												{:else if field.field_type === 'select'}
													<select
														id="ovr-{role}-{field.key}"
														bind:value={overrideFormValues[role][field.key]}
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
															checked={overrideFormValues[role][field.key] === 'true'}
															onchange={(e) => {
																overrideFormValues[role][field.key] = String((e.target as HTMLInputElement).checked);
															}}
															class="checkbox"
														/>
														<span class="text-xs">{field.help_text ?? ''}</span>
													</label>
												{:else}
													<input
														id="ovr-{role}-{field.key}"
														type={field.field_type === 'password' ? 'password' : 'text'}
														bind:value={overrideFormValues[role][field.key]}
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
										Leave fields blank to use the base plugin config value. Only filled fields are applied as an
										override.
									</p>

									<button
										type="button"
										class="btn btn-sm preset-tonal text-xs"
										onclick={() => {
											const obj = unflattenConfig(overrideFormValues[role], roleFields);
											roleStates[role].config_override_text =
												Object.keys(obj).length > 0 ? JSON.stringify(obj, null, 2) : '';
											overrideShowJson[role] = true;
										}}
									>
										Advanced: Edit as JSON
									</button>
								</div>
							{:else if hasFormFields && overrideShowJson[role]}
								<!-- JSON editor mode (with form available) -->
								<div class="mt-2 space-y-1">
									<textarea
										class="textarea font-mono text-xs"
										rows={4}
										placeholder={`{\n  "example_field": "value"\n}`}
										bind:value={roleStates[role].config_override_text}
										onblur={() => validateOverride(role)}
									></textarea>
									{#if s.config_override_error}
										<p class="text-xs rounded px-2 py-1 preset-filled-error-500">
											{s.config_override_error}
										</p>
									{/if}
									<p class="text-xs text-surface-400">
										JSON object merged on top of the plugin config. Leave empty to clear any existing override.
									</p>
									<button
										type="button"
										class="btn btn-sm preset-tonal text-xs"
										onclick={() => {
											try {
												const parsed = roleStates[role].config_override_text.trim()
													? (JSON.parse(roleStates[role].config_override_text) as Record<string, unknown>)
													: {};
												overrideFormValues[role] = flattenConfig(parsed, roleFields);
												overrideShowJson[role] = false;
												roleStates[role].config_override_error = null;
											} catch {
												showError('Config must be valid JSON to switch back to form view.');
											}
										}}
									>
										Back to Form
									</button>
								</div>
							{:else}
								<!-- No form fields — plain JSON editor -->
								<div class="mt-2 space-y-1">
									<textarea
										class="textarea font-mono text-xs"
										rows={4}
										placeholder={`{\n  "example_field": "value"\n}`}
										bind:value={roleStates[role].config_override_text}
										onblur={() => validateOverride(role)}
									></textarea>
									{#if s.config_override_error}
										<p class="text-xs rounded px-2 py-1 preset-filled-error-500">
											{s.config_override_error}
										</p>
									{/if}
									<p class="text-xs text-surface-400">
										JSON object merged on top of the plugin config. Leave empty to clear any existing override.
									</p>
								</div>
							{/if}
						</details>
					{/if}
				</div>
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
