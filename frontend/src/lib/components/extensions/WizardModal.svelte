<script lang="ts">
	import type { WizardStep, FieldDef } from '$lib/types';
	import { invokeExtensionAction } from '$lib/api';
	import { showError } from '$lib/notifications.svelte';
	import { SvelteMap } from 'svelte/reactivity';
	import SchemaForm from './SchemaForm.svelte';
	import Modal from '$lib/components/Modal.svelte';

	let {
		steps,
		actionLabel,
		extensionId,
		serviceId,
		encryptionPublicKey,
		extraParams = {},
		onclose,
		oncomplete
	}: {
		steps: WizardStep[];
		actionLabel: string;
		extensionId: string;
		serviceId?: string;
		encryptionPublicKey?: string;
		extraParams?: Record<string, unknown>;
		onclose: () => void;
		oncomplete: (result?: unknown) => void | Promise<void>;
	} = $props();

	let currentStep: number = $state(0);
	let loading: boolean = $state(false);
	/** Accumulated form values across all steps. */
	let accumulatedParams: Record<string, unknown> = $state({});
	/** Accumulated sensitive param values across all steps. */
	let accumulatedSensitive: Record<string, unknown> = $state({});
	/** Response data from each step's submit_action, keyed by step index. */
	let stepResponses: SvelteMap<number, unknown> = new SvelteMap();

	let step = $derived(steps[currentStep]);
	let isLastStep = $derived(currentStep === steps.length - 1);

	/** Collect sensitive field keys from a step's form fields. */
	function sensitiveKeys(fields: FieldDef[]): Set<string> {
		return new Set(fields.filter((f) => f.sensitive).map((f) => f.key));
	}

	/** Parse skip_actions from the review step's plan data. */
	function buildSkipActions(): string[] {
		// Find the connect step's response (step before review)
		const reviewIndex = steps.findIndex((s) => s.render_previous_response);
		if (reviewIndex < 1) return [];
		const planData = stepResponses.get(reviewIndex - 1) as Record<string, unknown> | undefined;
		if (!planData) return [];

		const actions = planData.actions as Array<Record<string, unknown>> | undefined;
		if (!Array.isArray(actions)) return [];

		// uncheckedActions is set by the review UI — actions the user deselected
		const unchecked = accumulatedParams._unchecked_actions as string[] | undefined;
		return unchecked ?? [];
	}

	async function handleStepSubmit(formValues: Record<string, unknown>) {
		const sensitive = sensitiveKeys(step.form.fields);
		const regularParams: Record<string, unknown> = {};
		const sensitiveParams: Record<string, unknown> = {};

		for (const [k, v] of Object.entries(formValues)) {
			// Don't include extraParams that SchemaForm merges in — we manage those separately.
			if (k in extraParams) continue;
			if (sensitive.has(k)) {
				sensitiveParams[k] = v;
			} else {
				regularParams[k] = v;
			}
		}

		// Merge into accumulated state.
		accumulatedParams = { ...accumulatedParams, ...regularParams };
		accumulatedSensitive = { ...accumulatedSensitive, ...sensitiveParams };

		// Check auto toggle — if set and we're on step 0, skip review step.
		const autoMode = accumulatedParams.auto === true;

		if (step.submit_action) {
			loading = true;
			try {
				// Build params for the action call.
				const callParams = { ...extraParams, ...accumulatedParams };

				// For execute steps, include skip_actions.
				if (currentStep > 0 && !step.render_previous_response) {
					const skipActions = buildSkipActions();
					if (skipActions.length > 0) {
						callParams.skip_actions = skipActions;
					}
				}

				const result = await invokeExtensionAction(
					extensionId,
					step.submit_action,
					callParams,
					serviceId,
					Object.keys(accumulatedSensitive).length > 0 ? accumulatedSensitive : undefined,
					encryptionPublicKey
				);
				stepResponses.set(currentStep, result);

				if (isLastStep) {
					await oncomplete(result);
				} else if (autoMode && steps[currentStep + 1]?.render_previous_response) {
					// Auto mode: skip the review step and go to execute.
					currentStep += 2;
				} else {
					currentStep += 1;
				}
			} catch (e) {
				showError(e instanceof Error ? e.message : 'Action failed');
			} finally {
				loading = false;
			}
		} else if (isLastStep) {
			await oncomplete();
		} else {
			currentStep += 1;
		}
	}

	/** Handle "Next" on a review step (no form submission, just advance). */
	async function handleReviewNext() {
		if (isLastStep) {
			await oncomplete();
		} else {
			currentStep += 1;
			// If the next step has a submit_action and no form fields, auto-submit.
			const nextStep = steps[currentStep];
			if (nextStep?.submit_action && nextStep.form.fields.length === 0) {
				await handleStepSubmit({});
			}
		}
	}

	function handleBack() {
		if (currentStep > 0) {
			// If previous step is a review step and the one before that was auto, skip back over review.
			currentStep -= 1;
		}
	}

	/** Toggle an action in the unchecked list. */
	function toggleAction(actionId: string, checked: boolean) {
		const current = (accumulatedParams._unchecked_actions as string[]) ?? [];
		if (checked) {
			accumulatedParams._unchecked_actions = current.filter((id) => id !== actionId);
		} else {
			accumulatedParams._unchecked_actions = [...current, actionId];
		}
		accumulatedParams = { ...accumulatedParams };
	}

	/** Get the plan data from the previous step's response. */
	function getPreviousResponse(): Record<string, unknown> | null {
		if (currentStep < 1) return null;
		return (stepResponses.get(currentStep - 1) as Record<string, unknown>) ?? null;
	}

	const impactColors: Record<string, string> = {
		high: 'preset-filled-error-500',
		medium: 'preset-filled-warning-500',
		low: 'preset-filled-primary-500',
		none: 'preset-tonal-surface'
	};
