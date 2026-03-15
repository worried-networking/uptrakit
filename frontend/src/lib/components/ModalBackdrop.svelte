<script lang="ts">
	import type { Snippet } from 'svelte';

	let { onclose, children }: { onclose: () => void; children: Snippet } = $props();

	let backdrop: HTMLDivElement;
	let previouslyFocused: Element | null = null;

	const FOCUSABLE =
		'button:not([disabled]), [href], input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

	$effect(() => {
		previouslyFocused = document.activeElement;

		// Focus the first focusable element inside the modal content
		const first = backdrop.querySelector<HTMLElement>(FOCUSABLE);
		if (first) {
			first.focus();
		}

		return () => {
			if (previouslyFocused instanceof HTMLElement) {
				previouslyFocused.focus();
			}
		};
	});

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			onclose();
			return;
		}

		if (e.key !== 'Tab') return;

		const focusable = [...backdrop.querySelectorAll<HTMLElement>(FOCUSABLE)];
		if (focusable.length === 0) return;

		const first = focusable[0];
		const last = focusable[focusable.length - 1];

		if (e.shiftKey && document.activeElement === first) {
			e.preventDefault();
			last.focus();
		} else if (!e.shiftKey && document.activeElement === last) {
			e.preventDefault();
			first.focus();
		}
	}
</script>

<div
	class="fixed inset-0 z-[70] overflow-y-auto bg-black/50"
	role="presentation"
	bind:this={backdrop}
	onkeydown={handleKeydown}
>
	<div
		class="flex min-h-full items-center justify-center p-4"
		role="presentation"
		onclick={(e) => {
			if (e.target === e.currentTarget) onclose();
		}}
	>
		{@render children()}
	</div>
</div>
