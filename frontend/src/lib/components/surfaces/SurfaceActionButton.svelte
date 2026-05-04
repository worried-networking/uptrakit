<script lang="ts" module>
	import type { ButtonVariant, ButtonSize } from '$lib/components/Button.svelte';
	import type { LabelDisplay } from '$lib/surfaces/label-display';

	export interface SurfaceActionButtonProps {
		label: string;
		icon?: string | null;
		labelDisplay?: LabelDisplay;
		variant: ButtonVariant;
		size?: ButtonSize;
		loading?: boolean;
		disabled?: boolean;
		onclick?: () => void;
		dataUi?: string;
	}
</script>

<script lang="ts">
	import Button from '$lib/components/Button.svelte';
	import { resolveIcon } from '$lib/icons';

	let {
		label,
		icon,
		labelDisplay = 'always',
		variant,
		size = 'md',
		loading = false,
		disabled = false,
		onclick,
		dataUi
	}: SurfaceActionButtonProps = $props();

	const resolved = $derived(resolveIcon(icon));
	const effectiveDisplay = $derived(resolved.ok ? labelDisplay : 'always');
	const iconSize = $derived(size === 'sm' ? 14 : 16);
	const showTooltipWrap = $derived(effectiveDisplay !== 'always' && !disabled);
	const ariaLabel = $derived(effectiveDisplay !== 'always' ? label : undefined);
</script>

{#snippet leadingIcon()}
	{#if resolved.ok}
		{@const Component = resolved.component}
		<Component size={iconSize} aria-hidden="true" />
	{/if}
{/snippet}

{#snippet body()}
	<Button
		{variant}
		{size}
		{loading}
		{disabled}
		{onclick}
		{ariaLabel}
		data-ui={dataUi}
		leadingIcon={resolved.ok ? leadingIcon : undefined}
	>
		{#if effectiveDisplay === 'always'}
			{label}
		{:else if effectiveDisplay === 'auto'}
			<span class="button-label-auto @max-[28em]/buttons:sr-only">{label}</span>
		{:else}
			<span class="sr-only">{label}</span>
		{/if}
	</Button>
{/snippet}

{#if showTooltipWrap}
	<span title={label} class="inline-flex">
		{@render body()}
	</span>
{:else}
	{@render body()}
{/if}
