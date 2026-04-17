<script lang="ts">
	export type ActionBadgeVariant = 'navigation' | 'bulk-update';
	export type ActionBadgeTone = 'info' | 'accent' | 'danger';

	const toneClasses: Record<ActionBadgeTone, string> = {
		info: 'border-[var(--color-info-border)] bg-[var(--color-info-bg)] text-[var(--color-info)]',
		accent:
			'border-[color:rgb(var(--accent-rgb)/0.28)] bg-[color:rgb(var(--accent-rgb)/0.12)] text-[var(--accent-bright)]',
		danger: 'border-[var(--color-error-border)] bg-[var(--color-error-bg)] text-[var(--color-error)]'
	};

	const variantClasses: Record<ActionBadgeVariant, string> = {
		navigation: 'min-h-[14px]',
		'bulk-update': 'min-h-[16px]'
	};

	let {
		variant = 'navigation',
		tone,
		idleLabel,
		hoverLabel,
		disabled = false,
		onclick
	}: {
		variant?: ActionBadgeVariant;
		tone: ActionBadgeTone;
		idleLabel: string;
		hoverLabel: string;
		disabled?: boolean;
		onclick?: (event: MouseEvent) => void;
	} = $props();

	function handleClick(event: MouseEvent): void {
		if (disabled) {
			event.preventDefault();
			return;
		}
		onclick?.(event);
	}
</script>

<button
	type="button"
	class={`group relative inline-flex min-w-max items-center justify-center rounded-[2px] border px-1.5 text-[7.5px] font-bold uppercase tracking-[0.04em] ${variantClasses[variant]} ${toneClasses[tone]} disabled:cursor-default disabled:opacity-50`}
	data-ui="action-badge"
	data-variant={variant}
	data-tone={tone}
	{disabled}
	onclick={handleClick}
>
	<span class="idle group-hover:invisible">{idleLabel}</span>
	<span aria-hidden="true" class="hov invisible absolute inset-0 flex items-center justify-center group-hover:visible">
		{hoverLabel}
	</span>
</button>
