<script lang="ts" module>
	import type { Snippet } from 'svelte';
	import type { MouseEventHandler } from 'svelte/elements';

	export type UpdateAllState = 'idle' | 'dim';

	export type UpdateAllButtonProps = {
		state: UpdateAllState;
		count?: number;
		onclick: MouseEventHandler<HTMLButtonElement>;
		ariaLabel?: string;
		children?: Snippet;
		class?: string;
	};
</script>

<script lang="ts">
	const BASE =
		'inline-flex items-center gap-1.5 h-[19px] px-2 rounded-card ' +
		'text-[8.5px] font-bold uppercase ' +
		'transition-[background,border-color,color] duration-fast ' +
		'active:opacity-pressed';

	const STATE_CLASSES: Record<UpdateAllState, string> = {
		idle:
			'bg-[rgba(var(--accent-rgb),0.06)] ' +
			'border border-[rgba(var(--accent-rgb),0.20)] ' +
			'text-[var(--accent)] ' +
			'hover:bg-[rgba(var(--accent-rgb),0.18)] ' +
			'hover:border-[rgba(var(--accent-rgb),0.45)] ' +
			'hover:text-[var(--accent-bright)]',
		dim:
			'bg-transparent ' + 'border border-[var(--border-default)] ' + 'text-[var(--text-muted)] ' + 'pointer-events-none'
	};

	let { state, count, onclick, ariaLabel, children, class: className = '' }: UpdateAllButtonProps = $props();

	const computedClass = $derived([BASE, STATE_CLASSES[state], className].filter(Boolean).join(' '));

	const isDim = $derived(state === 'dim');

	function handleKeydown(e: KeyboardEvent) {
		if (isDim && (e.key === 'Enter' || e.key === ' ')) e.preventDefault();
	}
</script>

{#snippet defaultLabel()}↑ Update all{/snippet}

<button
	type="button"
	aria-disabled={isDim || undefined}
	aria-label={ariaLabel}
	class={computedClass}
	onclick={isDim ? undefined : onclick}
	onkeydown={handleKeydown}
>
	{#if children}
		{@render children()}
	{:else}
		{@render defaultLabel()}
	{/if}
	{#if count !== undefined}
		&nbsp;·&nbsp;{count}
	{/if}
</button>
