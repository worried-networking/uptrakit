<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { SvelteMap } from 'svelte/reactivity';
	import { clearError, getErrorMessage, getSuccessMessage } from '$lib/notifications.svelte';
	import { Callout, SectionCard, StatusBadge } from '$lib/components/ui';
	import Button from './Button.svelte';
	import type { SystemAlert } from '$lib/types';

	let { alerts, onDismiss }: { alerts: SystemAlert[]; onDismiss: (id: string) => void } = $props();

	const successMessage = $derived(getSuccessMessage());
	const errorMessage = $derived(getErrorMessage());
	const MOBILE_BREAKPOINT = 640;
	const SUCCESS_INFO_TIMEOUT_MS = 4000;
	const WARNING_ERROR_TIMEOUT_MS = 8000;
	const SWIPE_THRESHOLD_PX = 80;

	type ToastTone = 'success' | 'info' | 'warning' | 'danger';
	type ToastKind = 'success' | 'error' | 'system';
	type ToastItem = {
		id: string;
		kind: ToastKind;
		tone: ToastTone;
		title: string;
		badgeLabel: string;
		message: string;
		alertId?: string;
		action?: string;
	};
	type ToastRuntime = {
		item: ToastItem;
		totalMs: number;
		remainingMs: number;
		startedAtMs: number | null;
		timeout: ReturnType<typeof setTimeout> | null;
		paused: boolean;
	};
	type SwipeSession = {
		pointerId: number;
		startX: number;
		startY: number;
	};
	type ProgressState = {
		scale: number;
		durationMs: number;
		paused: boolean;
	};

	let viewportWidth = $state(1024);
	let dismissedToastIds = $state(new Set<string>());
	let progressById = $state<Record<string, ProgressState>>({});
	let swipeOffsetById = $state<Record<string, { x: number; y: number }>>({});

	let successToastVersion = $state(0);
	let errorToastVersion = $state(0);
	let lastSuccessMessage: string | null = $state(null);
	let lastErrorMessage: string | null = $state(null);

	const runtimeById = new SvelteMap<string, ToastRuntime>();
	const swipeById = new SvelteMap<string, SwipeSession>();

	const isMobile = $derived(viewportWidth < MOBILE_BREAKPOINT);

	function systemAlertTone(alert: SystemAlert): 'warning' | 'info' | 'danger' {
		if (alert.severity === 'critical' || alert.severity === 'error') return 'danger';
		if (alert.severity === 'warning') return 'warning';
		return 'info';
	}

	function toneForAlert(alert: SystemAlert): ToastTone {
		return systemAlertTone(alert);
	}

	function progressClassForTone(tone: ToastTone): string {
		switch (tone) {
			case 'success':
				return 'bg-[var(--color-success)]';
			case 'warning':
				return 'bg-[var(--color-warning)]';
			case 'danger':
				return 'bg-[var(--color-error)]';
			case 'info':
				return 'bg-[var(--color-info)]';
		}
	}

	function durationMsForTone(tone: ToastTone): number {
		return tone === 'success' || tone === 'info' ? SUCCESS_INFO_TIMEOUT_MS : WARNING_ERROR_TIMEOUT_MS;
	}

	function badgeToneForToast(tone: ToastTone): 'success' | 'warning' | 'danger' | 'info' {
		switch (tone) {
			case 'success':
				return 'success';
			case 'warning':
				return 'warning';
			case 'danger':
				return 'danger';
			case 'info':
				return 'info';
		}
	}

	function badgeLabelForToast(item: ToastItem): string {
		if (item.kind === 'success') return 'Success';
		if (item.kind === 'error') return 'Error';
		if (item.kind === 'system') return item.badgeLabel;
		return 'Info';
	}

	function setProgressState(id: string, next: ProgressState) {
		progressById = { ...progressById, [id]: next };
	}

	function clearProgressState(id: string) {
		const next = { ...progressById };
		delete next[id];
		progressById = next;
	}

	function setSwipeOffset(id: string, x: number, y: number) {
		swipeOffsetById = { ...swipeOffsetById, [id]: { x, y } };
	}

	function clearSwipeOffset(id: string) {
		const next = { ...swipeOffsetById };
		delete next[id];
		swipeOffsetById = next;
	}

	function dismissToast(item: ToastItem) {
		dismissedToastIds = new Set([...dismissedToastIds, item.id]);
		stopRuntime(item.id);
		clearSwipeOffset(item.id);

		if (item.kind === 'error') {
			clearError();
			return;
		}
		if (item.kind === 'system' && item.alertId) {
			onDismiss(item.alertId);
		}
	}

	function stopRuntime(id: string) {
		const runtime = runtimeById.get(id);
		if (runtime?.timeout) {
			clearTimeout(runtime.timeout);
		}
		runtimeById.delete(id);
		clearProgressState(id);
	}

	function resumeRuntime(id: string) {
		const runtime = runtimeById.get(id);
		if (!runtime || runtime.remainingMs <= 0) {
			if (runtime) dismissToast(runtime.item);
			return;
		}

		if (runtime.timeout) clearTimeout(runtime.timeout);
		runtime.startedAtMs = Date.now();
		runtime.paused = false;
		runtime.timeout = setTimeout(() => dismissToast(runtime.item), runtime.remainingMs);

		setProgressState(id, {
			scale: progressById[id]?.scale ?? 1,
			durationMs: 0,
			paused: false
		});
		queueMicrotask(() => {
			if (!runtimeById.has(id)) return;
			setProgressState(id, {
				scale: 0,
				durationMs: runtime.remainingMs,
				paused: false
			});
		});
	}

	function pauseRuntime(id: string) {
		const runtime = runtimeById.get(id);
		if (!runtime || runtime.paused) return;
		if (runtime.startedAtMs !== null) {
			const elapsed = Date.now() - runtime.startedAtMs;
			runtime.remainingMs = Math.max(0, runtime.remainingMs - elapsed);
		}
		if (runtime.timeout) {
			clearTimeout(runtime.timeout);
			runtime.timeout = null;
		}
		runtime.startedAtMs = null;
		runtime.paused = true;

		const scale = runtime.totalMs > 0 ? runtime.remainingMs / runtime.totalMs : 0;
		setProgressState(id, {
			scale,
			durationMs: 0,
			paused: true
		});
	}

	function ensureRuntime(item: ToastItem) {
		if (runtimeById.has(item.id)) return;
		const totalMs = durationMsForTone(item.tone);
		runtimeById.set(item.id, {
			item,
			totalMs,
			remainingMs: totalMs,
			startedAtMs: null,
			timeout: null,
			paused: false
		});
		setProgressState(item.id, {
			scale: 1,
			durationMs: 0,
			paused: false
		});
		resumeRuntime(item.id);
	}

	function lookupToastById(id: string): ToastItem | undefined {
		return toastItems.find((item) => item.id === id);
	}

	function handlePointerDown(id: string, event: PointerEvent) {
		if (event.button !== 0) return;
		swipeById.set(id, {
			pointerId: event.pointerId,
			startX: event.clientX,
			startY: event.clientY
		});
		pauseRuntime(id);
		(event.currentTarget as HTMLElement | null)?.setPointerCapture(event.pointerId);
	}

	function handlePointerMove(id: string, event: PointerEvent) {
		const swipe = swipeById.get(id);
		if (!swipe || swipe.pointerId !== event.pointerId) return;
		setSwipeOffset(id, event.clientX - swipe.startX, event.clientY - swipe.startY);
	}

	function finishPointer(id: string, event: PointerEvent) {
		const swipe = swipeById.get(id);
		if (!swipe || swipe.pointerId !== event.pointerId) return;

		const offset = swipeOffsetById[id] ?? { x: 0, y: 0 };
		const shouldDismiss = isMobile
			? offset.y > SWIPE_THRESHOLD_PX && Math.abs(offset.y) > Math.abs(offset.x)
			: offset.x > SWIPE_THRESHOLD_PX && Math.abs(offset.x) > Math.abs(offset.y);
		swipeById.delete(id);
		(event.currentTarget as HTMLElement | null)?.releasePointerCapture(event.pointerId);

		const item = lookupToastById(id);
		if (shouldDismiss && item) {
			dismissToast(item);
			return;
		}

		clearSwipeOffset(id);
		resumeRuntime(id);
	}

	$effect(() => {
		if (successMessage && successMessage !== lastSuccessMessage) {
			successToastVersion += 1;
			lastSuccessMessage = successMessage;
		} else if (!successMessage) {
			lastSuccessMessage = null;
		}
	});

	$effect(() => {
		if (errorMessage && errorMessage !== lastErrorMessage) {
			errorToastVersion += 1;
			lastErrorMessage = errorMessage;
		} else if (!errorMessage) {
			lastErrorMessage = null;
		}
	});

	const allToastItems = $derived.by<ToastItem[]>(() => {
		const items: ToastItem[] = [];
		if (successMessage) {
			items.push({
				id: `success:${successToastVersion}`,
				kind: 'success',
				tone: 'success',
				title: 'Success',
				badgeLabel: 'Success',
				message: successMessage
			});
		}
		if (errorMessage) {
			items.push({
				id: `error:${errorToastVersion}`,
				kind: 'error',
				tone: 'danger',
				title: 'Error',
				badgeLabel: 'Error',
				message: errorMessage
			});
		}
		for (const alert of alerts) {
			items.push({
				id: `system:${alert.id}`,
				kind: 'system',
				tone: toneForAlert(alert),
				title: alert.title,
				badgeLabel: alert.severity,
				message: alert.message,
				alertId: alert.id,
				action: alert.action
			});
		}
		return items;
	});

	const toastItems = $derived(allToastItems.filter((item) => !dismissedToastIds.has(item.id)));

	$effect(() => {
		const activeIds = new Set(toastItems.map((item) => item.id));
		for (const item of toastItems) {
			ensureRuntime(item);
		}
		for (const [id] of runtimeById) {
			if (!activeIds.has(id)) stopRuntime(id);
		}
		const sourceIds = new Set(allToastItems.map((item) => item.id));
		const nextDismissed = new Set([...dismissedToastIds].filter((id) => sourceIds.has(id)));
		if (nextDismissed.size !== dismissedToastIds.size) {
			dismissedToastIds = nextDismissed;
		}
	});

	onMount(() => {
		viewportWidth = window.innerWidth;
		const onResize = () => {
			viewportWidth = window.innerWidth;
		};
		window.addEventListener('resize', onResize);
		return () => {
			window.removeEventListener('resize', onResize);
		};
	});

	onDestroy(() => {
		for (const runtime of runtimeById.values()) {
			if (runtime.timeout) clearTimeout(runtime.timeout);
		}
		runtimeById.clear();
		swipeById.clear();
	});
