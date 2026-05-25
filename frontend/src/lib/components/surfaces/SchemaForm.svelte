<script lang="ts">
	import type { FormField, SelectOption } from '$lib/types';
	import { apiGet } from '$lib/api';
	import { showError } from '$lib/notifications.svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import { CheckboxList, FormFieldRow, Input, Checkbox, Textarea, Select } from '$lib/components/forms';
	import Button from '$lib/components/Button.svelte';
	import { createFormDraft } from '$lib/forms/draft.svelte';

	type FieldRecord = Record<string, unknown>;

	let {
		fields,
		onsubmit,
		submitLabel = 'Save',
		loading = false,
		formId = undefined,
		hideSubmit = false,
		extraParams = {},
		loadInitialValues,
		loadSelectOptions
	}: {
		fields: FormField[];
		onsubmit: (values: Record<string, unknown>) => Promise<unknown>;
		submitLabel?: string;
		loading?: boolean;
		formId?: string;
		hideSubmit?: boolean;
		extraParams?: Record<string, unknown>;
		loadInitialValues?: () => Promise<Record<string, unknown>>;
		loadSelectOptions?: (actionId: string) => Promise<SelectOption[]>;
	} = $props();

	const form = createFormDraft<FieldRecord>({});
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
		return (form.draft[key] as string) ?? '';
	}

	function setToDraftString(set: SvelteSet<string>): string {
		return [...set].sort().join('\0');
	}

	function draftStringToSet(s: string): SvelteSet<string> {
		return new SvelteSet(s ? s.split('\0') : []);
	}

	function normalizeForDraft(raw: Record<string, unknown>): FieldRecord {
		const result: FieldRecord = {};
		for (const f of fields) {
			const v = raw[f.key];
			result[f.key] = Array.isArray(v)
				? [...(v as string[])].sort().join('\0')
				: v === null || v === undefined
					? ''
					: String(v);
		}
		return result;
	}

	function handleDiscard() {
		form.discard();
		for (const f of fields) {
			if (f.field_type === 'multi_select') {
				multiSets[f.key] = draftStringToSet((form.draft[f.key] as string) ?? '');
			}
		}
	}

	const draftMode = $derived(loadInitialValues !== undefined);
	const isValid = $derived(fields.every((f) => validateField(f) === null));

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
			return fieldValue(field.key) === 'true' ? null : requiredFieldMessage(field);
		}
		const raw = fieldValue(field.key);
		if (raw == null) {
			return requiredFieldMessage(field);
		}
		return String(raw).trim() !== '' ? null : requiredFieldMessage(field);
	}

	$effect(() => {
		const initial: FieldRecord = {};
		const initialMulti: Record<string, SvelteSet<string>> = {};
		const rowData = extraParams._row as Record<string, unknown> | undefined;
		for (const f of fields) {
			if (f.field_type === 'multi_select') {
				const rowValue = rowData?.[f.key];
				const fallbackValue = rowValue == null ? f.default_value : rowValue;
				const multiValues = parseMultiSelectValues(fallbackValue);
				initialMulti[f.key] = new SvelteSet<string>(multiValues);
				initial[f.key] = [...multiValues].sort().join('\0');
			} else if (rowData && rowData[f.key] != null) {
				initial[f.key] = String(rowData[f.key]);
			} else {
				initial[f.key] = f.default_value ?? '';
			}
		}
		form.load(initial);
		multiSets = initialMulti;
		fieldErrors = {};

		const loadFormValues = loadInitialValues;

		if (loadFormValues) {
			preLoading = true;
			loadFormValues()
				.then((obj) => {
					const normalized = normalizeForDraft(obj);
					// Preserve fields not returned by server from current draft baseline
					const merged: FieldRecord = { ...form.draft };
					for (const key of Object.keys(normalized)) {
						merged[key] = normalized[key];
					}
					form.load(merged);
					const nextMulti: Record<string, SvelteSet<string>> = {};
					for (const f of fields) {
						if (f.field_type === 'multi_select') {
							nextMulti[f.key] = draftStringToSet((form.draft[f.key] as string) ?? '');
						}
					}
					multiSets = nextMulti;
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
		const controlValue = fieldValue(field.visible_when.field);
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
		// - toggles → boolean (draft map stores "true" / "" as strings)
		// - numbers → JSON numbers
		// - empty text/textarea/password fields → omit entirely (absent = unset)
		// - all other fields → pass through as strings
		const coerced: Record<string, unknown> = {};
		for (const f of fields) {
			if (f.field_type === 'multi_select') {
				coerced[f.key] = JSON.stringify([...(multiSets[f.key] ?? [])]);
			} else {
				const raw = fieldValue(f.key);
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
		const payload = { ...submitParams, ...coerced };
		const result = await onsubmit(payload);
		if (draftMode) {
			if (result !== null && typeof result === 'object' && !Array.isArray(result)) {
				form.commit(normalizeForDraft(result as Record<string, unknown>));
			} else {
				const reloaded = await loadInitialValues!();
				form.load(normalizeForDraft(reloaded));
			}
			const nextMulti: Record<string, SvelteSet<string>> = {};
			for (const f of fields) {
				if (f.field_type === 'multi_select') {
					nextMulti[f.key] = draftStringToSet((form.draft[f.key] as string) ?? '');
				}
			}
			multiSets = nextMulti;
		}
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
					dirty={form.isFieldDirty(field.key)}
				>
					<Textarea
						id={field.key}
						value={fieldValue(field.key)}
						placeholder={field.placeholder}
						required={field.required}
						rows={3}
						error={fieldErrors[field.key]}
						oninput={(e) => {
							form.update(field.key, (e.target as HTMLTextAreaElement).value);
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
					dirty={form.isFieldDirty(field.key)}
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
							form.update(field.key, (e.target as HTMLTextAreaElement).value);
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
					dirty={form.isFieldDirty(field.key)}
				>
					{#if loadingOptions[field.key]}
						<p class="text-sm text-[var(--text-muted)]">Loading options...</p>
					{:else}
						<Select
							id={field.key}
							value={fieldValue(field.key)}
							options={resolvedOptions(field)}
							placeholder="Select..."
							required={field.required}
							error={fieldErrors[field.key]}
							onchange={(e) => {
								form.update(field.key, (e.target as HTMLSelectElement).value);
								clearFieldError(field.key);
							}}
						/>
					{/if}
				</FormFieldRow>
			{:else if field.field_type === 'multi_select'}
				<FormFieldRow
					label={field.label}
					required={field.required}
					hint={field.help_text}
					error={fieldErrors[field.key]}
					dirty={form.isFieldDirty(field.key)}
				>
					{#if loadingOptions[field.key]}
						<p class="text-sm text-[var(--text-muted)]">Loading options...</p>
					{:else}
						{@const opts = resolvedOptions(field)}
						{#if opts.length === 0}
							<p class="text-sm text-[var(--text-muted)]">No options available.</p>
						{:else}
							<div
								onchange={() => {
									form.update(field.key, setToDraftString(multiSets[field.key] ?? new SvelteSet<string>()));
									clearFieldError(field.key);
								}}
							>
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
					dirty={form.isFieldDirty(field.key)}
				>
					<Checkbox
						id={field.key}
						checked={fieldValue(field.key) === 'true'}
						disabled={loading}
						onchange={(e) => {
							form.update(field.key, (e.target as HTMLInputElement).checked ? 'true' : 'false');
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
					dirty={form.isFieldDirty(field.key)}
				>
					<Input
						id={field.key}
						type={warnUnknownFieldType(field.field_type)}
						value={fieldValue(field.key)}
						placeholder={field.placeholder}
						required={field.required}
						error={fieldErrors[field.key]}
						oninput={(e) => {
							form.update(field.key, (e.target as HTMLInputElement).value);
							clearFieldError(field.key);
						}}
					/>
				</FormFieldRow>
			{/if}
		{:else}
			<input type="hidden" name={field.key} value={fieldValue(field.key)} />
		{/if}
	{/each}

	{#if !hideSubmit}
		<div class="flex gap-2 justify-end">
			{#if draftMode && form.isDirty}
				<Button type="button" variant="ghost" disabled={loading || preLoading} onclick={handleDiscard}>Discard</Button>
			{/if}
			<Button
				type="submit"
				variant="primary"
				loading={loading || preLoading}
				disabled={!isValid || loading || preLoading || (draftMode && !form.isDirty)}
			>
				{submitLabel}
			</Button>
		</div>
	{/if}
</form>
