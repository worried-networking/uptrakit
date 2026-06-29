<script lang="ts">
	import { tick } from 'svelte';
	import { page } from '$app/state';
	import { deviceAuthApprove, deviceAuthDeny, deviceAuthLookup } from '$lib/api';
	import { getLoading, getUser } from '$lib/auth.svelte';
	import Button from '$lib/components/Button.svelte';
	import ConsentPrompt from '$lib/components/ConsentPrompt.svelte';
	import { Callout } from '$lib/components/ui';
	import PublicEntryShell from '$lib/components/ui/PublicEntryShell.svelte';

	let success = $state(false);
	let denied = $state(false);
	let processing = $state(false);
	let lookupPhase: 'idle' | 'loading' | 'done' | 'error' = $state('idle');
	let lookupError = $state('');
	let actionError = $state('');

	const ALPHABET = 'BCDFGHJKLMNPQRSTVWXZ';
	const ALPHABET_SET = new Set(ALPHABET);
	const ALPHABET_FILTER = new RegExp(`[^${ALPHABET}]`, 'g');
	const DEVICE_CODE_PATTERN = new RegExp(`^[${ALPHABET}]{4}-[${ALPHABET}]{4}$`);

	const initialParam = page.url.searchParams.get('user_code') ?? '';
	let chars: string[] = $state(
		DEVICE_CODE_PATTERN.test(initialParam) ? initialParam.replace(/-/g, '').split('') : Array(8).fill('')
	);
	let inputEls: (HTMLInputElement | null)[] = $state(Array(8).fill(null));

	let codeValid = $derived(chars.every((c) => c !== ''));
	let enteredCode = $derived(`${chars.slice(0, 4).join('')}-${chars.slice(4).join('')}`);
	let isLoggedIn = $derived(!!getUser());

	function resetLookup() {
		if (lookupPhase !== 'idle') {
			lookupPhase = 'idle';
			lookupError = '';
			// actionError is NOT cleared here — cleared at the start of onApprove/onDeny.
			// Clearing it here would silently hide a failed-approve error when the user edits a box.
		}
	}

	function friendlyLookupError(raw: string): string {
		const lower = raw.toLowerCase();
		if (lower.includes('not found')) {
			return 'Code not found. It may have expired or already been used.';
		}
		if (lower.includes('already authorized')) {
			return 'This device has already been authorized.';
		}
		return raw;
	}

	// lookupPhase is intentionally tracked as a dependency. The synchronous write
	// lookupPhase = 'loading' schedules a re-run, but the re-run exits immediately on the guard.
	// Tracking is required so resetLookup() → 'idle' re-triggers the effect when codeValid
	// doesn't change (user edits one box of a fully-filled code). untrack() would break this.
	$effect(() => {
		if (codeValid && isLoggedIn && lookupPhase === 'idle') {
			lookupPhase = 'loading';
			deviceAuthLookup({ query: { user_code: enteredCode } })
				.then(() => {
					lookupPhase = 'done';
				})
				.catch((err) => {
					lookupError = friendlyLookupError(err instanceof Error ? err.message : 'Lookup failed');
					lookupPhase = 'error';
				});
		}
	});

	function onBoxKeyDown(i: number, e: KeyboardEvent) {
		if (e.isComposing || e.ctrlKey || e.metaKey) return;

		if (e.key === 'Backspace') {
			e.preventDefault();
			resetLookup();
			if (chars[i]) {
				chars[i] = '';
			} else if (i > 0) {
				chars[i - 1] = '';
				inputEls[i - 1]?.focus();
			}
			return;
		}
		if (e.key === 'Delete') {
			e.preventDefault();
			resetLookup();
			chars[i] = '';
			return;
		}
		if (e.key === 'ArrowLeft') {
			e.preventDefault();
			inputEls[Math.max(0, i - 1)]?.focus();
			return;
		}
		if (e.key === 'ArrowRight') {
			e.preventDefault();
			inputEls[Math.min(7, i + 1)]?.focus();
			return;
		}
		if (e.key === 'Home') {
			e.preventDefault();
			inputEls[0]?.focus();
			return;
		}
		if (e.key === 'End') {
			e.preventDefault();
			inputEls[7]?.focus();
			return;
		}
		if (['Tab', 'Enter'].includes(e.key)) return;

		if (!ALPHABET_SET.has(e.key.toUpperCase())) {
			e.preventDefault();
		}
	}

	async function onBoxInput(i: number, e: Event) {
		resetLookup();
		const input = e.target as HTMLInputElement;
		const char = input.value.toUpperCase().replace(ALPHABET_FILTER, '').slice(-1);
		chars[i] = char;
		input.value = char;
		if (char && i < 7) {
			await tick();
			inputEls[i + 1]?.focus();
		}
	}

	async function onPaste(e: ClipboardEvent) {
		e.preventDefault();
		resetLookup();
		const text = e.clipboardData?.getData('text') ?? '';
		const consonants = text.toUpperCase().replace(ALPHABET_FILTER, '').slice(0, 8);
		chars = Array.from({ length: 8 }, (_, i) => consonants[i] ?? '');
		const nextEmpty = chars.indexOf('');
		await tick();
		inputEls[nextEmpty === -1 ? 7 : nextEmpty]?.focus();
	}

	async function onApprove() {
		if (lookupPhase !== 'done') return;
		actionError = '';
		processing = true;
		try {
			await deviceAuthApprove({ body: { user_code: enteredCode } });
			success = true;
		} catch (err) {
			actionError = err instanceof Error ? err.message : 'Failed to authorize device';
		} finally {
			processing = false;
		}
	}

	async function onDeny() {
		if (lookupPhase !== 'done') return;
		actionError = '';
		processing = true;
		try {
			await deviceAuthDeny({ body: { user_code: enteredCode } });
			denied = true;
		} catch (err) {
			actionError = err instanceof Error ? err.message : 'Failed to deny device';
		} finally {
			processing = false;
		}
	}
