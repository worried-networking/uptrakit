<script lang="ts">
	import { tick, untrack } from 'svelte';
	import { Search, X } from 'lucide-svelte';
	import Button from '$lib/components/Button.svelte';
	import { Input } from '$lib/components/forms';

	let {
		id,
		value,
		onchange,
		placeholder = 'Search...',
		debounceMs = 300
	}: {
		id: string;
		value: string;
		onchange: (v: string) => void;
		placeholder?: string;
		debounceMs?: number;
	} = $props();

	let localValue = $state(untrack(() => value));
	let expanded = $state(untrack(() => value !== ''));
	let inputEl: HTMLInputElement | undefined = $state();
	let timer: ReturnType<typeof setTimeout> | undefined;

	$effect(() => {
		clearTimeout(timer);
		localValue = value;
		if (value !== '') expanded = true;
	});

	$effect(() => {
		return () => clearTimeout(timer);
	});

	async function expand() {
		expanded = true;
		await tick();
		inputEl?.focus();
	}

	function clear() {
		clearTimeout(timer);
		localValue = '';
		expanded = false;
		onchange('');
	}

	function handleInput(e: Event) {
		localValue = (e.currentTarget as HTMLInputElement).value;
		clearTimeout(timer);
		timer = setTimeout(() => onchange(localValue), debounceMs);
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') clear();
	}
</script>

{#if !expanded}
	<Button variant="ghost" size="sm" ariaLabel={placeholder} onclick={expand}>
		{#snippet leadingIcon()}<Search size={14} aria-hidden="true" />{/snippet}
	</Button>
{:else}
	<div class="flex w-full items-center gap-1 md:w-auto">
		<Input
			bind:el={inputEl}
			{id}
			type="search"
			{placeholder}
			aria-label={placeholder}
			value={localValue}
			class="w-full md:w-48"
			oninput={handleInput}
			onkeydown={handleKeydown}
		/>
		<Button variant="ghost" size="sm" ariaLabel="Clear search" onclick={clear}>
			{#snippet leadingIcon()}<X size={14} aria-hidden="true" />{/snippet}
		</Button>
	</div>
{/if}
