<script lang="ts">
	import { Select } from '$lib/components/forms';

	export type ProviderOption = {
		id: string;
		label: string;
		description?: string;
		disabled?: boolean;
	};

	let {
		id,
		label = 'Provider',
		providers = [],
		selectedId,
		onSelect
	}: {
		id: string;
		label?: string;
		providers: ProviderOption[];
		selectedId?: string;
		onSelect?: (id: string) => void;
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
		const nextId = (event.currentTarget as HTMLSelectElement).value;
		if (!isControlled) {
			uncontrolledId = nextId;
		}
		onSelect?.(nextId);
	}
</script>

<div class="space-y-2" data-ui="provider-selector">
	<label for={id} class="block space-y-2">
		<span class="text-sm font-medium text-[var(--text-primary)]">{label}</span>
	</label>
	<Select
		{id}
		value={currentId}
		options={providers.map((p) => ({ value: p.id, label: p.label, disabled: p.disabled }))}
		onchange={handleChange}
	/>
	{#if selectedProvider?.description}
		<p class="text-sm text-[var(--text-secondary)]">{selectedProvider.description}</p>
	{/if}
</div>