</script>

<Modal title={actionLabel} maxWidth="max-w-2xl" {onclose}>
	<!-- Step indicator -->
	<div class="mb-6 flex items-center gap-2">
		{#each steps as s, i (s.step_id)}
			<div class="flex items-center gap-2">
				<div
					class="flex h-7 w-7 items-center justify-center rounded-full text-xs font-bold
						{i === currentStep
						? 'bg-primary-500 text-white'
						: i < currentStep
							? 'bg-primary-200 dark:bg-primary-800 text-primary-700 dark:text-primary-200'
							: 'bg-surface-200 dark:bg-surface-700 text-surface-500'}"
				>
					{i + 1}
				</div>
				<span class="text-sm {i === currentStep ? 'font-semibold' : 'text-surface-500'}">
					{s.label}
				</span>
				{#if i < steps.length - 1}
					<div class="mx-1 h-px w-6 bg-surface-300 dark:bg-surface-600"></div>
				{/if}
			</div>
		{/each}
	</div>

	<!-- Step content -->
	{#if step.render_previous_response}
		<!-- Review step: display previous step's response -->
		{@const plan = getPreviousResponse()}
		{#if plan}
			<!-- Host info summary -->
			{#if plan.host_info}
				{@const info = plan.host_info as Record<string, unknown>}
				<div class="card preset-tonal-surface mb-4 p-4">
					<h4 class="mb-2 text-sm font-semibold">Host Information</h4>
					<dl class="grid grid-cols-2 gap-x-4 gap-y-1 text-sm">
						{#each Object.entries(info) as [key, value] (key)}
							<dt class="text-surface-500">{key.replace(/_/g, ' ')}</dt>
							<dd class="font-mono">{String(value)}</dd>
						{/each}
					</dl>
				</div>
			{/if}

			<!-- Actions list with toggles -->
			{#if Array.isArray(plan.actions)}
				<div class="space-y-2">
					<h4 class="text-sm font-semibold">Planned Actions</h4>
					{#each plan.actions as action, idx (idx)}
						{@const actionObj = action as Record<string, unknown>}
						{@const unchecked = (accumulatedParams._unchecked_actions as string[]) ?? []}
						{@const isChecked = !unchecked.includes(String(actionObj.id))}
						{@const isSkippable = Boolean(actionObj.skippable)}
						<label class="card flex items-start gap-3 p-3 {isChecked ? 'preset-tonal-surface' : 'opacity-60'}">
							<input
								type="checkbox"
								class="checkbox mt-0.5"
								checked={isChecked}
								disabled={!isSkippable}
								onchange={() => toggleAction(String(actionObj.id), !isChecked)}
							/>
							<div class="flex-1">
								<div class="flex items-center gap-2">
									<span class="text-sm font-medium">{actionObj.label}</span>
									{#if actionObj.security_impact && actionObj.security_impact !== 'none'}
										<span
											class="badge text-xs {impactColors[String(actionObj.security_impact)] ?? 'preset-tonal-surface'}"
										>
											{actionObj.security_impact}
										</span>
									{/if}
									{#if !isSkippable}
										<span class="badge preset-tonal-surface text-xs">required</span>
									{/if}
								</div>
								<p class="mt-0.5 text-xs text-surface-500">{actionObj.description}</p>
							</div>
						</label>
					{/each}
				</div>
			{/if}
		{:else}
			<p class="text-surface-500">No plan data available.</p>
		{/if}
	{:else if step.form.fields.length > 0}
		<!-- Form step -->
		<SchemaForm
			fields={step.form.fields}
			onsubmit={handleStepSubmit}
			submitLabel={isLastStep ? 'Execute' : 'Next'}
			{loading}
			{extensionId}
			{serviceId}
			{extraParams}
			preLoadAction={step.form.pre_load_action}
		/>
	{:else}
		<!-- Empty form step with submit_action (auto-submitted) -->
		{#if loading}
			<div class="flex items-center justify-center py-8">
				<div class="border-primary-500 h-8 w-8 animate-spin rounded-full border-4 border-t-transparent"></div>
				<span class="ml-3 text-surface-500">Executing...</span>
			</div>
		{/if}
	{/if}

	{#snippet footer()}
		<button class="btn preset-tonal-surface" onclick={onclose} disabled={loading}> Cancel </button>
		{#if currentStep > 0}
			<button class="btn preset-tonal-surface" onclick={handleBack} disabled={loading}> Back </button>
		{/if}
		{#if step.render_previous_response}
			<button class="btn preset-filled-primary-500" onclick={handleReviewNext} disabled={loading}>
				{isLastStep ? 'Done' : 'Execute'}
			</button>
		{/if}
	{/snippet}
</Modal>
