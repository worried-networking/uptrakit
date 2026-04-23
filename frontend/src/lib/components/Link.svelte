<script lang="ts" module>
	import type { Snippet } from 'svelte';

	export type LinkVariant = 'default' | 'muted' | 'danger';

	export type LinkProps = {
		href: string;
		variant?: LinkVariant;
		external?: boolean;
		children: Snippet;
		class?: string;
	};
</script>

<script lang="ts">
	const BASE =
		'font-medium underline underline-offset-4 ' +
		'focus-visible:outline-none ' +
		'focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] ' +
		'transition-colors duration-fast';

	const VARIANT_CLASSES: Record<LinkVariant, string> = {
		default: 'text-[var(--accent)] hover:text-[var(--accent-bright)]',
		muted: 'text-[var(--text-muted)] hover:text-[var(--text-primary)]',
		danger: 'text-[var(--color-danger)] hover:text-[var(--color-danger)] hover:opacity-80'
	};

	let { href, variant = 'default', external = false, children, class: className = '' }: LinkProps = $props();

	const computedClass = $derived([BASE, VARIANT_CLASSES[variant], className].filter(Boolean).join(' '));
</script>

<a
	{href}
	target={external ? '_blank' : undefined}
	rel={external ? 'noopener noreferrer' : undefined}
	class={computedClass}
>
	{@render children()}
</a>