</script>

<PublicEntryShell
	eyebrow="Device approval"
	title="Authorize Device"
	subtitle="Confirm the code shown in your CLI to finish signing in."
>
	{#if getLoading()}
		<Callout tone="info" message="Loading your session..." />
	{:else if success}
		<Callout tone="success" title="Device approved" message="CLI session approved. You can close this tab." />
	{:else if denied}
		<Callout tone="warning" title="Device denied" message="CLI authorization denied. You can close this tab." />
	{:else}
		<div class="flex items-center justify-center gap-2" role="group" aria-label="Device code">
			{#each [0, 1, 2, 3] as i (i)}
				<input
					bind:this={inputEls[i]}
					type="text"
					maxlength="1"
					value={chars[i]}
					autocomplete="off"
					aria-label="Character {i + 1} of 8"
					onkeydown={(e) => onBoxKeyDown(i, e)}
					oninput={(e) => onBoxInput(i, e)}
					onpaste={onPaste}
					class="h-12 w-10 rounded-card border border-[var(--border-default)] bg-[var(--bg-surface)] text-center font-mono text-xl uppercase text-[var(--text-primary)] caret-transparent focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] focus-visible:border-[var(--accent)] transition-[border-color,box-shadow] duration-fast"
				/>
			{/each}
			<span class="select-none font-mono text-xl text-[var(--text-muted)]" aria-hidden="true">–</span>
			{#each [4, 5, 6, 7] as i (i)}
				<input
					bind:this={inputEls[i]}
					type="text"
					maxlength="1"
					value={chars[i]}
					autocomplete="off"
					aria-label="Character {i + 1} of 8"
					onkeydown={(e) => onBoxKeyDown(i, e)}
					oninput={(e) => onBoxInput(i, e)}
					onpaste={onPaste}
					class="h-12 w-10 rounded-card border border-[var(--border-default)] bg-[var(--bg-surface)] text-center font-mono text-xl uppercase text-[var(--text-primary)] caret-transparent focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] focus-visible:border-[var(--accent)] transition-[border-color,box-shadow] duration-fast"
				/>
			{/each}
		</div>

		{#if !codeValid}
			<!-- user still entering the code — no prompt yet -->
		{:else if !isLoggedIn}
			<Callout tone="info" message="You need to log in before you can authorize this device." />
			<Button
				variant="primary"
				href="/login?redirect=/device?user_code={encodeURIComponent(enteredCode)}"
				class="w-full justify-center"
			>
				Log in
			</Button>
		{:else if lookupPhase === 'loading'}
			<Callout tone="info" message="Verifying code…" />
		{:else if lookupPhase === 'error'}
			<Callout tone="danger" title="Code not recognized" message={lookupError} />
		{:else if lookupPhase === 'done'}
			{#if actionError}
				<Callout tone="danger" title="Unable to process device request" message={actionError} />
			{/if}
			<!-- trust="verified": device clients are always controller-internal (uptrakit CLI).
			     No third-party DCR path exists for device-flow clients. -->
			<ConsentPrompt trust="verified" approving={processing} {onApprove} {onDeny} />
		{/if}
	{/if}
</PublicEntryShell>
