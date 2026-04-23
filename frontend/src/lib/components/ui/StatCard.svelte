<script lang="ts">
	export type StatCardTone = 'muted' | 'success' | 'info' | 'warning' | 'danger';

	const toneTokens: Record<StatCardTone, string> = {
		muted: '--text-muted',
		success: '--color-success',
		info: '--color-info',
		warning: '--color-warning',
		danger: '--color-danger'
	};

	let {
		href,
		label,
		value,
		valueTone = 'muted',
		subLabel
	}: {
		href: string;
		label: string;
		value: string | number;
		valueTone?: StatCardTone;
		subLabel: string;
	} = $props();
</script>

<a
	{href}
	class="block rounded-card border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-4 transition-[background,border-color,color] duration-fast hover:border-[var(--accent)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]"
	data-ui="stat-card"
	data-tone={valueTone}
>
	<p class="text-badge font-semibold uppercase tracking-pill text-[var(--text-secondary)]">
		{label}
	</p>
	<p class="mt-1 text-sm font-bold" style="color: var({toneTokens[valueTone]})">
		{value}
	</p>
	<p class="mt-1 text-table-body text-[var(--text-secondary)]">
		{subLabel}
	</p>
</a>
