<script lang="ts">
	import FormFieldReadOnly from '$lib/components/forms/FormFieldReadOnly.svelte';
	import { FormLayout, setFormLayout } from '$lib/components/forms/form-layout-context';

	setFormLayout(FormLayout.Modal);

	let { label = 'Probe', value = 'value' }: { label?: string; value?: string } = $props();
</script>

<FormFieldReadOnly {label} {value} />