</script>

<div
	class="pointer-events-none fixed bottom-[calc(4.5rem+env(safe-area-inset-bottom))] left-1/2 flex w-[min(20rem,calc(100vw-1.5rem))] max-w-[calc(100vw-1.5rem)] -translate-x-1/2 flex-col gap-1.5 sm:bottom-auto sm:left-auto sm:right-4 sm:top-4 sm:w-[300px] sm:max-w-[calc(100vw-2rem)] sm:translate-x-0"
	aria-label="Notifications"
	data-ui="toast-notifications"
>
	{#each toastItems as item (item.id)}
		{@const progress = progressById[item.id] ?? { scale: 1, durationMs: 0, paused: false }}
		{@const swipeOffset = swipeOffsetById[item.id] ?? { x: 0, y: 0 }}
		<div
			class="pointer-events-auto"
			data-ui="toast-notification"
			role="group"
			aria-label="Toast notification"
			style={`transform: translate(${swipeOffset.x}px, ${swipeOffset.y}px);`}
			onmouseenter={() => pauseRuntime(item.id)}
			onmouseleave={() => resumeRuntime(item.id)}
			onpointerdown={(event) => handlePointerDown(item.id, event)}
			onpointermove={(event) => handlePointerMove(item.id, event)}
			onpointerup={(event) => finishPointer(item.id, event)}
			onpointercancel={(event) => finishPointer(item.id, event)}
		>
			<SectionCard>
				{#snippet actions()}
					<StatusBadge tone={badgeToneForToast(item.tone)} label={badgeLabelForToast(item)} />
					<Button variant="ghost" size="sm" onclick={() => dismissToast(item)}>Dismiss</Button>
				{/snippet}
				<div class="space-y-2">
					<Callout tone={item.tone} title={item.kind === 'system' ? item.title : undefined} message={item.message} />
					{#if item.kind === 'system'}
						<div class="flex flex-wrap gap-2">
							{#if item.action === 'renew_server_certificate'}
								<Button variant="ghost" size="sm" href="/settings/global">Go to Global Settings</Button>
							{/if}
						</div>
					{/if}
					<div class="h-[2px] overflow-hidden rounded-[1px] bg-black/10 dark:bg-white/10" aria-hidden="true">
						<span
							data-ui="toast-progress"
							class={`block h-full origin-left ${progressClassForTone(item.tone)}`}
							style={`transform: scaleX(${progress.scale}); transition: transform ${progress.durationMs}ms linear; ${progress.paused ? 'transition: none;' : ''}`}
						></span>
					</div>
					{#if item.kind === 'system'}
						<div class="sr-only">
							Swipe {isMobile ? 'down' : 'right'} to dismiss.
						</div>
					{/if}
				</div>
			</SectionCard>
		</div>
	{/each}
</div>
