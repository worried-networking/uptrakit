<script lang="ts">
	import { Select } from '$lib/components/forms';

	export type ProviderOption = {
		id: string;
		label: string;
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

	const fallbackId = $derived(providers.find((provider) => !provider.disabled)?.id ?? '');
	const currentId = $derived(selectedId ?? fallbackId);

	function handleChange(event: Event): void {
		onSelect?.((event.currentTarget as HTMLSelectElement).value);
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
</div>
