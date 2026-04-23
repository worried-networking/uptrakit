<script lang="ts">
	let { label }: { label: string } = $props();
</script>

<span
	class="inline-flex min-h-badge items-center rounded-full border border-[var(--border-default)] bg-[var(--bg-raised)] px-2 text-badge font-bold uppercase tracking-pill text-[var(--text-secondary)]"
	data-ui="pill-badge"
>
	{label}
</span>
