<script lang="ts">
	import PillBadge from './ui/PillBadge.svelte';

	let { name, color }: { name: string; color: string } = $props();
</script>

<span data-color={color}>
	<PillBadge label={name} />
</span>
