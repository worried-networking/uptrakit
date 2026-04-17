<script lang="ts">
	export type StatusBadgeTone = 'neutral' | 'info' | 'success' | 'warning' | 'danger';

	const toneClasses: Record<StatusBadgeTone, string> = {
		neutral: 'border-[var(--border-default)] bg-[var(--bg-raised)] text-[var(--text-secondary)]',
		info: 'border-[var(--color-info-border)] bg-[var(--color-info-bg)] text-[var(--color-info)]',
		success: 'border-[var(--color-success-border)] bg-[var(--color-success-bg)] text-[var(--color-success)]',
		warning: 'border-[var(--color-warning-border)] bg-[var(--color-warning-bg)] text-[var(--color-warning)]',
		danger: 'border-[var(--color-error-border)] bg-[var(--color-error-bg)] text-[var(--color-error)]'
	};

	let {
		tone = 'neutral',
		label
	}: {
		tone?: StatusBadgeTone;
		label: string;
	} = $props();
</script>

<span
	class={`inline-flex min-h-[14px] items-center justify-center rounded-[2px] border px-1.5 text-[7.5px] font-bold uppercase tracking-[0.04em] ${toneClasses[tone]}`}
	data-ui="status-badge"
	data-tone={tone}
>
	{label}
</span>
