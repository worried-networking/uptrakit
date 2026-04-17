<script lang="ts">
	import type { FormField, SelectOption } from '$lib/types';
	import { apiGet } from '$lib/api';
	import { showError } from '$lib/notifications.svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import CheckboxList from '$lib/components/CheckboxList.svelte';

	let {
		fields,
		onsubmit,
		submitLabel = 'Submit',
		loading = false,
		formId = undefined,
		hideSubmit = false,
		extraParams = {},
		loadInitialValues,
		loadSelectOptions
	}: {
		fields: FormField[];
		onsubmit: (values: Record<string, unknown>) => Promise<void>;
		submitLabel?: string;
		loading?: boolean;
		formId?: string;
		hideSubmit?: boolean;
		extraParams?: Record<string, unknown>;
		loadInitialValues?: () => Promise<Record<string, unknown>>;
		loadSelectOptions?: (actionId: string) => Promise<SelectOption[]>;
	} = $props();

	let values: Record<string, string> = $state({});
	// Separate state for multi_select fields — stores selected values as SvelteSet<string>.
	let multiSets: Record<string, SvelteSet<string>> = $state({});
	let dynamicOptions: Record<string, SelectOption[]> = $state({});
	let loadingOptions: Record<string, boolean> = $state({});

	// Non-reactive loader bookkeeping keyed by field name. This is intentionally
	// separate from render state so option loading can be invalidated when fields
	// are reused without creating effect update loops.
	const loadedOptionSourceByField: Record<string, string> = {};
	const activeOptionRequestByField: Record<string, string> = {};

	let preLoading: boolean = $state(false);

	$effect(() => {
		const initial: Record<string, string> = {};
		const initialMulti: Record<string, SvelteSet<string>> = {};
		const rowData = extraParams._row as Record<string, unknown> | undefined;
		for (const f of fields) {
			if (f.field_type === 'multi_select') {
				initialMulti[f.key] = new SvelteSet<string>();
			} else if (rowData && rowData[f.key] != null) {
				initial[f.key] = String(rowData[f.key]);
			} else {
				initial[f.key] = f.default_value ?? '';
			}
		}
		values = initial;
		multiSets = initialMulti;

		const loadFormValues = loadInitialValues;

		if (loadFormValues) {
			preLoading = true;
			loadFormValues()
				.then((obj) => {
					for (const f of fields) {
						if (obj[f.key] != null) {
							values[f.key] = String(obj[f.key]);
						}
					}
				})
				.catch((e) => {
					showError(e instanceof Error ? e.message : 'Failed to load form data');
				})
				.finally(() => {
					preLoading = false;
				});
		}
	});

	$effect(() => {
		const activeFieldKeys = new Set(fields.map((field) => field.key));

		for (const fieldKey of Object.keys(loadedOptionSourceByField)) {
			if (activeFieldKeys.has(fieldKey)) {
				continue;
			}
			delete loadedOptionSourceByField[fieldKey];
			delete activeOptionRequestByField[fieldKey];
		}

		for (const f of fields) {
			if ((f.field_type === 'select' || f.field_type === 'multi_select') && f.select_source) {
				const sourceKey = selectSourceKey(f);
				if (!sourceKey || loadedOptionSourceByField[f.key] === sourceKey) {
					continue;
				}
				loadedOptionSourceByField[f.key] = sourceKey;
				activeOptionRequestByField[f.key] = sourceKey;
				dynamicOptions = Object.fromEntries(Object.entries(dynamicOptions).filter(([fieldKey]) => fieldKey !== f.key));
				loadingOptions = { ...loadingOptions, [f.key]: true };
				if (f.select_source.type === 'rest_api') {
					loadRestApiOptions(
						f.key,
						sourceKey,
						f.select_source.path,
						f.select_source.value_field,
						f.select_source.label_field
					);
				} else if (f.select_source.type === 'action') {
					loadActionOptions(f.key, sourceKey, f.select_source.action_id);
				}
			} else if (loadedOptionSourceByField[f.key]) {
				delete loadedOptionSourceByField[f.key];
				delete activeOptionRequestByField[f.key];
				dynamicOptions = Object.fromEntries(Object.entries(dynamicOptions).filter(([fieldKey]) => fieldKey !== f.key));
				loadingOptions = Object.fromEntries(Object.entries(loadingOptions).filter(([fieldKey]) => fieldKey !== f.key));
			}
		}
	});

	function selectSourceKey(field: FormField): string | null {
		if (!field.select_source) {
			return null;
		}
		if (field.select_source.type === 'rest_api') {
			return `rest:${field.select_source.path}:${field.select_source.value_field}:${field.select_source.label_field}`;
		}
		return `action:${field.select_source.action_id}`;
	}

	async function loadRestApiOptions(
		fieldKey: string,
		sourceKey: string,
		path: string,
		valueField: string,
		labelField: string
	) {
		try {
			const allItems: unknown[] = [];
			let page = 1;
			let totalPages = 1;

			do {
				const sep = path.includes('?') ? '&' : '?';
				const result = await apiGet<unknown>(`${path}${sep}page=${page}&per_page=1000`);

				if (Array.isArray(result)) {
					allItems.push(...result);
					break;
				}

				const obj = result as Record<string, unknown>;
				const items = (obj.items as unknown[]) ?? [];
				allItems.push(...items);
				totalPages = (obj.total_pages as number) ?? 1;
				page++;
			} while (page <= totalPages);

			dynamicOptions = {
				...dynamicOptions,
				...(activeOptionRequestByField[fieldKey] === sourceKey
					? {
							[fieldKey]: allItems.map((item) => {
								const i = item as Record<string, unknown>;
								return {
									value: String(i[valueField] ?? ''),
									label: String(i[labelField] ?? '')
								};
							})
						}
					: {})
			};
		} catch (e) {
			showError(e instanceof Error ? e.message : `Failed to load options for ${fieldKey}`);
			if (activeOptionRequestByField[fieldKey] === sourceKey) {
				delete loadedOptionSourceByField[fieldKey];
				delete activeOptionRequestByField[fieldKey];
				dynamicOptions = { ...dynamicOptions, [fieldKey]: [] };
				loadingOptions = { ...loadingOptions, [fieldKey]: false };
			}
		} finally {
			if (activeOptionRequestByField[fieldKey] === sourceKey) {
				loadingOptions = { ...loadingOptions, [fieldKey]: false };
			}
		}
	}

	async function loadActionOptions(fieldKey: string, sourceKey: string, actionId: string) {
		try {
			const options = loadSelectOptions ? await loadSelectOptions(actionId) : [];
			if (activeOptionRequestByField[fieldKey] === sourceKey) {
				dynamicOptions = { ...dynamicOptions, [fieldKey]: options };
			}
		} catch (e) {
			showError(e instanceof Error ? e.message : `Failed to load options for ${fieldKey}`);
			if (activeOptionRequestByField[fieldKey] === sourceKey) {
				delete loadedOptionSourceByField[fieldKey];
				delete activeOptionRequestByField[fieldKey];
				dynamicOptions = { ...dynamicOptions, [fieldKey]: [] };
				loadingOptions = { ...loadingOptions, [fieldKey]: false };
			}
		} finally {
			if (activeOptionRequestByField[fieldKey] === sourceKey) {
				loadingOptions = { ...loadingOptions, [fieldKey]: false };
			}
		}
	}

	function resolvedOptions(field: FormField): SelectOption[] {
		if (dynamicOptions[field.key] !== undefined) return dynamicOptions[field.key];
		return field.options ?? [];
	}

	function isFieldVisible(field: FormField): boolean {
		if (!field.visible_when) return true;
		const controlValue = values[field.visible_when.field] ?? '';
		return field.visible_when.values.includes(controlValue);
	}

	async function handleSubmit(e: SubmitEvent) {
		e.preventDefault();
		// Coerce values to the correct types expected by the backend:
		// - multi_select → JSON-encoded string array
		// - toggles → boolean (values map stores "true" / "" as strings)
		// - numbers → JSON numbers
		// - empty text/textarea/password fields → omit entirely (absent = unset)
		// - all other fields → pass through as strings
		const coerced: Record<string, unknown> = {};
		for (const f of fields) {
			if (f.field_type === 'multi_select') {
				coerced[f.key] = JSON.stringify([...(multiSets[f.key] ?? [])]);
			} else {
				const raw = values[f.key];
				if (f.field_type === 'toggle') {
					coerced[f.key] = raw === 'true';
				} else if (f.field_type === 'number' && raw !== '') {
					const parsed = Number(raw);
					coerced[f.key] = Number.isFinite(parsed) ? parsed : raw;
				} else if (raw !== '') {
					coerced[f.key] = raw;
				}
				// Empty optional text fields are intentionally omitted.
			}
		}
		const submitParams = Object.fromEntries(Object.entries(extraParams).filter(([key]) => key !== '_row'));
		await onsubmit({ ...submitParams, ...coerced });
	}
