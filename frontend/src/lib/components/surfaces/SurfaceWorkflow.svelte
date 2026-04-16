<script lang="ts">
	import Modal from '$lib/components/Modal.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import SchemaForm from '$lib/components/surfaces/SchemaForm.svelte';
	import { invokeSurfaceInteraction } from '$lib/api';
	import { buildSurfaceInteractionRequest, type SurfaceEncryptionContext } from '$lib/surfaces/interactions';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import type { SelectOption } from '$lib/types';
	import type { InteractionDescriptor, WorkflowStepDescriptor } from '$lib/surfaces/contract';
	import { SvelteMap } from 'svelte/reactivity';

	let {
		surfaceId,
		interaction,
		interactions = [],
		targetProviderId,
		encryptionContext,
		baseParams = {},
		size = 'md',
		oncomplete
	}: {
		surfaceId: string;
		interaction: InteractionDescriptor;
		interactions?: InteractionDescriptor[];
		targetProviderId?: string;
		encryptionContext?: SurfaceEncryptionContext;
		baseParams?: Record<string, unknown>;
		size?: 'sm' | 'md';
		oncomplete?: (result: unknown) => void | Promise<void>;
	} = $props();

	const WORKFLOW_FORM_ID = 'surface-workflow-step-form';

	let currentStep = $state(0);
	let loading = $state(false);
	let showModal = $state(false);
	let showConfirm = $state(false);
	let accumulatedParams: Record<string, unknown> = $state({});
	let accumulatedSensitive: Record<string, unknown> = $state({});
	let stepResponses: SvelteMap<number, unknown> = new SvelteMap();

	const actionLabel = $derived(interaction.label ?? interaction.interaction_id);
	const workflowSteps = $derived(interaction.workflow_steps ?? []);
	const step = $derived(workflowSteps[currentStep]);
	const isLastStep = $derived(currentStep === workflowSteps.length - 1);
	const buttonClass = $derived(size === 'sm' ? 'btn btn-sm text-xs' : 'btn');
	const presetClass = $derived(
		interaction.confirmation?.severity === 'danger' ? 'preset-filled-error-500' : 'preset-filled-primary-500'
	);
	const requestBaseParams = $derived(Object.fromEntries(Object.entries(baseParams).filter(([key]) => key !== '_row')));

	const impactColors: Record<string, string> = {
		high: 'preset-filled-error-500',
		medium: 'preset-filled-warning-500',
		low: 'preset-filled-primary-500',
		none: 'preset-tonal-surface'
	};

	function resetWorkflowState(): void {
		currentStep = 0;
		loading = false;
		accumulatedParams = {};
		accumulatedSensitive = {};
		stepResponses = new SvelteMap();
	}

	function sensitiveKeys(stepDescriptor: WorkflowStepDescriptor): Set<string> {
		const fields = stepDescriptor.form_ui?.fields ?? [];
		return new Set(fields.filter((field) => field.sensitive).map((field) => field.key));
	}

	function getPreviousResponse(): Record<string, unknown> | null {
		if (currentStep < 1) {
			return null;
		}
		const previous = stepResponses.get(currentStep - 1);
		if (!previous || typeof previous !== 'object' || Array.isArray(previous)) {
			return null;
		}
		return previous as Record<string, unknown>;
	}

	function toggleAction(actionId: string, checked: boolean): void {
		const currentUnchecked = (accumulatedParams._unchecked_actions as string[]) ?? [];
		if (checked) {
			accumulatedParams._unchecked_actions = currentUnchecked.filter((id) => id !== actionId);
		} else {
			accumulatedParams._unchecked_actions = [...currentUnchecked, actionId];
		}
		accumulatedParams = { ...accumulatedParams };
	}

	function buildSkipActions(): string[] {
		const reviewIndex = workflowSteps.findIndex((stepDescriptor) => stepDescriptor.render_previous_response);
		if (reviewIndex < 1) {
			return [];
		}
		const unchecked = accumulatedParams._unchecked_actions;
		return Array.isArray(unchecked) ? unchecked.filter((entry): entry is string => typeof entry === 'string') : [];
	}

	function findInteraction(interactionId: string): InteractionDescriptor | undefined {
		return interactions.find((candidate) => candidate.interaction_id === interactionId);
	}

	function isAutoExecutableStep(stepDescriptor: WorkflowStepDescriptor | undefined): boolean {
		if (!stepDescriptor || stepDescriptor.render_previous_response) {
			return false;
		}
		return !!stepDescriptor.submit_interaction_id && (stepDescriptor.form_ui?.fields?.length ?? 0) === 0;
	}

	async function submitStep(stepIndex: number, formValues: Record<string, unknown>): Promise<void> {
		const stepDescriptor = workflowSteps[stepIndex];
		if (!stepDescriptor) {
			return;
		}

		const stepFormFields = stepDescriptor.form_ui?.fields ?? [];
		const stepFieldKeys = new Set(stepFormFields.map((field) => field.key));
		const sensitiveFieldSet = sensitiveKeys(stepDescriptor);
		const regularStepParams: Record<string, unknown> = {};
		const sensitiveStepParams: Record<string, unknown> = {};

		for (const [key, value] of Object.entries(formValues)) {
			if (!stepFieldKeys.has(key) && key in baseParams) {
				continue;
			}
			if (sensitiveFieldSet.has(key)) {
				sensitiveStepParams[key] = value;
			} else {
				regularStepParams[key] = value;
			}
		}

		accumulatedParams = { ...accumulatedParams, ...regularStepParams };
		accumulatedSensitive = { ...accumulatedSensitive, ...sensitiveStepParams };

		const autoMode = accumulatedParams.auto === true;

		if (stepDescriptor.submit_interaction_id) {
			const submitInteraction = findInteraction(stepDescriptor.submit_interaction_id);
			if (!submitInteraction) {
				showError(`Missing step interaction "${stepDescriptor.submit_interaction_id}"`);
				return;
			}

			loading = true;
			try {
				const callParams = { ...requestBaseParams, ...accumulatedParams };
				if (stepIndex > 0 && !stepDescriptor.render_previous_response) {
					const skipActions = buildSkipActions();
					if (skipActions.length > 0) {
						callParams.skip_actions = skipActions;
					}
				}

				const request = await buildSurfaceInteractionRequest(
					submitInteraction,
					{ ...callParams, ...accumulatedSensitive },
					{
						targetProviderId,
						encryption: encryptionContext
					}
				);
				const result = await invokeSurfaceInteraction(surfaceId, stepDescriptor.submit_interaction_id, request);
				stepResponses.set(stepIndex, result);

				if (stepIndex === workflowSteps.length - 1) {
					showSuccess(`${actionLabel} completed`);
					showModal = false;
					await oncomplete?.(result);
					resetWorkflowState();
					return;
				}

				const nextStepIndex = stepIndex + 1;
				if (autoMode && workflowSteps[nextStepIndex]?.render_previous_response) {
					const executeStepIndex = Math.min(nextStepIndex + 1, workflowSteps.length - 1);
					const executeStep = workflowSteps[executeStepIndex];
					if (isAutoExecutableStep(executeStep)) {
						currentStep = executeStepIndex;
						await submitStep(executeStepIndex, {});
						return;
					}
					currentStep = executeStepIndex;
				} else {
					currentStep = nextStepIndex;
				}
			} catch (error) {
				showError(error instanceof Error ? error.message : 'Workflow step failed');
			} finally {
				loading = false;
			}
			return;
		}

		if (stepIndex === workflowSteps.length - 1) {
			showSuccess(`${actionLabel} completed`);
			showModal = false;
			await oncomplete?.(stepResponses.get(stepIndex - 1) ?? null);
			resetWorkflowState();
			return;
		}

		currentStep = stepIndex + 1;
	}

	async function handleStepSubmit(formValues: Record<string, unknown>): Promise<void> {
		await submitStep(currentStep, formValues);
	}

	async function handleReviewNext(): Promise<void> {
		if (!step) {
			return;
		}
		if (isLastStep) {
			showModal = false;
			await oncomplete?.(stepResponses.get(currentStep - 1) ?? null);
			resetWorkflowState();
			return;
		}

		const nextStepIndex = currentStep + 1;
		const nextStep = workflowSteps[nextStepIndex];
		if (nextStep && nextStep.submit_interaction_id && (nextStep.form_ui?.fields?.length ?? 0) === 0) {
			await submitStep(nextStepIndex, {});
			return;
		}
		currentStep = nextStepIndex;
	}

	function handleBack(): void {
		if (currentStep > 0) {
			currentStep -= 1;
		}
	}

	function startWorkflow(): void {
		if ((workflowSteps.length ?? 0) === 0) {
			showError('Workflow steps are missing from the surface contract.');
			return;
		}
		if (interaction.confirmation) {
			showConfirm = true;
			return;
		}
		showModal = true;
		resetWorkflowState();
	}

	async function loadStepInitialValues(): Promise<Record<string, unknown>> {
		const preLoadInteractionId = step?.form_ui?.pre_load_interaction_id;
		if (!preLoadInteractionId) {
			return {};
		}
		const preLoadInteraction = findInteraction(preLoadInteractionId);
		if (!preLoadInteraction) {
			return {};
		}
		const request = await buildSurfaceInteractionRequest(
			preLoadInteraction,
			{ ...requestBaseParams, ...accumulatedParams },
			{ targetProviderId }
		);
		const result = await invokeSurfaceInteraction(surfaceId, preLoadInteraction.interaction_id, request);
		if (result && typeof result === 'object' && !Array.isArray(result)) {
			return result as Record<string, unknown>;
		}
		return {};
	}

	async function loadSelectOptions(actionId: string): Promise<SelectOption[]> {
		const loadOptionsInteraction = findInteraction(actionId);
		if (!loadOptionsInteraction) {
			return [];
		}
		const request = await buildSurfaceInteractionRequest(
			loadOptionsInteraction,
			{ ...requestBaseParams, ...accumulatedParams },
			{
				targetProviderId,
				encryption: encryptionContext
			}
		);
		const result = await invokeSurfaceInteraction(surfaceId, loadOptionsInteraction.interaction_id, request);
		if (!result || typeof result !== 'object' || Array.isArray(result)) {
			return [];
		}
		return ((result as Record<string, unknown>).options as SelectOption[]) ?? [];
	}
