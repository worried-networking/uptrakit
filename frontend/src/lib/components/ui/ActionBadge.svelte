<script lang="ts">
	export type ActionBadgeVariant = 'navigation' | 'bulk-update';
	export type ActionBadgeTone = 'info' | 'accent' | 'danger';

	const toneClasses: Record<ActionBadgeTone, string> = {
		info: 'border-[var(--color-info-border)] bg-[var(--color-info-bg)] text-[var(--color-info)] hover:bg-[color-mix(in_srgb,var(--color-info-bg)_60%,var(--color-info)_40%)] hover:border-[color-mix(in_srgb,var(--color-info-border)_70%,var(--color-info)_30%)]',
		accent:
			'border-[color:rgb(var(--accent-rgb)/0.28)] bg-[color:rgb(var(--accent-rgb)/0.12)] text-[var(--accent-bright)] hover:bg-[color:rgb(var(--accent-rgb)/0.18)] hover:border-[color:rgb(var(--accent-rgb)/0.45)]',
		danger:
			'border-[var(--color-danger-border)] bg-[var(--color-danger-bg)] text-[var(--color-danger)] hover:bg-[color-mix(in_srgb,var(--color-danger-bg)_60%,var(--color-danger)_40%)] hover:border-[color-mix(in_srgb,var(--color-danger-border)_70%,var(--color-danger)_30%)]'
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
	class={`group relative inline-flex min-w-max items-center justify-center rounded-[2px] border px-1.5 text-[7.5px] font-bold uppercase tracking-[0.04em] transition-[background,border-color,color] duration-[120ms] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] ${variantClasses[variant]} ${toneClasses[tone]} disabled:pointer-events-none disabled:cursor-default disabled:opacity-40`}
	data-ui="action-badge"
	data-variant={variant}
	data-tone={tone}
	{disabled}
	onclick={handleClick}
>
	<span class="grid grid-cols-1 items-center justify-items-center">
		<span class="idle col-start-1 row-start-1 group-hover:invisible">{idleLabel}</span>
		<span aria-hidden="true" class="hov invisible col-start-1 row-start-1 group-hover:visible">
			{hoverLabel}
		</span>
	</span>
</button>
