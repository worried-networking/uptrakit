<script lang="ts" module>
	let _nextId = 0;
</script>

<script lang="ts">
	import { Info } from 'lucide-svelte';
	import { portal } from '$lib/actions/portal';

	let {
		content,
		id
	}: {
		content: string;
		id?: string;
	} = $props();

	_nextId += 1;
	const tooltipId = $derived(id ?? `tooltip-${_nextId}`);

	let triggerEl: HTMLButtonElement | undefined = $state(undefined);
	let tooltipEl: HTMLDivElement | undefined = $state(undefined);
	let visible = $state(false);
	let placement: 'top' | 'bottom' = $state('top');
	let tooltipTop = $state(0);
	let tooltipLeft = $state(0);
	let arrowLeft = $state(0);

	let hideTimeout: ReturnType<typeof setTimeout> | undefined;

	function show() {
		clearTimeout(hideTimeout);
		visible = true;
	}

	function scheduleHide() {
		hideTimeout = setTimeout(() => {
			visible = false;
		}, 100);
	}

	$effect(() => {
		return () => clearTimeout(hideTimeout);
	});
</script>

{#if content}
	<button
		type="button"
		bind:this={triggerEl}
		aria-label="More information"
		aria-describedby={tooltipId}
		class="inline-flex cursor-default items-center text-[var(--text-muted)] hover:text-[var(--text-secondary)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]"
		onclick={(e) => e.stopPropagation()}
		onmouseenter={show}
		onmouseleave={scheduleHide}
		onfocus={show}
		onblur={scheduleHide}
		onkeydown={(e) => {
			if (e.key === 'Escape' && visible) {
				visible = false;
			}
		}}
	>
		<Info size={14} aria-hidden="true" />
	</button>

	<div
		bind:this={tooltipEl}
		use:portal
		id={tooltipId}
		role="tooltip"
		data-ui="tooltip"
		class="fixed z-[100] max-w-[220px] rounded-panel border border-[var(--border-default)] bg-[var(--bg-raised)] px-3 py-2 text-xs whitespace-pre-line text-[var(--text-primary)] transition-[opacity] duration-fast"
		class:invisible={!visible}
		class:opacity-0={!visible}
		style="top: {tooltipTop}px; left: {tooltipLeft}px;"
		onmouseenter={show}
		onmouseleave={scheduleHide}
	>
		{content}
		<div
			class="absolute h-1.5 w-1.5 rotate-45 border-[var(--border-default)] bg-[var(--bg-raised)]"
			class:border-b={placement === 'top'}
			class:border-r={placement === 'top'}
			class:border-t={placement === 'bottom'}
			class:border-l={placement === 'bottom'}
			style="left: {arrowLeft - 3}px; {placement === 'top' ? 'bottom: -3px' : 'top: -3px'}"
		></div>
	</div>
{/if}