</script>

<form id={formId} onsubmit={handleSubmit} class="space-y-4">
	{#each fields as field (field.key)}
		{#if field.field_type !== 'hidden' && isFieldVisible(field)}
			{#if field.field_type === 'textarea'}
				<label class="label">
					<span>
						{field.label}
						{#if field.required}<span class="text-error-500">*</span>{/if}
					</span>
					<textarea
						id={field.key}
						bind:value={values[field.key]}
						placeholder={field.placeholder}
						required={field.required}
						class="textarea"
						rows="3"
					></textarea>
				</label>
			{:else if field.field_type === 'ssh_private_key'}
				<label class="label">
					<span>
						{field.label}
						{#if field.required}<span class="text-error-500">*</span>{/if}
					</span>
					<textarea
						id={field.key}
						bind:value={values[field.key]}
						placeholder={field.placeholder}
						required={field.required}
						class="textarea font-mono text-xs"
						rows="8"
						spellcheck="false"
						autocomplete="off"
					></textarea>
				</label>
			{:else if field.field_type === 'select'}
				<label class="label">
					<span>
						{field.label}
						{#if field.required}<span class="text-error-500">*</span>{/if}
					</span>
					{#if loadingOptions[field.key]}
						<p class="text-sm text-surface-500">Loading options...</p>
					{:else}
						<select id={field.key} bind:value={values[field.key]} required={field.required} class="select">
							<option value="">Select...</option>
							{#each resolvedOptions(field) as opt (opt.value)}
								<option value={opt.value}>{opt.label}</option>
							{/each}
						</select>
					{/if}
				</label>
			{:else if field.field_type === 'multi_select'}
				<div>
					<span class="label">
						{field.label}
						{#if field.required}<span class="text-error-500">*</span>{/if}
					</span>
					{#if loadingOptions[field.key]}
						<p class="text-sm text-surface-500">Loading options...</p>
					{:else}
						{@const opts = resolvedOptions(field)}
						{#if opts.length === 0}
							<p class="text-sm text-surface-500">No options available.</p>
						{:else}
							<CheckboxList
								items={opts.map((o) => ({ value: o.value, label: o.label }))}
								selected={multiSets[field.key]}
							/>
						{/if}
					{/if}
				</div>
			{:else if field.field_type === 'toggle'}
				<label class="flex items-center gap-2">
					<input
						type="checkbox"
						id={field.key}
						checked={values[field.key] === 'true'}
						onchange={(e) => {
							values[field.key] = String((e.target as HTMLInputElement).checked);
						}}
						class="checkbox"
					/>
					<span class="text-sm">{field.help_text ?? field.label}</span>
				</label>
			{:else}
				<label class="label">
					<span>
						{field.label}
						{#if field.required}<span class="text-error-500">*</span>{/if}
					</span>
					<input
						id={field.key}
						type={field.field_type === 'password' ? 'password' : field.field_type === 'number' ? 'number' : 'text'}
						bind:value={values[field.key]}
						placeholder={field.placeholder}
						required={field.required}
						class="input"
					/>
				</label>
			{/if}

			{#if field.help_text && field.field_type !== 'toggle' && field.field_type !== 'multi_select'}
				<p class="-mt-2 text-xs text-surface-500">{field.help_text}</p>
			{/if}
		{:else}
			<input type="hidden" name={field.key} bind:value={values[field.key]} />
		{/if}
	{/each}

	{#if !hideSubmit}
		<button type="submit" class="btn preset-filled-primary-500" disabled={loading || preLoading}>
			{#if preLoading}
				Loading...
			{:else if loading}
				Processing...
			{:else}
				{submitLabel}
			{/if}
		</button>
	{/if}
</form>
