<script lang="ts">
	import type { FieldDef } from '$lib/types';

	let {
		fields,
		onsubmit,
		submitLabel = 'Submit',
		loading = false
	}: {
		fields: FieldDef[];
		onsubmit: (values: Record<string, unknown>) => Promise<void>;
		submitLabel?: string;
		loading?: boolean;
	} = $props();

	let values: Record<string, string> = $state({});

	$effect(() => {
		const initial: Record<string, string> = {};
		for (const f of fields) {
			initial[f.key] = f.default_value ?? '';
		}
		values = initial;
	});

	async function handleSubmit(e: SubmitEvent) {
		e.preventDefault();
		await onsubmit(values);
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
					<select id={field.key} bind:value={values[field.key]} required={field.required} class="select w-full">
						<option value="">Select...</option>
						{#each field.options ?? [] as opt (opt.value)}
							<option value={opt.value}>{opt.label}</option>
						{/each}
					</select>
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
