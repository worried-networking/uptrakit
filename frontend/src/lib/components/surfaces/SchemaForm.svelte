<script lang="ts">
	import type { FormField, SelectOption } from '$lib/types';
	import { apiGet } from '$lib/api';
	import { showError } from '$lib/notifications.svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import CheckboxList from '$lib/components/CheckboxList.svelte';
	import { FormFieldRow } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';
	import Input from '$lib/components/Input.svelte';
	import Checkbox from '$lib/components/Checkbox.svelte';
	import Textarea from '$lib/components/Textarea.svelte';

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
	let fieldErrors: Record<string, string> = $state({});
	let dynamicOptions: Record<string, SelectOption[]> = $state({});
	let loadingOptions: Record<string, boolean> = $state({});

	// Non-reactive loader bookkeeping keyed by field name. This is intentionally
	// separate from render state so option loading can be invalidated when fields
	// are reused without creating effect update loops.
	const loadedOptionSourceByField: Record<string, string> = {};
	const activeOptionRequestByField: Record<string, string> = {};

	const warnedFieldTypes: Record<string, true> = {};

	function fieldValue(key: string): string {
		return values[key] ?? '';
	}

	function warnUnknownFieldType(fieldType: string): 'text' | 'password' | 'number' {
		if (fieldType === 'password') return 'password';
		if (fieldType === 'number') return 'number';
		if (!['text', 'select', 'multi_select', 'toggle', 'hidden', 'textarea', 'ssh_private_key'].includes(fieldType)) {
			if (!(fieldType in warnedFieldTypes)) {
				warnedFieldTypes[fieldType] = true;
				console.warn(`[SchemaForm] Unknown field_type "${fieldType}" — rendering as text input`);
			}
		}
		return 'text';
	}

	let preLoading: boolean = $state(false);

	function parseMultiSelectValues(value: unknown): string[] {
		if (Array.isArray(value)) {
			return value.map((entry) => String(entry));
		}
		if (value instanceof Set) {
			return [...value].map((entry) => String(entry));
		}
		if (typeof value === 'string') {
			const trimmed = value.trim();
			if (trimmed === '') {
				return [];
			}
			try {
				const parsed = JSON.parse(trimmed);
				if (Array.isArray(parsed)) {
					return parsed.map((entry) => String(entry));
				}
			} catch {
				// Plain string values are treated as a single selected option.
			}
			return [trimmed];
		}
		if (typeof value === 'number' || typeof value === 'boolean') {
			return [String(value)];
		}
		return [];
	}

	function clearFieldError(fieldKey: string) {
		if (!(fieldKey in fieldErrors)) {
			return;
		}
		fieldErrors = Object.fromEntries(Object.entries(fieldErrors).filter(([key]) => key !== fieldKey));
	}

	function requiredFieldMessage(field: FormField): string {
		return `${field.label} is required.`;
	}

	function validateField(field: FormField): string | null {
		if (!field.required || field.field_type === 'hidden' || !isFieldVisible(field)) {
			return null;
		}
		if (field.field_type === 'multi_select') {
			return (multiSets[field.key]?.size ?? 0) > 0 ? null : requiredFieldMessage(field);
		}
		if (field.field_type === 'toggle') {
			return values[field.key] === 'true' ? null : requiredFieldMessage(field);
		}
		const raw = values[field.key];
		if (raw == null) {
			return requiredFieldMessage(field);
		}
		return String(raw).trim() !== '' ? null : requiredFieldMessage(field);
	}

	$effect(() => {
		const initial: Record<string, string> = {};
		const initialMulti: Record<string, SvelteSet<string>> = {};
		const rowData = extraParams._row as Record<string, unknown> | undefined;
		for (const f of fields) {
			if (f.field_type === 'multi_select') {
				const rowValue = rowData?.[f.key];
				const fallbackValue = rowValue == null ? f.default_value : rowValue;
				initialMulti[f.key] = new SvelteSet<string>(parseMultiSelectValues(fallbackValue));
			} else if (rowData && rowData[f.key] != null) {
				initial[f.key] = String(rowData[f.key]);
			} else {
				initial[f.key] = f.default_value ?? '';
			}
		}
		values = initial;
		multiSets = initialMulti;
		fieldErrors = {};

		const loadFormValues = loadInitialValues;

		if (loadFormValues) {
			preLoading = true;
			loadFormValues()
				.then((obj) => {
					const loadedValues = { ...values };
					const loadedMultiSets = { ...multiSets };
					for (const f of fields) {
						if (obj[f.key] != null) {
							if (f.field_type === 'multi_select') {
								loadedMultiSets[f.key] = new SvelteSet<string>(parseMultiSelectValues(obj[f.key]));
							} else {
								loadedValues[f.key] = String(obj[f.key]);
							}
						}
					}
					values = loadedValues;
					multiSets = loadedMultiSets;
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
		const nextErrors: Record<string, string> = {};
		for (const field of fields) {
			const fieldError = validateField(field);
			if (fieldError) {
				nextErrors[field.key] = fieldError;
			}
		}
		fieldErrors = nextErrors;
		if (Object.keys(nextErrors).length > 0) {
			return;
		}
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

<form id={formId} onsubmit={handleSubmit} class="space-y-4" novalidate>
	{#each fields as field (field.key)}
		{#if field.field_type !== 'hidden' && isFieldVisible(field)}
			{#if field.field_type === 'textarea'}
				<FormFieldRow
					label={field.label}
					inputId={field.key}
					required={field.required}
					hint={field.help_text}
					error={fieldErrors[field.key]}
				>
					<Textarea
						id={field.key}
						value={fieldValue(field.key)}
						placeholder={field.placeholder}
						required={field.required}
						rows={3}
						error={fieldErrors[field.key]}
						oninput={(e) => {
							values[field.key] = (e.target as HTMLTextAreaElement).value;
							clearFieldError(field.key);
						}}
					/>
				</FormFieldRow>
			{:else if field.field_type === 'ssh_private_key'}
				<FormFieldRow
					label={field.label}
					inputId={field.key}
					required={field.required}
					hint={field.help_text}
					error={fieldErrors[field.key]}
				>
					<Textarea
						id={field.key}
						value={fieldValue(field.key)}
						placeholder={field.placeholder}
						required={field.required}
						rows={8}
						variant="mono"
						error={fieldErrors[field.key]}
						oninput={(e) => {
							values[field.key] = (e.target as HTMLTextAreaElement).value;
							clearFieldError(field.key);
						}}
					/>
				</FormFieldRow>
			{:else if field.field_type === 'select'}
				<FormFieldRow
					label={field.label}
					inputId={field.key}
					required={field.required}
					hint={field.help_text}
					error={fieldErrors[field.key]}
				>
					{#if loadingOptions[field.key]}
						<p class="text-sm text-surface-500">Loading options...</p>
					{:else}
						<select
							id={field.key}
							bind:value={values[field.key]}
							required={field.required}
							class="select"
							aria-invalid={fieldErrors[field.key] ? 'true' : undefined}
							onchange={() => clearFieldError(field.key)}
						>
							<option value="">Select...</option>
							{#each resolvedOptions(field) as opt (opt.value)}
								<option value={opt.value}>{opt.label}</option>
							{/each}
						</select>
					{/if}
				</FormFieldRow>
			{:else if field.field_type === 'multi_select'}
				<FormFieldRow
					label={field.label}
					required={field.required}
					hint={field.help_text}
					error={fieldErrors[field.key]}
				>
					{#if loadingOptions[field.key]}
						<p class="text-sm text-surface-500">Loading options...</p>
					{:else}
						{@const opts = resolvedOptions(field)}
						{#if opts.length === 0}
							<p class="text-sm text-surface-500">No options available.</p>
						{:else}
							<div onchange={() => clearFieldError(field.key)}>
								<CheckboxList
									items={opts.map((o) => ({ value: o.value, label: o.label }))}
									selected={multiSets[field.key] ?? new SvelteSet<string>()}
								/>
							</div>
						{/if}
					{/if}
				</FormFieldRow>
			{:else if field.field_type === 'toggle'}
				<FormFieldRow
					label={field.label}
					inputId={field.key}
					required={field.required}
					hint={field.help_text}
					error={fieldErrors[field.key]}
				>
					<Checkbox
						id={field.key}
						checked={values[field.key] === 'true'}
						disabled={loading}
						onchange={(e) => {
							values[field.key] = (e.target as HTMLInputElement).checked ? 'true' : 'false';
							clearFieldError(field.key);
						}}
					/>
				</FormFieldRow>
			{:else}
				<FormFieldRow
					label={field.label}
					inputId={field.key}
					required={field.required}
					hint={field.help_text}
					error={fieldErrors[field.key]}
				>
					<Input
						id={field.key}
						type={warnUnknownFieldType(field.field_type)}
						value={fieldValue(field.key)}
						placeholder={field.placeholder}
						required={field.required}
						error={fieldErrors[field.key]}
						oninput={(e) => {
							values[field.key] = (e.target as HTMLInputElement).value;
							clearFieldError(field.key);
						}}
					/>
				</FormFieldRow>
			{/if}
		{:else}
			<input type="hidden" name={field.key} bind:value={values[field.key]} />
		{/if}
	{/each}

	{#if !hideSubmit}
		<Button type="submit" variant="primary" loading={loading || preLoading}>
			{submitLabel}
		</Button>
	{/if}
</form>
