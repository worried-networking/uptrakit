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
	class="block rounded-[3px] border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-4 transition-[background,border-color,color] duration-[120ms] hover:border-[var(--accent)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]"
	data-ui="stat-card"
	data-tone={valueTone}
>
	<p class="text-[7.5px] font-semibold uppercase tracking-[0.08em] text-[var(--text-secondary)]">
		{label}
	</p>
	<p class="mt-1 text-[14px] font-bold" style="color: var({toneTokens[valueTone]})">
		{value}
	</p>
	<p class="mt-1 text-[10px] text-[var(--text-secondary)]">
		{subLabel}
	</p>
</a>
