<script lang="ts" module>
	import type { Snippet } from 'svelte';
	import type { MouseEventHandler } from 'svelte/elements';

	export type ButtonVariant = 'primary' | 'ghost' | 'danger' | 'secondary' | 'accent';
	export type ButtonSize = 'sm' | 'md';

	type CommonProps = {
		variant: ButtonVariant;
		size?: ButtonSize;
		disabled?: boolean;
		loading?: boolean;
		leadingIcon?: Snippet;
		trailingIcon?: Snippet;
		ariaLabel?: string;
		ariaCurrent?: 'page' | 'step' | 'location' | 'date' | 'time' | boolean;
		'aria-controls'?: string;
		'aria-expanded'?: boolean;
		'aria-haspopup'?: 'menu' | 'listbox' | 'tree' | 'grid' | 'dialog' | boolean;
		children?: Snippet;
		class?: string;
		'data-ui'?: string;
		form?: string;
	};

	export type ButtonProps =
		| (CommonProps & {
				href: string;
				target?: string;
				rel?: string;
				type?: never;
				onclick?: never;
		  })
		| (CommonProps & {
				href?: never;
				target?: never;
				rel?: never;
				type?: 'button' | 'submit' | 'reset';
				onclick?: MouseEventHandler<HTMLButtonElement>;
		  });
</script>

<script lang="ts">
	const BASE =
		'inline-flex items-center gap-1.5 rounded-card font-bold uppercase ' +
		'transition-[background,border-color,color] duration-fast ' +
		'disabled:opacity-40 disabled:pointer-events-none ' +
		'aria-disabled:opacity-40 aria-disabled:pointer-events-none ' +
		'active:opacity-pressed ' +
		'focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]';

	const SIZE_CLASSES: Record<ButtonSize, string> = {
		md: 'h-[23px] px-3 text-button',
		sm: 'h-[19px] px-2 text-button-sm'
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
			'bg-[var(--color-danger-bg)] ' +
			'border border-[var(--color-danger-border)] ' +
			'text-[var(--color-danger)] ' +
			'hover:bg-[var(--color-danger-bg-hover)] ' +
			'hover:border-[var(--color-danger-border-hover)]',
		secondary:
			'bg-[var(--bg-raised)] border border-[var(--border-default)] ' +
			'text-[var(--text-primary)] ' +
			'hover:bg-[var(--bg-hover)] ' +
			'active:opacity-pressed',
		accent:
			'bg-[var(--accent)] border border-[var(--accent)] ' +
			'text-[var(--text-inverted)] ' +
			'hover:bg-[var(--accent-bright)] hover:border-[var(--accent-bright)]'
	};

	let {
		variant,
		size = 'md',
		disabled = false,
		loading = false,
		leadingIcon,
		trailingIcon,
		ariaLabel,
		ariaCurrent,
		'aria-controls': ariaControls,
		'aria-expanded': ariaExpanded,
		'aria-haspopup': ariaHaspopup,
		children,
		class: className = '',
		href,
		target,
		rel,
		type,
		form,
		onclick,
		'data-ui': dataUi
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
		{target}
		{rel}
		role="button"
		aria-disabled={inert || undefined}
		aria-busy={loading || undefined}
		aria-label={ariaLabel ?? undefined}
		aria-controls={ariaControls ?? undefined}
		aria-expanded={ariaExpanded ?? undefined}
		aria-haspopup={ariaHaspopup ?? undefined}
		data-ui={dataUi}
		onclick={handleLinkClick}
		onkeydown={handleLinkKeydown}
		class={computedClass}
	>
		{#if loading}
			{@render spinner()}
		{:else if leadingIcon}
			{@render leadingIcon()}
		{/if}
		{#if children}{@render children()}{/if}
		{#if trailingIcon && !loading}
			{@render trailingIcon()}
		{/if}
	</a>
{:else}
	<button
		type={type ?? 'button'}
		{form}
		disabled={inert}
		aria-busy={loading || undefined}
		aria-label={ariaLabel ?? undefined}
		aria-current={ariaCurrent ?? undefined}
		aria-controls={ariaControls ?? undefined}
		aria-expanded={ariaExpanded ?? undefined}
		aria-haspopup={ariaHaspopup ?? undefined}
		data-ui={dataUi}
		class={computedClass}
		onclick={inert ? undefined : onclick}
	>
		{#if loading}
			{@render spinner()}
		{:else if leadingIcon}
			{@render leadingIcon()}
		{/if}
		{#if children}{@render children()}{/if}
		{#if trailingIcon && !loading}
			{@render trailingIcon()}
		{/if}
	</button>
{/if}
