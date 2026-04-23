<script lang="ts">
	import Modal from '$lib/components/Modal.svelte';
	import Button from '$lib/components/Button.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import SchemaForm from '$lib/components/surfaces/SchemaForm.svelte';
	import { invokeSurfaceInteraction } from '$lib/api';
	import Callout from '$lib/components/ui/Callout.svelte';
	import SectionCard from '$lib/components/ui/SectionCard.svelte';
	import StatusBadge from '$lib/components/ui/StatusBadge.svelte';
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
	let showContractIssue = $state(false);
	let accumulatedParams: Record<string, unknown> = $state({});
	let accumulatedSensitive: Record<string, unknown> = $state({});
	let stepResponses: SvelteMap<number, unknown> = new SvelteMap();

	const workflowSteps = $derived(interaction.workflow_steps ?? []);
	const actionLabel = $derived(typeof interaction.label === 'string' ? interaction.label.trim() : '');
	const confirmLabel = $derived(interaction.confirmation?.confirm_label?.trim() || actionLabel);
	const step = $derived(workflowSteps[currentStep]);
	const isLastStep = $derived(currentStep === workflowSteps.length - 1);
	const confirmVariantForSeverity = $derived<'danger' | 'primary'>(
		interaction.confirmation?.severity === 'danger' ? 'danger' : 'primary'
	);
	const requestBaseParams = $derived(Object.fromEntries(Object.entries(baseParams).filter(([key]) => key !== '_row')));

	function resetWorkflowState(): void {
		currentStep = 0;
		loading = false;
		accumulatedParams = {};
		accumulatedSensitive = {};
		stepResponses = new SvelteMap();
	}

	function markContractIssue(): void {
		showContractIssue = true;
		showConfirm = false;
		showModal = false;
		resetWorkflowState();
	}

	function hasContractIssue(): boolean {
		if (actionLabel.length === 0 || workflowSteps.length === 0) {
			return true;
		}
		for (const stepDescriptor of workflowSteps) {
			if (
				typeof stepDescriptor.label !== 'string' ||
				stepDescriptor.label.trim().length === 0 ||
				(stepDescriptor.submit_interaction_id && !findInteraction(stepDescriptor.submit_interaction_id))
			) {
				return true;
			}
		}
		return false;
	}

	$effect(() => {
		if (showContractIssue && !hasContractIssue()) {
			showContractIssue = false;
		}
	});

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
				markContractIssue();
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
		if (hasContractIssue()) {
			markContractIssue();
			return;
		}
		showContractIssue = false;
		if (interaction.confirmation) {
			showConfirm = true;
			return;
		}
		showModal = true;
		resetWorkflowState();
	}

	function stepIndicatorState(index: number): 'active' | 'completed' | 'upcoming' {
		if (index === currentStep) {
			return 'active';
		}
		if (index < currentStep) {
			return 'completed';
		}
		return 'upcoming';
	}

	function stepChipLabel(stepDescriptor: WorkflowStepDescriptor): string {
		return typeof stepDescriptor.label === 'string' ? stepDescriptor.label.trim() : '';
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

{#if actionLabel.length === 0 || showContractIssue}
	<Callout tone="warning" title="Action unavailable" message="This action is not available right now." />
{:else}
	<Button variant={confirmVariantForSeverity} {size} {loading} data-ui="workflow-trigger" onclick={startWorkflow}>
		{actionLabel}
	</Button>
{/if}

{#if showModal && step}
	<Modal
		title={actionLabel}
		maxWidth="max-w-2xl"
		onclose={() => {
			showModal = false;
			resetWorkflowState();
		}}
	>
		<div class="mb-6 flex flex-wrap items-center gap-2" data-ui="workflow-step-indicator">
			{#each workflowSteps as workflowStep, index (workflowStep.step_id)}
				{@const state = stepIndicatorState(index)}
				<span
					data-ui="workflow-step-chip"
					data-state={state}
					class="inline-flex h-[18px] items-center rounded-badge px-2 text-badge font-semibold uppercase tracking-badge {state ===
					'completed'
						? 'bg-[var(--color-success-bg)] text-[var(--color-success)]'
						: state === 'active'
							? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
							: 'bg-[var(--bg-raised)] text-[var(--text-secondary)]'}"
				>
					{index + 1}. {stepChipLabel(workflowStep)}
				</span>
			{/each}
		</div>

		{#if step.render_previous_response}
			{@const plan = getPreviousResponse()}
			{#if plan}
				<div class="space-y-4" data-ui="workflow-review-state">
					{#if plan.host_info}
						{@const info = plan.host_info as Record<string, unknown>}
						<div class="mb-4">
							<SectionCard title="Host Information">
								<dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
									{#each Object.entries(info) as [key, value] (key)}
										<dt class="text-[var(--text-muted)] whitespace-nowrap">{key.replace(/_/g, ' ')}</dt>
										<dd class="font-mono break-all">{String(value)}</dd>
									{/each}
								</dl>
							</SectionCard>
						</div>
					{/if}

					{#if Array.isArray(plan.actions)}
						{@const hasHighImpact = plan.actions.some(
							(entry) =>
								typeof entry === 'object' &&
								entry !== null &&
								(entry as Record<string, unknown>).security_impact === 'high'
						)}
						<div class="space-y-2">
							<h4 class="text-sm font-semibold">Planned Actions</h4>
							<div class="space-y-2" data-ui="workflow-security-impact">
								<Callout
									tone={hasHighImpact ? 'danger' : 'warning'}
									title="Security impact"
									message="Review privileged or system-level actions before you execute this workflow."
								/>
								<div class="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-[var(--text-muted)]">
									<span
										><StatusBadge tone="danger" label="High" /> — grants direct privileged access (e.g. sudoers NOPASSWD)</span
									>
									<span
										><StatusBadge tone="warning" label="Medium" /> — modifies system or infrastructure configuration</span
									>
									<span><StatusBadge tone="info" label="Low" /> — minor privilege change (e.g. group membership)</span>
								</div>
							</div>
							{#each plan.actions as action, idx (idx)}
								{@const actionObj = action as Record<string, unknown>}
								{@const unchecked = (accumulatedParams._unchecked_actions as string[]) ?? []}
								{@const isChecked = !unchecked.includes(String(actionObj.id))}
								{@const isSkippable = Boolean(actionObj.skippable)}
								{@const impact = String(actionObj.security_impact ?? '').toLowerCase()}
								<label
									class="rounded-card border border-[var(--border-subtle)] flex items-start gap-3 p-3 {isChecked
										? 'bg-[var(--bg-raised)]'
										: 'bg-[var(--bg-surface)] opacity-60'}"
								>
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
											{#if impact && impact !== 'none'}
												<StatusBadge
													tone={impact === 'high' ? 'danger' : impact === 'medium' ? 'warning' : 'info'}
													label={impact.charAt(0).toUpperCase() + impact.slice(1)}
												/>
											{/if}
											{#if !isSkippable}
												<StatusBadge tone="neutral" label="required" />
											{/if}
										</div>
										<p class="mt-0.5 text-xs text-[var(--text-muted)]">{actionObj.description}</p>
										{#if Array.isArray(actionObj.commands) && (actionObj.commands as string[]).length > 0}
											<ul class="mt-1 space-y-0.5 text-xs text-[var(--text-muted)]">
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
				</div>
			{:else}
				<Callout tone="info" message="No plan data available." />
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
				<div class="border-[var(--accent)] h-8 w-8 animate-spin rounded-full border-4 border-t-transparent"></div>
				<span class="ml-3 text-[var(--text-muted)]">Executing...</span>
			</div>
		{/if}

		{#snippet footer()}
			<Button
				variant="secondary"
				disabled={loading}
				onclick={() => {
					showModal = false;
					resetWorkflowState();
				}}
			>
				Cancel
			</Button>
			{#if currentStep > 0}
				<Button variant="secondary" disabled={loading} onclick={handleBack}>Back</Button>
			{/if}
			{#if step.render_previous_response}
				<Button variant="primary" {loading} onclick={handleReviewNext}>
					{isLastStep ? 'Done' : 'Execute'}
				</Button>
			{:else if (step.form_ui?.fields?.length ?? 0) > 0}
				<Button variant="primary" type="submit" form={WORKFLOW_FORM_ID} {loading}>
					{isLastStep ? 'Run' : 'Continue'}
				</Button>
			{:else if step.submit_interaction_id}
				<Button variant="primary" {loading} onclick={() => void handleStepSubmit({})}>
					{isLastStep ? 'Run' : 'Continue'}
				</Button>
			{:else}
				<Button variant="primary" {loading} onclick={handleReviewNext}>
					{isLastStep ? 'Done' : 'Continue'}
				</Button>
			{/if}
		{/snippet}
	</Modal>
{/if}

{#if showConfirm && interaction.confirmation}
	<ConfirmDialog
		title={interaction.confirmation.title}
		messagePrefix={interaction.confirmation.message}
		entityName={actionLabel}
		{confirmLabel}
		confirmVariant={confirmVariantForSeverity}
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
