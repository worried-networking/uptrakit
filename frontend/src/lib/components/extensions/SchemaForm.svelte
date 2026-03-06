<script lang="ts">
	import type { FieldDef, SelectOption } from '$lib/types';
	import { apiGet } from '$lib/api';
	import { showError } from '$lib/notifications.svelte';

	let {
		fields,
		onsubmit,
		submitLabel = 'Submit',
		loading = false,
		extensionId: _extensionId,
		serviceId: _serviceId,
		extraParams = {}
	}: {
		fields: FieldDef[];
		onsubmit: (values: Record<string, unknown>) => Promise<void>;
		submitLabel?: string;
		loading?: boolean;
		extensionId?: string;
		serviceId?: string;
		extraParams?: Record<string, unknown>;
	} = $props();

	let values: Record<string, string> = $state({});
	let dynamicOptions: Record<string, SelectOption[]> = $state({});
	let loadingOptions: Record<string, boolean> = $state({});

	$effect(() => {
		const initial: Record<string, string> = {};
		for (const f of fields) {
			initial[f.key] = f.default_value ?? '';
		}
		values = initial;
	});

	$effect(() => {
		for (const f of fields) {
			if (f.field_type === 'select' && f.select_source?.type === 'rest_api') {
				loadSelectSourceOptions(f.key, f.select_source.path, f.select_source.value_field, f.select_source.label_field);
			}
		}
	});

	async function loadSelectSourceOptions(fieldKey: string, path: string, valueField: string, labelField: string) {
		loadingOptions = { ...loadingOptions, [fieldKey]: true };
		try {
			const result = await apiGet<unknown>(path);
			const arr: unknown[] = Array.isArray(result)
				? result
				: (((result as Record<string, unknown>)?.items as unknown[]) ?? []);
			dynamicOptions = {
				...dynamicOptions,
				[fieldKey]: arr.map((item) => {
					const i = item as Record<string, unknown>;
					return {
						value: String(i[valueField] ?? ''),
						label: String(i[labelField] ?? '')
					};
				})
			};
		} catch (e) {
			showError(e instanceof Error ? e.message : `Failed to load options for ${fieldKey}`);
			dynamicOptions = { ...dynamicOptions, [fieldKey]: [] };
		} finally {
			loadingOptions = { ...loadingOptions, [fieldKey]: false };
		}
	}

	function resolvedOptions(field: FieldDef): SelectOption[] {
		if (dynamicOptions[field.key] !== undefined) return dynamicOptions[field.key];
		return field.options ?? [];
	}

	async function handleSubmit(e: SubmitEvent) {
		e.preventDefault();
		await onsubmit({ ...extraParams, ...values });
	}
</script>

<form onsubmit={handleSubmit} class="space-y-4">
	{#each fields as field (field.key)}
		{#if field.field_type !== 'hidden'}
			<div>
				<label for={field.key} class="mb-1 block text-sm font-medium">
					{field.label}
					{#if field.required}<span class="text-error-500">*</span>{/if}
				</label>

				{#if field.field_type === 'textarea'}
					<textarea
						id={field.key}
						bind:value={values[field.key]}
						placeholder={field.placeholder}
						required={field.required}
						class="input w-full"
						rows="3"
					></textarea>
				{:else if field.field_type === 'select'}
					{#if loadingOptions[field.key]}
						<p class="text-sm text-surface-500">Loading options...</p>
					{:else}
						<select id={field.key} bind:value={values[field.key]} required={field.required} class="select w-full">
							<option value="">Select...</option>
							{#each resolvedOptions(field) as opt (opt.value)}
								<option value={opt.value}>{opt.label}</option>
							{/each}
						</select>
					{/if}
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
						<span class="text-sm">{field.help_text ?? ''}</span>
					</label>
				{:else}
					<input
						id={field.key}
						type={field.field_type === 'password' ? 'password' : field.field_type === 'number' ? 'number' : 'text'}
						bind:value={values[field.key]}
						placeholder={field.placeholder}
						required={field.required}
						class="input w-full"
					/>
				{/if}

				{#if field.help_text && field.field_type !== 'toggle'}
					<p class="mt-1 text-xs text-surface-500">{field.help_text}</p>
				{/if}
			</div>
		{:else}
			<input type="hidden" name={field.key} bind:value={values[field.key]} />
		{/if}
	{/each}

	<button type="submit" class="btn preset-filled-primary-500" disabled={loading}>
		{loading ? 'Processing...' : submitLabel}
	</button>
</form>
