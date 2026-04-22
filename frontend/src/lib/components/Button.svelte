<script lang="ts" module>
	import type { Snippet } from 'svelte';
	import type { MouseEventHandler } from 'svelte/elements';

	export type ButtonVariant = 'primary' | 'ghost' | 'danger' | 'secondary';
	export type ButtonSize = 'sm' | 'md';

	type CommonProps = {
		variant: ButtonVariant;
		size?: ButtonSize;
		disabled?: boolean;
		loading?: boolean;
		leadingIcon?: Snippet;
		trailingIcon?: Snippet;
		ariaLabel?: string;
		children: Snippet;
		class?: string;
	};

	export type ButtonProps =
		| (CommonProps & {
				href: string;
				type?: never;
				onclick?: never;
		  })
		| (CommonProps & {
				href?: never;
				type?: 'button' | 'submit' | 'reset';
				onclick?: MouseEventHandler<HTMLButtonElement>;
		  });
</script>

<script lang="ts">
	const BASE =
		'inline-flex items-center gap-1.5 rounded-[3px] font-bold uppercase tracking-wide ' +
		'transition-[background,border-color,color] duration-[0.12s] ' +
		'disabled:opacity-40 disabled:pointer-events-none ' +
		'aria-disabled:opacity-40 aria-disabled:pointer-events-none ' +
		'active:opacity-[0.88]';

	const SIZE_CLASSES: Record<ButtonSize, string> = {
		md: 'h-[23px] px-3 text-[9px]',
		sm: 'h-[19px] px-2 text-[8.5px]'
	};

	const VARIANT_CLASSES: Record<ButtonVariant, string> = {
		primary:
			'bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))] ' +
			'text-[var(--text-inverted)] ' +
			'hover:bg-[linear-gradient(90deg,var(--accent-dark),var(--accent-bright))]',
		ghost:
			'bg-transparent border border-[var(--border-default)] ' +
			'text-[var(--text-primary)] ' +
			'hover:bg-[var(--bg-raised)]',
		danger:
			'bg-[var(--color-error-bg)] ' +
			'border border-[var(--color-error-border)] ' +
			'text-[var(--color-error)] ' +
			'hover:bg-[var(--color-error-bg-hover)] ' +
			'hover:border-[var(--color-error-border-hover)]',
		secondary:
			'bg-[var(--bg-raised)] border border-[var(--border-default)] ' +
			'text-[var(--text-primary)] ' +
			'hover:bg-[var(--bg-hover)] ' +
			'active:opacity-[0.88]'
	};

	let {
		variant,
		size = 'md',
		disabled = false,
		loading = false,
		leadingIcon,
		trailingIcon,
		ariaLabel,
		children,
		class: className = '',
		href,
		type,
		onclick
	}: ButtonProps = $props();

	const computedClass = $derived(
		[BASE, SIZE_CLASSES[size], VARIANT_CLASSES[variant], className].filter(Boolean).join(' ')
	);

	const inert = $derived(disabled || loading);

	function handleLinkClick(e: MouseEvent) {
		if (inert) e.preventDefault();
	}

	function handleLinkKeydown(e: KeyboardEvent) {
		if (inert && (e.key === ' ' || e.key === 'Enter')) e.preventDefault();
	}
</script>

{#snippet spinner()}
	<span
		class="inline-block h-[9px] w-[9px] animate-spin rounded-full border border-current border-t-transparent [animation-duration:0.7s]"
		aria-hidden="true"
	></span>
{/snippet}

{#if href !== undefined}
	<a
		{href}
		role="button"
		aria-disabled={inert || undefined}
		aria-busy={loading || undefined}
		aria-label={ariaLabel ?? undefined}
		onclick={handleLinkClick}
		onkeydown={handleLinkKeydown}
		class={computedClass}
	>
		{#if loading}
			{@render spinner()}
		{:else if leadingIcon}
			{@render leadingIcon()}
		{/if}
		{@render children()}
		{#if trailingIcon && !loading}
			{@render trailingIcon()}
		{/if}
	</a>
{:else}
	<button
		type={type ?? 'button'}
		disabled={inert}
		aria-busy={loading || undefined}
		aria-label={ariaLabel ?? undefined}
		class={computedClass}
		onclick={inert ? undefined : onclick}
	>
		{#if loading}
			{@render spinner()}
		{:else if leadingIcon}
			{@render leadingIcon()}
		{/if}
		{@render children()}
		{#if trailingIcon && !loading}
			{@render trailingIcon()}
		{/if}
	</button>
{/if}
