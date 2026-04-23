<script lang="ts" module>
	import type { Snippet } from 'svelte';
	import type { MouseEventHandler } from 'svelte/elements';

	export type UpdateAllState = 'idle' | 'dim';

	export type UpdateAllButtonProps = {
		state: UpdateAllState;
		onclick: MouseEventHandler<HTMLButtonElement>;
		ariaLabel?: string;
		children?: Snippet;
		class?: string;
	};
</script>

<script lang="ts">
	const BASE =
		'group relative inline-flex min-w-max items-center justify-center rounded-badge border px-1.5 min-h-badge ' +
		'text-badge font-bold uppercase tracking-badge ' +
		'transition-[background,border-color,color] duration-fast ' +
		'focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] ' +
		'border-[color:rgb(var(--accent-rgb)/0.28)] bg-[color:rgb(var(--accent-rgb)/0.12)] text-[var(--accent-bright)] ' +
		'hover:bg-[color:rgb(var(--accent-rgb)/0.18)] hover:border-[color:rgb(var(--accent-rgb)/0.45)]';

	const DIM = 'opacity-40 pointer-events-none';

	let { state, onclick, ariaLabel, children, class: className = '' }: UpdateAllButtonProps = $props();

	const isDim = $derived(state === 'dim');
	const computedClass = $derived([BASE, isDim ? DIM : '', className].filter(Boolean).join(' '));

	function handleKeydown(e: KeyboardEvent) {
		if (isDim && (e.key === 'Enter' || e.key === ' ')) e.preventDefault();
	}
</script>

{#snippet defaultLabel()}Update all{/snippet}

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
</button>
