<script lang="ts">
	import { Input, Checkbox, Textarea, Select } from '$lib/components/forms';
	import type { InputType } from '$lib/components/forms';
	import Link from '$lib/components/Link.svelte';
	import type { LinkVariant } from '$lib/components/Link.svelte';

	const INPUT_TYPES: InputType[] = ['text', 'email', 'password', 'url', 'number', 'search'];
	const LINK_VARIANTS: LinkVariant[] = ['default', 'muted', 'danger'];

	let checkedA = $state(false);
	let checkedB = $state(true);
	let selectVal = $state('b');
	let selectValEmpty = $state('');
</script>

<main class="flex flex-col gap-6 p-6" data-testid="form-primitive-preview-root">
	<section data-testid="input-types">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Input — type matrix</h2>
		<div class="flex flex-col gap-3" style="width: 320px;">
			{#each INPUT_TYPES as type (type)}
				<div data-testid="input-cell-{type}">
					<label class="mb-1 block text-xs text-[var(--text-muted)]" for="preview-{type}">
						{type}
					</label>
					<Input id="preview-{type}" {type} value="" placeholder={type} />
				</div>
			{/each}
		</div>
	</section>

	<section data-testid="input-states">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Input — states</h2>
		<div class="flex flex-col gap-3" style="width: 320px;">
			<div data-testid="input-cell-normal">
				<label class="mb-1 block text-xs text-[var(--text-muted)]" for="preview-normal">normal</label>
				<Input id="preview-normal" type="text" value="" placeholder="Normal input" />
			</div>
			<div data-testid="input-cell-disabled">
				<label class="mb-1 block text-xs text-[var(--text-muted)]" for="preview-disabled">disabled</label>
				<Input id="preview-disabled" type="text" value="" placeholder="Disabled input" disabled />
			</div>
			<div data-testid="input-cell-error">
				<label class="mb-1 block text-xs text-[var(--text-muted)]" for="preview-error">error</label>
				<Input id="preview-error" type="text" value="" placeholder="Error input" error="This field is required" />
			</div>
		</div>
	</section>

	<section data-testid="checkbox-states">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Checkbox — states</h2>
		<div class="flex flex-col gap-3">
			<div class="flex items-center gap-2" data-testid="checkbox-cell-unchecked">
				<Checkbox id="preview-checkbox-unchecked" bind:checked={checkedA} />
				<label class="text-sm text-[var(--text-primary)]" for="preview-checkbox-unchecked">Unchecked</label>
			</div>
			<div class="flex items-center gap-2" data-testid="checkbox-cell-checked">
				<Checkbox id="preview-checkbox-checked" bind:checked={checkedB} />
				<label class="text-sm text-[var(--text-primary)]" for="preview-checkbox-checked">Checked</label>
			</div>
			<div class="flex items-center gap-2" data-testid="checkbox-cell-disabled">
				<Checkbox id="preview-checkbox-disabled" checked={false} disabled />
				<label class="text-sm text-[var(--text-primary)]" for="preview-checkbox-disabled">Disabled unchecked</label>
			</div>
			<div class="flex items-center gap-2" data-testid="checkbox-cell-disabled-checked">
				<Checkbox id="preview-checkbox-disabled-checked" checked={true} disabled />
				<label class="text-sm text-[var(--text-primary)]" for="preview-checkbox-disabled-checked"
					>Disabled checked</label
				>
			</div>
		</div>
	</section>

	<section data-testid="link-variants">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Link — variants</h2>
		<div class="flex flex-wrap gap-4">
			{#each LINK_VARIANTS as variant (variant)}
				<div data-testid="link-cell-{variant}">
					<Link href="/dev/form-primitive-preview" {variant}>{variant} link</Link>
				</div>
			{/each}
		</div>
	</section>

	<section data-testid="link-external">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Link — external</h2>
		<div class="flex flex-wrap gap-4">
			<div data-testid="link-cell-external">
				<Link href="https://example.com" external>External link</Link>
			</div>
			<div data-testid="link-cell-internal">
				<Link href="/dev/form-primitive-preview">Internal link</Link>
			</div>
		</div>
	</section>

	<section data-testid="textarea-states">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Textarea — states</h2>
		<div class="flex flex-col gap-3" style="width: 480px;">
			<div data-testid="textarea-cell-default">
				<label class="mb-1 block text-xs text-[var(--text-muted)]" for="preview-ta-default"> default / normal </label>
				<Textarea id="preview-ta-default" value="" placeholder="Default textarea" rows={4} />
			</div>
			<div data-testid="textarea-cell-error">
				<label class="mb-1 block text-xs text-[var(--text-muted)]" for="preview-ta-error"> default / error </label>
				<Textarea id="preview-ta-error" value="" placeholder="Error textarea" rows={4} error="This field is required" />
			</div>
			<div data-testid="textarea-cell-mono">
				<label class="mb-1 block text-xs text-[var(--text-muted)]" for="preview-ta-mono"> mono / normal </label>
				<Textarea id="preview-ta-mono" value="" placeholder={'{ "key": "value" }'} rows={4} variant="mono" />
			</div>
			<div data-testid="textarea-cell-mono-error">
				<label class="mb-1 block text-xs text-[var(--text-muted)]" for="preview-ta-mono-error"> mono / error </label>
				<Textarea
					id="preview-ta-mono-error"
					value=""
					placeholder={'{ "key": "value" }'}
					rows={4}
					variant="mono"
					error="Invalid JSON"
				/>
			</div>
			<div data-testid="textarea-cell-disabled">
				<label class="mb-1 block text-xs text-[var(--text-muted)]" for="preview-ta-disabled"> disabled </label>
				<Textarea id="preview-ta-disabled" value="" placeholder="Disabled textarea" rows={4} disabled />
			</div>
		</div>
	</section>

	<section class="space-y-4 p-6">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Select — states</h2>
		<div class="grid grid-cols-2 gap-4">
			<div>
				<p class="mb-1 text-xs text-[var(--text-muted)]">Default</p>
				<Select
					id="preview-select-default"
					bind:value={selectVal}
					options={[
						{ value: 'a', label: 'Option A' },
						{ value: 'b', label: 'Option B' },
						{ value: 'c', label: 'Option C' }
					]}
				/>
			</div>
			<div>
				<p class="mb-1 text-xs text-[var(--text-muted)]">With placeholder</p>
				<Select
					id="preview-select-placeholder"
					bind:value={selectValEmpty}
					options={[
						{ value: 'x', label: 'Choice X' },
						{ value: 'y', label: 'Choice Y' }
					]}
					placeholder="Select an option"
				/>
			</div>
			<div>
				<p class="mb-1 text-xs text-[var(--text-muted)]">Error</p>
				<Select
					id="preview-select-error"
					bind:value={selectVal}
					options={[
						{ value: 'a', label: 'Option A' },
						{ value: 'b', label: 'Option B' }
					]}
					error="This field is required"
				/>
			</div>
			<div>
				<p class="mb-1 text-xs text-[var(--text-muted)]">Disabled</p>
				<Select
					id="preview-select-disabled"
					value="a"
					options={[
						{ value: 'a', label: 'Option A' },
						{ value: 'b', label: 'Option B' }
					]}
					disabled
				/>
			</div>
			<div>
				<p class="mb-1 text-xs text-[var(--text-muted)]">Grouped + disabled</p>
				<Select
					id="demo-grouped"
					placeholder="Select config..."
					options={[
						{
							label: 'Saved',
							options: [
								{ value: 'cfg:1', label: 'Production' },
								{ value: 'cfg:2', label: 'Staging' }
							]
						},
						{
							label: 'Inline',
							options: [
								{ value: 'type:apt', label: 'APT (deprecated)', disabled: true },
								{ value: 'type:docker', label: 'Docker' }
							]
						}
					]}
				/>
			</div>
		</div>
	</section>
</main>
