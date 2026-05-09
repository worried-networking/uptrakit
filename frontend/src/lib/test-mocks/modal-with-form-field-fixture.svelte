<script lang="ts">
	import Modal from '$lib/components/Modal.svelte';
	import FormFieldRow from '$lib/components/forms/FormFieldRow.svelte';
	import { createRawSnippet } from 'svelte';

	let { onclose }: { onclose: () => void } = $props();

	const fieldChildren = createRawSnippet(() => ({
		render: () => '<input id="probe" type="text" />'
	}));
</script>

<Modal {onclose} title="Stacking probe">
	<FormFieldRow label="Probe" inputId="probe" children={fieldChildren} />
</Modal>
