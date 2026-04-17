<script lang="ts">
	let { label }: { label: string } = $props();
</script>

<span
	class="inline-flex min-h-[14px] items-center rounded-full border border-[var(--border-default)] bg-[var(--bg-raised)] px-2 text-[7.5px] font-semibold uppercase tracking-[0.08em] text-[var(--text-secondary)]"
	data-ui="pill-badge"
>
	{label}
</span>
