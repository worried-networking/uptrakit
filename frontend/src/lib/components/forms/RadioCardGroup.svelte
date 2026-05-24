<script lang="ts" module>
	export interface RadioCardOption<T extends string = string> {
		value: T;
		label: string;
		tooltip?: string;
	}
</script>

<script lang="ts" generics="T extends string">
	import { Tooltip } from '$lib/components/ui';

	let {
		name,
		value,
		options,
		onchange,
		disabled = false
	}: {
		name: string;
		value: T;
		options: RadioCardOption<T>[];
		onchange?: (value: T) => void;
		disabled?: boolean;
	} = $props();

	function select(v: T) {
		if (!disabled) onchange?.(v);
	}

	function handleKeydown(e: KeyboardEvent, idx: number) {
		if (disabled) return;
		if (e.key === 'Enter' || e.key === ' ') {
			e.preventDefault();
			select(options[idx].value);
			return;
		}
		let next = idx;
		if (e.key === 'ArrowRight' || e.key === 'ArrowDown') {
			e.preventDefault();
			next = (idx + 1) % options.length;
		} else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') {
			e.preventDefault();
			next = (idx - 1 + options.length) % options.length;
		}
		if (next !== idx) {
			onchange?.(options[next].value);
		}
	}
</script>

<div
	role="radiogroup"
	aria-label={name}
	style="display: grid; grid-template-columns: repeat({options.length}, 1fr); gap: 0.5rem;"
>
	{#each options as option, i (option.value)}
		{@const selected = option.value === value}
		<div
			role="radio"
			tabindex={disabled ? -1 : 0}
			aria-checked={selected}
			aria-disabled={disabled}
			aria-label={option.label}
			onclick={() => select(option.value)}
			onkeydown={(e) => handleKeydown(e, i)}
			class="
				rounded px-3 py-3 text-left transition-[background,border-color,color]
				duration-fast cursor-pointer
				{selected
				? 'border-2 border-[rgba(var(--accent-rgb,6,182,212),0.6)] bg-[rgba(var(--accent-rgb,6,182,212),0.07)] text-[var(--accent-bright)]'
				: 'border border-[var(--border-subtle)] bg-transparent text-[var(--text-secondary)]'}
				{disabled ? 'cursor-not-allowed opacity-40' : ''}
			"
		>
			<div class="flex items-center gap-1">
				<span class="text-sm font-semibold">{option.label}</span>
				{#if option.tooltip}
					<Tooltip content={option.tooltip} />
				{/if}
			</div>
		</div>
	{/each}
</div>
