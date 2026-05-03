<script lang="ts">
	import ModalBackdrop from './ModalBackdrop.svelte';
	import type { Snippet } from 'svelte';
	import { FormLayout, setFormLayout } from '$lib/components/forms/form-layout-context';

	setFormLayout(FormLayout.Modal);

	let {
		onclose,
		title,
		subtitle,
		maxWidth = 'max-w-2xl',
		children,
		footer
	}: {
		onclose: () => void;
		title?: string;
		subtitle?: Snippet;
		maxWidth?: string;
		children: Snippet;
		footer?: Snippet;
	} = $props();
</script>

<ModalBackdrop {onclose}>
	<div
		class="bg-[var(--bg-surface)] z-[910] flex w-full max-h-[calc(100vh-4rem)] flex-col overflow-hidden border border-[var(--border-subtle)] rounded-panel {maxWidth} shadow-xl"
		data-ui="modal-shell"
		role="dialog"
		aria-modal="true"
		aria-labelledby={title ? 'modal-title' : undefined}
	>
		{#if title}
			<div class="flex-shrink-0 px-6 pt-6 pb-2">
				<h3 class="text-subsection-title font-bold text-[var(--text-primary)]" id="modal-title">{title}</h3>
				{#if subtitle}
					<p class="mt-1 text-sm text-[var(--text-muted)]">{@render subtitle()}</p>
				{/if}
			</div>
		{/if}
		<div
			class="min-h-0 flex-1 overflow-y-auto px-6 {title ? (subtitle ? 'pt-1' : 'pt-2') : 'pt-6'} {footer
				? 'pb-2'
				: 'pb-6'}"
		>
			{@render children()}
		</div>
		{#if footer}
			<div class="flex flex-shrink-0 justify-end gap-2 px-6 pb-6 pt-2">
				{@render footer()}
			</div>
		{/if}
	</div>
</ModalBackdrop>