</script>

<button type="button" class="{buttonClass} {presetClass}" disabled={loading} onclick={startWorkflow}>
	{loading ? 'Processing...' : actionLabel}
</button>

{#if showModal && step}
	<Modal
		title={actionLabel}
		maxWidth="max-w-2xl"
		onclose={() => {
			showModal = false;
			resetWorkflowState();
		}}
	>
		<div class="mb-6 flex items-center gap-2">
			{#each workflowSteps as workflowStep, index (workflowStep.step_id)}
				<div class="flex items-center gap-2">
					<div
						class="flex h-7 w-7 items-center justify-center rounded-full text-xs font-bold
							{index === currentStep
							? 'bg-primary-500 text-white'
							: index < currentStep
								? 'bg-primary-200 dark:bg-primary-800 text-primary-700 dark:text-primary-200'
								: 'bg-surface-200 dark:bg-surface-700 text-surface-500'}"
					>
						{index + 1}
					</div>
					<span class="text-sm {index === currentStep ? 'font-semibold' : 'text-surface-500'}">
						{workflowStep.label ?? workflowStep.step_id}
					</span>
					{#if index < workflowSteps.length - 1}
						<div class="mx-1 h-px w-6 bg-surface-300 dark:bg-surface-600"></div>
					{/if}
				</div>
			{/each}
		</div>

		{#if step.render_previous_response}
			{@const plan = getPreviousResponse()}
			{#if plan}
				{#if plan.host_info}
					{@const info = plan.host_info as Record<string, unknown>}
					<div class="card preset-tonal-surface mb-4 p-4">
						<h4 class="mb-2 text-sm font-semibold">Host Information</h4>
						<dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
							{#each Object.entries(info) as [key, value] (key)}
								<dt class="text-surface-500 whitespace-nowrap">{key.replace(/_/g, ' ')}</dt>
								<dd class="font-mono break-all">{String(value)}</dd>
							{/each}
						</dl>
					</div>
				{/if}

				{#if Array.isArray(plan.actions)}
					<div class="space-y-2">
						<h4 class="text-sm font-semibold">Planned Actions</h4>
						<div class="mb-3 mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-surface-500">
							<span>Security impact:</span>
							<span
								><span class="badge preset-filled-error-500 text-xs">high</span> — grants direct privileged access (e.g. sudoers
								NOPASSWD)</span
							>
							<span
								><span class="badge preset-filled-warning-500 text-xs">medium</span> — modifies system or infrastructure configuration</span
							>
							<span
								><span class="badge preset-filled-primary-500 text-xs">low</span> — minor privilege change (e.g. group membership)</span
							>
						</div>
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
												class="badge text-xs {impactColors[String(actionObj.security_impact)] ??
													'preset-tonal-surface'}"
											>
												{actionObj.security_impact}
											</span>
										{/if}
										{#if !isSkippable}
											<span class="badge preset-tonal-surface text-xs">required</span>
										{/if}
									</div>
									<p class="mt-0.5 text-xs text-surface-500">{actionObj.description}</p>
									{#if Array.isArray(actionObj.commands) && (actionObj.commands as string[]).length > 0}
										<ul class="mt-1 space-y-0.5 text-xs text-surface-500">
											{#each actionObj.commands as cmd (cmd)}
												<li class="font-mono">{cmd}</li>
											{/each}
										</ul>
									{/if}
								</div>
							</label>
						{/each}
					</div>
				{/if}
			{:else}
				<p class="text-surface-500">No plan data available.</p>
			{/if}
		{:else if (step.form_ui?.fields?.length ?? 0) > 0}
			<SchemaForm
				fields={step.form_ui?.fields ?? []}
				onsubmit={handleStepSubmit}
				formId={WORKFLOW_FORM_ID}
				hideSubmit={true}
				{loading}
				extraParams={requestBaseParams}
				loadInitialValues={step.form_ui?.pre_load_interaction_id ? loadStepInitialValues : undefined}
				{loadSelectOptions}
			/>
		{:else if loading}
			<div class="flex items-center justify-center py-8">
				<div class="border-primary-500 h-8 w-8 animate-spin rounded-full border-4 border-t-transparent"></div>
				<span class="ml-3 text-surface-500">Executing...</span>
			</div>
		{/if}

		{#snippet footer()}
			<button
				class="btn preset-tonal-surface"
				onclick={() => {
					showModal = false;
					resetWorkflowState();
				}}
				disabled={loading}
			>
				Cancel
			</button>
			{#if currentStep > 0}
				<button class="btn preset-tonal-surface" onclick={handleBack} disabled={loading}>Back</button>
			{/if}
			{#if step.render_previous_response}
				<button class="btn preset-filled-primary-500" onclick={handleReviewNext} disabled={loading}>
					{isLastStep ? 'Done' : 'Execute'}
				</button>
			{:else if (step.form_ui?.fields?.length ?? 0) > 0}
				<button class="btn preset-filled-primary-500" type="submit" form={WORKFLOW_FORM_ID} disabled={loading}>
					{isLastStep ? 'Run' : 'Continue'}
				</button>
			{:else if step.submit_interaction_id}
				<button class="btn preset-filled-primary-500" onclick={() => void handleStepSubmit({})} disabled={loading}>
					{isLastStep ? 'Run' : 'Continue'}
				</button>
			{:else}
				<button class="btn preset-filled-primary-500" onclick={handleReviewNext} disabled={loading}>
					{isLastStep ? 'Done' : 'Continue'}
				</button>
			{/if}
		{/snippet}
	</Modal>
{/if}

{#if showConfirm && interaction.confirmation}
	<ConfirmDialog
		title={interaction.confirmation.title}
		messagePrefix={interaction.confirmation.message}
		entityName={actionLabel}
		confirmLabel={interaction.confirmation.confirm_label ?? actionLabel}
		onconfirm={() => {
			showConfirm = false;
			showModal = true;
			resetWorkflowState();
		}}
		oncancel={() => {
			showConfirm = false;
		}}
	/>
{/if}
