<script lang="ts">
	export type ProviderOption = {
		id: string;
		label: string;
		description?: string;
		disabled?: boolean;
	};

	let {
		label = 'Provider',
		providers = [],
		selectedId,
		onSelect,
		emptyMessage = 'No options available.'
	}: {
		label?: string;
		providers: ProviderOption[];
		selectedId?: string;
		onSelect?: (id: string) => void;
		emptyMessage?: string;
	} = $props();

	let uncontrolledId = $state('');

	const fallbackId = $derived(providers.find((provider) => !provider.disabled)?.id ?? '');
	const isControlled = $derived(selectedId !== undefined);
	const currentId = $derived(isControlled ? (selectedId ?? fallbackId) : uncontrolledId || fallbackId);
	const selectedProvider = $derived(providers.find((provider) => provider.id === currentId));

	$effect(() => {
		if (isControlled) {
			return;
		}
		if (!providers.some((provider) => provider.id === uncontrolledId && !provider.disabled)) {
			uncontrolledId = fallbackId;
		}
	});

	function handleChange(event: Event): void {
		const select = event.currentTarget as HTMLSelectElement;
		const nextId = select.value;
		if (!isControlled) {
			uncontrolledId = nextId;
		}
		onSelect?.(nextId);
		if (isControlled) {
			select.value = currentId;
		}
	}
</script>

<div class="space-y-2" data-ui="provider-selector">
	<label class="space-y-2">
		<span class="text-sm font-medium text-[var(--text-primary)]">{label}</span>
		<select
			class="select w-full rounded-card border-[var(--border-default)] bg-[var(--bg-surface)] text-[var(--text-primary)]"
			value={currentId}
			disabled={providers.length === 0}
			onchange={handleChange}
		>
			{#if providers.length === 0}
				<option value="">{emptyMessage}</option>
			{:else}
				{#each providers as provider (provider.id)}
					<option value={provider.id} disabled={provider.disabled}>{provider.label}</option>
				{/each}
			{/if}
		</select>
	</label>

	{#if selectedProvider?.description}
		<p class="text-sm text-[var(--text-secondary)]">{selectedProvider.description}</p>
	{/if}
</div>
